use std::cell::Cell;
use std::collections::{HashSet, VecDeque};

use crate::opensearch::Entry;

pub const TIME_RANGES: &[(&str, u32)] = &[
    ("Last 1 hour",   1),
    ("Last 6 hours",  6),
    ("Last 24 hours", 24),
    ("Last 7 days",   168),
    ("Last 30 days",  720),
];

const MAX_HISTORY: usize = 10;

pub enum AppMessage {
    IndicesLoaded(Vec<String>),
    EntriesLoaded(Vec<Entry>, usize),
    LlmChunk(String),
    LlmDone,
    Error(String),
    /// Update the status bar text (used by background skills).
    Status(String),
    /// A skill saved a file — carry the filename for the status message.
    ReportSaved(String),
}

pub struct App {
    // Index
    pub indices: Vec<String>,
    pub current_index: Option<String>,

    // Entries
    pub entries: Vec<Entry>,
    pub total_entries: usize,
    pub table_cursor: usize,
    pub page: usize,
    pub page_size: usize,

    // Selection
    pub selected_ids: HashSet<String>,

    // Filters
    pub filter_level: u8,
    pub filter_hours: u32,
    pub filter_agent: String,

    // LLM & analysis
    pub llm_provider: String,       // e.g. "openrouter:sonnet4.6" or "ollama"
    pub llm_tags: Vec<String>,      // all available tags for cycling
    pub analysis_text: String,
    pub is_analysing: bool,
    pub analysis_scroll: i32,
    pub analysis_auto_scroll: bool,
    pub analysis_max_scroll: Cell<usize>,

    // Analysis history
    pub analysis_history: VecDeque<String>,
    pub history_view: Option<usize>, // None = current live, Some(i) = history[i]

    // UI state
    pub status: String,
    pub should_quit: bool,
    pub show_index_picker: bool,
    pub index_picker_cursor: usize,
    pub show_help: bool,

    // Detail panel
    pub show_detail: bool,
    pub detail_scroll: usize,
    pub detail_max_scroll: Cell<usize>,

    // Time range picker
    pub show_time_picker: bool,
    pub time_picker_cursor: usize,

    // Agent filter input
    pub show_agent_filter: bool,
    pub agent_filter_input: String,

    // Skill picker
    pub show_skill_picker: bool,
    pub skill_picker_cursor: usize,
    pub is_running_skill: bool,

    // Output directory for exports and reports
    pub output_dir: String,
}

impl App {
    pub fn new(page_size: usize, llm_provider: String, llm_tags: Vec<String>, output_dir: String) -> Self {
        let time_cursor = TIME_RANGES.iter().position(|(_, h)| *h == 24).unwrap_or(2);
        Self {
            indices: vec![],
            current_index: None,
            entries: vec![],
            total_entries: 0,
            table_cursor: 0,
            page: 0,
            page_size,
            selected_ids: HashSet::new(),
            filter_level: 1,
            filter_hours: 24,
            filter_agent: String::new(),
            llm_provider,
            llm_tags,
            analysis_text: String::new(),
            is_analysing: false,
            analysis_scroll: 0,
            analysis_auto_scroll: true,
            analysis_max_scroll: Cell::new(0),
            analysis_history: VecDeque::new(),
            history_view: None,
            status: "Connecting to OpenSearch...".to_string(),
            should_quit: false,
            show_index_picker: false,
            index_picker_cursor: 0,
            show_help: false,
            show_detail: false,
            detail_scroll: 0,
            detail_max_scroll: Cell::new(0),
            show_time_picker: false,
            time_picker_cursor: time_cursor,
            show_agent_filter: false,
            agent_filter_input: String::new(),
            show_skill_picker: false,
            skill_picker_cursor: 0,
            is_running_skill: false,
            output_dir,
        }
    }

    /// Select all visible entries on the current page.
    pub fn select_all_visible(&mut self) {
        for e in &self.entries {
            self.selected_ids.insert(e.id.clone());
        }
        self.update_status();
    }

    pub fn current_entry(&self) -> Option<&Entry> {
        self.entries.get(self.table_cursor)
    }

    /// Pretty-printed JSON of the current entry's _source.
    pub fn detail_json(&self) -> String {
        self.current_entry()
            .and_then(|e| serde_json::to_string_pretty(&e.raw["_source"]).ok())
            .unwrap_or_default()
    }

    pub fn toggle_select_current(&mut self) {
        if let Some(entry) = self.entries.get(self.table_cursor) {
            let id = entry.id.clone();
            if !self.selected_ids.remove(&id) {
                self.selected_ids.insert(id);
            }
            self.update_status();
        }
    }

    pub fn move_up(&mut self) {
        self.table_cursor = self.table_cursor.saturating_sub(1);
    }

    pub fn move_down(&mut self) {
        if self.table_cursor + 1 < self.entries.len() {
            self.table_cursor += 1;
        }
    }

    pub fn next_page(&mut self) {
        let max = self.total_entries.saturating_sub(1) / self.page_size;
        if self.page < max {
            self.page += 1;
            self.table_cursor = 0;
        }
    }

    pub fn prev_page(&mut self) {
        if self.page > 0 {
            self.page -= 1;
            self.table_cursor = 0;
        }
    }

    pub fn selected_entries(&self) -> Vec<&Entry> {
        self.entries.iter().filter(|e| self.selected_ids.contains(&e.id)).collect()
    }

    pub fn clear_selection(&mut self) {
        self.selected_ids.clear();
        self.update_status();
    }

    pub fn toggle_llm(&mut self) {
        if self.llm_tags.is_empty() { return; }
        let current = self.llm_tags.iter().position(|t| t == &self.llm_provider);
        let next = match current {
            Some(i) => (i + 1) % self.llm_tags.len(),
            None => 0,
        };
        self.llm_provider = self.llm_tags[next].clone();
        self.update_status();
    }

    /// Save completed analysis to history ring buffer.
    pub fn save_analysis(&mut self) {
        let text = self.analysis_text.trim().to_owned();
        if text.is_empty() { return; }
        if self.analysis_history.len() >= MAX_HISTORY {
            self.analysis_history.pop_front();
        }
        self.analysis_history.push_back(text);
        self.history_view = None;
    }

    /// Tab: step backward through history (wraps back to current).
    pub fn cycle_history(&mut self) {
        if self.analysis_history.is_empty() { return; }
        self.history_view = match self.history_view {
            None => Some(self.analysis_history.len().saturating_sub(1)),
            Some(0) => None,
            Some(i) => Some(i - 1),
        };
        self.analysis_scroll = 0;
        self.analysis_auto_scroll = false;
    }

    /// Text currently shown in the analysis panel.
    pub fn displayed_analysis(&self) -> &str {
        match self.history_view {
            None => &self.analysis_text,
            Some(i) => self.analysis_history.get(i).map(String::as_str).unwrap_or(""),
        }
    }

    pub fn update_status(&mut self) {
        let pages = self.total_entries.div_ceil(self.page_size).max(1);
        let index = self.current_index.as_deref().unwrap_or("none");
        let agent_part = if self.filter_agent.is_empty() {
            String::new()
        } else {
            format!(" | Agent: {}", self.filter_agent)
        };
        self.status = format!(
            " {} entries | Page {}/{} | Selected: {} | Lvl≥{} | Last {}h{} | {} | LLM: {}",
            self.total_entries,
            self.page + 1,
            pages,
            self.selected_ids.len(),
            self.filter_level,
            self.filter_hours,
            agent_part,
            index,
            self.llm_provider,
        );
    }
}
