use std::sync::Arc;

use crossterm::event::{self, KeyCode, KeyModifiers};
use tokio::sync::mpsc;

use crate::app::{App, AppMessage, TIME_RANGES};
use crate::skills;
use crate::tasks;

pub fn handle_key(
    app: &mut App,
    key: event::KeyEvent,
    tx: &mpsc::Sender<AppMessage>,
    os_client: &Arc<crate::opensearch::OpenSearchClient>,
    llm_client: &Arc<crate::llm::LlmClient>,
) {
    // Modal overlays — highest priority
    if app.show_help          { app.show_help = false; return; }
    if app.show_index_picker  { return handle_index_picker(app, key, tx, os_client); }
    if app.show_detail        { return handle_detail(app, key); }
    if app.show_time_picker   { return handle_time_picker(app, key, tx, os_client); }
    if app.show_agent_filter  { return handle_agent_filter(app, key, tx, os_client); }
    if app.show_skill_picker  { return handle_skill_picker(app, key, tx, os_client, llm_client); }

    // Main view
    handle_main(app, key, tx, os_client, llm_client);
}

// ─── Index Picker ────────────────────────────────────────────────────────────

fn handle_index_picker(
    app: &mut App,
    key: event::KeyEvent,
    tx: &mpsc::Sender<AppMessage>,
    os_client: &Arc<crate::opensearch::OpenSearchClient>,
) {
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
                tasks::spawn_load_entries(app, tx.clone(), os_client.clone());
            }
        }
        _ => {}
    }
}

// ─── Detail Panel ────────────────────────────────────────────────────────────

fn handle_detail(app: &mut App, key: event::KeyEvent) {
    match key.code {
        KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => app.show_detail = false,
        KeyCode::Up | KeyCode::Char('k') => {
            app.detail_scroll = app.detail_scroll.saturating_sub(1);
        }
        KeyCode::Down | KeyCode::Char('j') => {
            let max = app.detail_max_scroll.get();
            if app.detail_scroll < max {
                app.detail_scroll += 1;
            }
        }
        KeyCode::PageUp => {
            app.detail_scroll = app.detail_scroll.saturating_sub(10);
        }
        KeyCode::PageDown => {
            let max = app.detail_max_scroll.get();
            app.detail_scroll = (app.detail_scroll + 10).min(max);
        }
        _ => {}
    }
}

// ─── Time Range Picker ───────────────────────────────────────────────────────

fn handle_time_picker(
    app: &mut App,
    key: event::KeyEvent,
    tx: &mpsc::Sender<AppMessage>,
    os_client: &Arc<crate::opensearch::OpenSearchClient>,
) {
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => app.show_time_picker = false,
        KeyCode::Up | KeyCode::Char('k') => {
            app.time_picker_cursor = app.time_picker_cursor.saturating_sub(1);
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if app.time_picker_cursor + 1 < TIME_RANGES.len() {
                app.time_picker_cursor += 1;
            }
        }
        KeyCode::Enter => {
            app.filter_hours = TIME_RANGES[app.time_picker_cursor].1;
            app.show_time_picker = false;
            app.page = 0;
            tasks::spawn_load_entries(app, tx.clone(), os_client.clone());
        }
        _ => {}
    }
}

// ─── Agent Filter ────────────────────────────────────────────────────────────

fn handle_agent_filter(
    app: &mut App,
    key: event::KeyEvent,
    tx: &mpsc::Sender<AppMessage>,
    os_client: &Arc<crate::opensearch::OpenSearchClient>,
) {
    match key.code {
        KeyCode::Esc => app.show_agent_filter = false,
        KeyCode::Enter => {
            app.filter_agent = app.agent_filter_input.trim().to_owned();
            app.show_agent_filter = false;
            app.page = 0;
            tasks::spawn_load_entries(app, tx.clone(), os_client.clone());
        }
        KeyCode::Backspace => { app.agent_filter_input.pop(); }
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.agent_filter_input.clear();
        }
        KeyCode::Char(c) => app.agent_filter_input.push(c),
        _ => {}
    }
}

// ─── Skill Picker ────────────────────────────────────────────────────────────

fn handle_skill_picker(
    app: &mut App,
    key: event::KeyEvent,
    tx: &mpsc::Sender<AppMessage>,
    os_client: &Arc<crate::opensearch::OpenSearchClient>,
    llm_client: &Arc<crate::llm::LlmClient>,
) {
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => app.show_skill_picker = false,
        KeyCode::Up | KeyCode::Char('k') => {
            app.skill_picker_cursor = app.skill_picker_cursor.saturating_sub(1);
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if app.skill_picker_cursor + 1 < skills::SKILLS.len() {
                app.skill_picker_cursor += 1;
            }
        }
        KeyCode::Enter => {
            if let Some(skill) = skills::SKILLS.get(app.skill_picker_cursor) {
                app.show_skill_picker = false;
                match skill.id {
                    "daily_report" => {
                        tasks::spawn_run_report(app, tx.clone(), os_client.clone(), llm_client.clone(), app.output_dir.clone());
                    }
                    _ => {
                        app.status = format!(" Skill '{}' not implemented yet.", skill.id);
                    }
                }
            }
        }
        _ => {}
    }
}

// ─── Main View ───────────────────────────────────────────────────────────────

fn handle_main(
    app: &mut App,
    key: event::KeyEvent,
    tx: &mpsc::Sender<AppMessage>,
    os_client: &Arc<crate::opensearch::OpenSearchClient>,
    llm_client: &Arc<crate::llm::LlmClient>,
) {
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

        // Detail panel
        KeyCode::Enter => {
            if app.current_entry().is_some() {
                app.show_detail = true;
                app.detail_scroll = 0;
            }
        }

        // Select all visible
        KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.select_all_visible();
        }

        // Analyse selected
        KeyCode::Char('a') | KeyCode::Char('A') => {
            app.history_view = None;
            tasks::spawn_analyse(app, tx.clone(), llm_client.clone());
        }

        // Skill picker
        KeyCode::Char('s') | KeyCode::Char('S') => {
            app.show_skill_picker = true;
        }

        // Reload
        KeyCode::Char('r') | KeyCode::Char('R') => {
            tasks::spawn_load_entries(app, tx.clone(), os_client.clone());
        }

        // Pagination
        KeyCode::Char('n') | KeyCode::Right => {
            app.next_page();
            tasks::spawn_load_entries(app, tx.clone(), os_client.clone());
        }
        KeyCode::Char('p') | KeyCode::Left => {
            app.prev_page();
            tasks::spawn_load_entries(app, tx.clone(), os_client.clone());
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

        // Time range picker
        KeyCode::Char('t') | KeyCode::Char('T') => {
            app.time_picker_cursor = TIME_RANGES
                .iter()
                .position(|(_, h)| *h == app.filter_hours)
                .unwrap_or(2);
            app.show_time_picker = true;
        }

        // Agent filter
        KeyCode::Char('f') | KeyCode::Char('F') => {
            app.agent_filter_input = app.filter_agent.clone();
            app.show_agent_filter = true;
        }

        // Export
        KeyCode::Char('e') | KeyCode::Char('E') => {
            let dir = app.output_dir.clone();
            tasks::export(app, &dir);
        }

        // Scroll analysis panel
        KeyCode::Char('[') => {
            app.analysis_auto_scroll = false;
            let min = -(app.analysis_max_scroll.get() as i32);
            app.analysis_scroll = (app.analysis_scroll - 1).max(min);
        }
        KeyCode::Char(']') => {
            app.analysis_auto_scroll = false;
            app.analysis_scroll = (app.analysis_scroll + 1).min(0);
        }

        // Analysis history
        KeyCode::Tab => app.cycle_history(),

        // Help
        KeyCode::Char('h') | KeyCode::Char('H') | KeyCode::Char('?') => app.show_help = true,

        // Toggle LLM model
        KeyCode::Char('l') | KeyCode::Char('L') => app.toggle_llm(),

        // Level filter
        KeyCode::Char('+') | KeyCode::Char('=') => {
            app.filter_level = app.filter_level.saturating_add(1).min(15);
            tasks::spawn_load_entries(app, tx.clone(), os_client.clone());
        }
        KeyCode::Char('-') => {
            app.filter_level = app.filter_level.saturating_sub(1).max(1);
            tasks::spawn_load_entries(app, tx.clone(), os_client.clone());
        }

        _ => {}
    }
}
