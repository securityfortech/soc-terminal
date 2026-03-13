use std::io;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use tokio::sync::mpsc;

mod app;
mod config;
mod llm;
mod opensearch;
mod ui;

use app::{App, AppMessage};
use opensearch::Filters;

#[tokio::main]
async fn main() -> Result<()> {
    let config_path =
        std::env::var("SIEM_CONFIG").unwrap_or_else(|_| "config.yaml".to_string());
    let file = std::fs::File::open(&config_path)
        .map_err(|e| anyhow::anyhow!("Cannot open {config_path}: {e}"))?;
    let config: config::Config = serde_yaml::from_reader(file)?;

    let os_client = Arc::new(opensearch::OpenSearchClient::new(config.opensearch.clone())?);
    let llm_client = Arc::new(llm::LlmClient::new(config.llm.clone()));

    let mut app = App::new(config.ui.page_size, config.llm.provider.clone());
    let (tx, mut rx) = mpsc::channel::<AppMessage>(256);

    // Load indices on startup
    spawn_load_indices(tx.clone(), os_client.clone());

    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = event_loop(&mut terminal, &mut app, &mut rx, &tx, &os_client, &llm_client).await;

    // Always restore terminal
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

        // Drain all pending background messages
        while let Ok(msg) = rx.try_recv() {
            handle_message(app, msg, tx, os_client);
        }

        // Handle keyboard (non-blocking, 16 ms tick)
        if event::poll(Duration::from_millis(16))? {
            if let Event::Key(key) = event::read()? {
                handle_key(app, key, tx, os_client, llm_client);
            }
        }

        if app.should_quit {
            break;
        }
    }
    Ok(())
}

// ─── Message handler ─────────────────────────────────────────────────────────

fn handle_message(
    app: &mut App,
    msg: AppMessage,
    tx: &mpsc::Sender<AppMessage>,
    os_client: &Arc<opensearch::OpenSearchClient>,
) {
    match msg {
        AppMessage::IndicesLoaded(indices) => {
            app.indices = indices.clone();
            // Default to most recent wazuh-alerts, else first index
            let default = indices
                .iter()
                .find(|i| i.contains("wazuh-alerts"))
                .or_else(|| indices.first())
                .cloned();
            if let Some(idx) = default {
                app.current_index = Some(idx);
                spawn_load_entries(app, tx.clone(), os_client.clone());
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
            app.is_analysing = false;
            app.update_status();
        }
        AppMessage::Error(e) => {
            app.status = format!(" Error: {e}");
            if app.is_analysing {
                app.analysis_text.push_str(&format!("\n\n⚠  {e}"));
            }
            app.is_analysing = false;
        }
    }
}

// ─── Key handler ─────────────────────────────────────────────────────────────

fn handle_key(
    app: &mut App,
    key: event::KeyEvent,
    tx: &mpsc::Sender<AppMessage>,
    os_client: &Arc<opensearch::OpenSearchClient>,
    llm_client: &Arc<llm::LlmClient>,
) {
    // Help overlay captures all keys while open
    if app.show_help {
        app.show_help = false;
        return;
    }

    // Index picker captures all keys while open
    if app.show_index_picker {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => app.show_index_picker = false,
            KeyCode::Up | KeyCode::Char('k') => {
                app.index_picker_cursor = app.index_picker_cursor.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if app.index_picker_cursor + 1 < app.indices.len() {
                    app.index_picker_cursor += 1;
                }
            }
            KeyCode::Enter => {
                if let Some(idx) = app.indices.get(app.index_picker_cursor).cloned() {
                    app.current_index = Some(idx);
                    app.page = 0;
                    app.selected_ids.clear();
                    app.show_index_picker = false;
                    spawn_load_entries(app, tx.clone(), os_client.clone());
                }
            }
            _ => {}
        }
        return;
    }

    match key.code {
        // Quit
        KeyCode::Char('q') | KeyCode::Char('Q') => app.should_quit = true,
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.should_quit = true
        }

        // Navigation
        KeyCode::Up | KeyCode::Char('k') => app.move_up(),
        KeyCode::Down | KeyCode::Char('j') => app.move_down(),

        // Selection
        KeyCode::Char(' ') => app.toggle_select_current(),

        // Analyse selected
        KeyCode::Char('a') | KeyCode::Char('A') => {
            spawn_analyse(app, tx.clone(), llm_client.clone());
        }

        // Reload
        KeyCode::Char('r') | KeyCode::Char('R') => {
            spawn_load_entries(app, tx.clone(), os_client.clone());
        }

        // Pagination
        KeyCode::Char('n') | KeyCode::Right => {
            app.next_page();
            spawn_load_entries(app, tx.clone(), os_client.clone());
        }
        KeyCode::Char('p') | KeyCode::Left => {
            app.prev_page();
            spawn_load_entries(app, tx.clone(), os_client.clone());
        }

        // Clear selection
        KeyCode::Char('c') => app.clear_selection(),

        // Index picker
        KeyCode::Char('i') | KeyCode::Char('I') => {
            if let Some(cur) = &app.current_index {
                app.index_picker_cursor =
                    app.indices.iter().position(|i| i == cur).unwrap_or(0);
            }
            app.show_index_picker = true;
        }

        // Scroll analysis panel  (scroll is offset from bottom: 0=bottom, -N=N lines up)
        KeyCode::Char('[') => {
            app.analysis_auto_scroll = false;
            let min = -(app.analysis_max_scroll.get() as i32);
            app.analysis_scroll = (app.analysis_scroll - 1).max(min); // clamp at top
        }
        KeyCode::Char(']') => {
            app.analysis_auto_scroll = false;
            app.analysis_scroll = (app.analysis_scroll + 1).min(0); // clamp at bottom
        }

        // Help
        KeyCode::Char('h') | KeyCode::Char('H') | KeyCode::Char('?') => app.show_help = true,

        // Toggle LLM provider
        KeyCode::Char('l') | KeyCode::Char('L') => app.toggle_llm(),

        // Level filter
        KeyCode::Char('+') | KeyCode::Char('=') => {
            app.filter_level = app.filter_level.saturating_add(1).min(15);
            spawn_load_entries(app, tx.clone(), os_client.clone());
        }
        KeyCode::Char('-') => {
            app.filter_level = app.filter_level.saturating_sub(1).max(1);
            spawn_load_entries(app, tx.clone(), os_client.clone());
        }

        _ => {}
    }
}

// ─── Background task spawners ────────────────────────────────────────────────

fn spawn_load_indices(tx: mpsc::Sender<AppMessage>, client: Arc<opensearch::OpenSearchClient>) {
    tokio::spawn(async move {
        match client.list_indices().await {
            Ok(v) => { let _ = tx.send(AppMessage::IndicesLoaded(v)).await; }
            Err(e) => { let _ = tx.send(AppMessage::Error(e.to_string())).await; }
        }
    });
}

fn spawn_load_entries(
    app: &mut App,
    tx: mpsc::Sender<AppMessage>,
    client: Arc<opensearch::OpenSearchClient>,
) {
    let Some(index) = app.current_index.clone() else { return };

    let filters = Filters {
        min_level: app.filter_level,
        agent: if app.filter_agent.is_empty() { None } else { Some(app.filter_agent.clone()) },
        hours: Some(app.filter_hours),
    };
    let from = app.page * app.page_size;
    let size = app.page_size;

    app.status = " Loading...".to_string();

    tokio::spawn(async move {
        match client.get_entries(&index, &filters, size, from).await {
            Ok((e, t)) => { let _ = tx.send(AppMessage::EntriesLoaded(e, t)).await; }
            Err(e) => { let _ = tx.send(AppMessage::Error(e.to_string())).await; }
        }
    });
}

fn spawn_analyse(
    app: &mut App,
    tx: mpsc::Sender<AppMessage>,
    llm_client: Arc<llm::LlmClient>,
) {
    let selected: Vec<_> = app.selected_entries().into_iter().cloned().collect();
    if selected.is_empty() {
        app.status = " No entries selected — press Space to select entries first.".to_string();
        return;
    }

    // Clone config with current provider override
    let mut cfg = llm_client.config.clone();
    cfg.provider = app.llm_provider.clone();
    let client = llm::LlmClient::new(cfg);

    app.analysis_text =
        format!("Analysing {} entries with {}…\n\n", selected.len(), app.llm_provider);
    app.is_analysing = true;
    app.analysis_scroll = 0;        // 0 = bottom
    app.analysis_auto_scroll = true;

    let (chunk_tx, mut chunk_rx) = mpsc::channel::<String>(128);
    let app_tx = tx.clone();

    // Forward chunks → AppMessage
    tokio::spawn(async move {
        while let Some(chunk) = chunk_rx.recv().await {
            let _ = app_tx.send(AppMessage::LlmChunk(chunk)).await;
        }
        let _ = app_tx.send(AppMessage::LlmDone).await;
    });

    // Run LLM
    tokio::spawn(async move {
        if let Err(e) = client.analyse(&selected, chunk_tx).await {
            let _ = tx.send(AppMessage::Error(e.to_string())).await;
        }
        // chunk_tx dropped → forwarding task exits → LlmDone sent
    });
}
