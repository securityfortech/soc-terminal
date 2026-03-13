use std::collections::HashSet;

use crate::opensearch::Entry;

pub enum AppMessage {
    IndicesLoaded(Vec<String>),
    EntriesLoaded(Vec<Entry>, usize),
    LlmChunk(String),
    LlmDone,
    Error(String),
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

    // LLM
    pub llm_provider: String,
    pub analysis_text: String,
    pub is_analysing: bool,
    pub analysis_scroll: i32,                   // offset from bottom (0=bottom, -N=N lines up)
    pub analysis_auto_scroll: bool,             // follows bottom while streaming
    pub analysis_max_scroll: std::cell::Cell<usize>, // cached by render, used to clamp scroll

    // UI state
    pub status: String,
    pub should_quit: bool,
    pub show_index_picker: bool,
    pub index_picker_cursor: usize,
    pub show_help: bool,
}

impl App {
    pub fn new(page_size: usize, llm_provider: String) -> Self {
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
            analysis_text: String::new(),
            is_analysing: false,
            analysis_scroll: 0,
            analysis_auto_scroll: true,
            analysis_max_scroll: std::cell::Cell::new(0),
            status: "Connecting to OpenSearch...".to_string(),
            should_quit: false,
            show_index_picker: false,
            index_picker_cursor: 0,
            show_help: false,
        }
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
        self.llm_provider = match self.llm_provider.as_str() {
            "claude" => "ollama".to_string(),
            _ => "claude".to_string(),
        };
        self.update_status();
    }

    pub fn update_status(&mut self) {
        let pages = if self.total_entries == 0 {
            1
        } else {
            (self.total_entries + self.page_size - 1) / self.page_size
        };
        let index = self.current_index.as_deref().unwrap_or("none");
        self.status = format!(
            " {} entries | Page {}/{} | Selected: {} | LvL≥{} | Last {}h | {} | LLM: {}",
            self.total_entries,
            self.page + 1,
            pages,
            self.selected_ids.len(),
            self.filter_level,
            self.filter_hours,
            index,
            self.llm_provider,
        );
    }
}
