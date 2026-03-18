use std::io;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use crossterm::{
    event::{self, Event},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use tokio::sync::mpsc;

mod app;
mod config;
mod input;
mod llm;
mod opensearch;
mod report;
mod skills;
mod tasks;
mod ui;

use app::{App, AppMessage};

#[tokio::main]
async fn main() -> Result<()> {
    let config_path =
        std::env::var("SIEM_CONFIG").unwrap_or_else(|_| "config.yaml".to_string());
    let file = std::fs::File::open(&config_path)
        .map_err(|e| anyhow::anyhow!("Cannot open {config_path}: {e}"))?;
    let config: config::Config = serde_yaml::from_reader(file)?;

    let os_client = Arc::new(opensearch::OpenSearchClient::new(config.opensearch.clone())?);
    let llm_client = Arc::new(llm::LlmClient::new(config.llm.clone()));

    // Build LLM tag list: openrouter:<tag> for each cloud model, local:<tag> for Ollama
    let mut llm_tags: Vec<String> = config
        .llm
        .openrouter
        .models
        .iter()
        .map(|m| format!("openrouter:{}", m.tag))
        .collect();
    for m in &config.llm.ollama.models {
        llm_tags.push(format!("local:{}", m.tag));
    }

    let default_provider = if config.llm.provider == "openrouter" {
        llm_tags.iter().find(|t| t.starts_with("openrouter:"))
    } else {
        llm_tags.iter().find(|t| t.starts_with("local:"))
    }
    .cloned()
    .unwrap_or_else(|| llm_tags.first().cloned().unwrap_or_else(|| "local:default".to_string()));

    let mut app = App::new(config.ui.page_size, default_provider, llm_tags, config.ui.output_dir.clone());
    let (tx, mut rx) = mpsc::channel::<AppMessage>(256);

    tasks::spawn_load_indices(tx.clone(), os_client.clone());

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = event_loop(&mut terminal, &mut app, &mut rx, &tx, &os_client, &llm_client).await;

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

async fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    rx: &mut mpsc::Receiver<AppMessage>,
    tx: &mpsc::Sender<AppMessage>,
    os_client: &Arc<opensearch::OpenSearchClient>,
    llm_client: &Arc<llm::LlmClient>,
) -> Result<()> {
    loop {
        terminal.draw(|f| ui::render(f, app))?;

        while let Ok(msg) = rx.try_recv() {
            handle_message(app, msg, tx, os_client);
        }

        if event::poll(Duration::from_millis(16))? {
            if let Event::Key(key) = event::read()? {
                input::handle_key(app, key, tx, os_client, llm_client);
            }
        }

        if app.should_quit {
            break;
        }
    }
    Ok(())
}

fn handle_message(
    app: &mut App,
    msg: AppMessage,
    tx: &mpsc::Sender<AppMessage>,
    os_client: &Arc<opensearch::OpenSearchClient>,
) {
    match msg {
        AppMessage::IndicesLoaded(indices) => {
            app.indices = indices.clone();
            let default = indices
                .iter()
                .find(|i| i.contains("wazuh-alerts"))
                .or_else(|| indices.first())
                .cloned();
            if let Some(idx) = default {
                app.current_index = Some(idx);
                tasks::spawn_load_entries(app, tx.clone(), os_client.clone());
            } else {
                app.status = "No indices found.".to_string();
            }
        }
        AppMessage::EntriesLoaded(entries, total) => {
            app.entries = entries;
            app.total_entries = total;
            app.table_cursor = 0;
            app.update_status();
        }
        AppMessage::LlmChunk(text) => {
            app.analysis_text.push_str(&text);
        }
        AppMessage::LlmDone => {
            app.save_analysis();
            app.is_analysing = false;
            app.update_status();
        }
        AppMessage::Error(e) => {
            app.status = format!(" Error: {e}");
            if app.is_analysing {
                app.analysis_text.push_str(&format!("\n\n⚠  {e}"));
            }
            app.is_analysing = false;
            app.is_running_skill = false;
        }
        AppMessage::Status(s) => {
            app.status = s;
        }
        AppMessage::ReportSaved(filename) => {
            app.status = format!(" Report saved → {filename}");
            app.is_running_skill = false;
        }
    }
}
