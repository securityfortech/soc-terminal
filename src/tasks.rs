use std::sync::Arc;

use tokio::sync::mpsc;

use crate::app::{App, AppMessage};
use crate::opensearch::{self, Filters};
use crate::{llm, report};

// ─── Export ──────────────────────────────────────────────────────────────────

pub fn export(app: &mut App, output_dir: &str) {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let entries: Vec<serde_json::Value> = app
        .entries
        .iter()
        .filter(|e| app.selected_ids.contains(&e.id))
        .map(|e| e.raw.clone())
        .collect();

    let analysis = app.displayed_analysis().to_owned();
    let count = entries.len();

    let data = serde_json::json!({
        "exported_at_unix": ts,
        "analysis": analysis,
        "entry_count": count,
        "entries": entries,
    });

    let dir = std::path::Path::new(output_dir);
    let _ = std::fs::create_dir_all(dir);
    let path = dir.join(format!("soc-export-{ts}.json"));

    match serde_json::to_string_pretty(&data) {
        Ok(content) => match std::fs::write(&path, content) {
            Ok(_) => app.status = format!(" Exported {count} entries → {}", path.display()),
            Err(e) => app.status = format!(" Export failed: {e}"),
        },
        Err(e) => app.status = format!(" Export failed: {e}"),
    }
}

// ─── Background task spawners ────────────────────────────────────────────────

pub fn spawn_load_indices(
    tx: mpsc::Sender<AppMessage>,
    client: Arc<opensearch::OpenSearchClient>,
) {
    tokio::spawn(async move {
        match client.list_indices().await {
            Ok(v) => { let _ = tx.send(AppMessage::IndicesLoaded(v)).await; }
            Err(e) => { let _ = tx.send(AppMessage::Error(e.to_string())).await; }
        }
    });
}

pub fn spawn_load_entries(
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

pub fn spawn_run_report(
    app: &mut App,
    tx: mpsc::Sender<AppMessage>,
    os_client: Arc<opensearch::OpenSearchClient>,
    llm_client: Arc<llm::LlmClient>,
    output_dir: String,
) {
    let Some(index) = app.current_index.clone() else {
        app.status = " No index selected — cannot generate report.".to_string();
        return;
    };

    if app.is_running_skill {
        app.status = " A skill is already running.".to_string();
        return;
    }

    let hours = app.filter_hours;
    let mut cfg = llm_client.config.clone();
    cfg.provider = app.llm_provider.clone();
    let client = llm::LlmClient::new(cfg);

    app.analysis_text = "Generating Daily Activity Report…\n\n".to_string();
    app.is_analysing = true;
    app.is_running_skill = true;
    app.analysis_scroll = 0;
    app.analysis_auto_scroll = true;
    app.history_view = None;

    tokio::spawn(async move {
        let _ = tx.send(AppMessage::Status(" Fetching SOC statistics…".to_string())).await;

        let stats = match os_client.get_daily_stats(&index, hours).await {
            Ok(s) => s,
            Err(e) => {
                let _ = tx.send(AppMessage::Error(e.to_string())).await;
                return;
            }
        };

        let header = format!(
            "# SOC Daily Activity Report\n\
             **Period:** Last {hours}h  |  **Index:** {index}  |  **Total Alerts:** {total}\n\n\
             ## Executive Summary\n\n",
            total = stats.total
        );
        let _ = tx.send(AppMessage::LlmChunk(header)).await;

        let _ = tx.send(AppMessage::Status(" Generating LLM executive summary…".to_string())).await;

        let (chunk_tx, mut chunk_rx) = mpsc::channel::<String>(128);
        let stats_for_llm = stats.clone();
        let err_tx = tx.clone();

        tokio::spawn(async move {
            if let Err(e) = client.generate_report_summary(&stats_for_llm, chunk_tx).await {
                let _ = err_tx.send(AppMessage::Error(e.to_string())).await;
            }
        });

        let mut llm_text = String::new();
        while let Some(chunk) = chunk_rx.recv().await {
            llm_text.push_str(&chunk);
            let _ = tx.send(AppMessage::LlmChunk(chunk)).await;
        }

        match report::build_and_save(&stats, &llm_text, &output_dir) {
            Ok(filename) => {
                let _ = tx.send(AppMessage::ReportSaved(filename)).await;
            }
            Err(e) => {
                let _ = tx.send(AppMessage::Error(format!("Report save failed: {e}"))).await;
            }
        }

        let _ = tx.send(AppMessage::LlmDone).await;
    });
}

pub fn spawn_analyse(
    app: &mut App,
    tx: mpsc::Sender<AppMessage>,
    llm_client: Arc<llm::LlmClient>,
) {
    let selected: Vec<_> = app.selected_entries().into_iter().cloned().collect();
    if selected.is_empty() {
        app.status = " No entries selected — press Space to select entries first.".to_string();
        return;
    }

    let mut cfg = llm_client.config.clone();
    cfg.provider = app.llm_provider.clone();
    let client = llm::LlmClient::new(cfg);

    app.analysis_text =
        format!("Analysing {} entries with {}…\n\n", selected.len(), app.llm_provider);
    app.is_analysing = true;
    app.analysis_scroll = 0;
    app.analysis_auto_scroll = true;

    let (chunk_tx, mut chunk_rx) = mpsc::channel::<String>(128);
    let app_tx = tx.clone();

    tokio::spawn(async move {
        while let Some(chunk) = chunk_rx.recv().await {
            let _ = app_tx.send(AppMessage::LlmChunk(chunk)).await;
        }
        let _ = app_tx.send(AppMessage::LlmDone).await;
    });

    tokio::spawn(async move {
        if let Err(e) = client.analyse(&selected, chunk_tx).await {
            let _ = tx.send(AppMessage::Error(e.to_string())).await;
        }
    });
}
