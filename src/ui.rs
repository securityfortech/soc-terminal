use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{
        Block, Borders, Cell, Clear, List, ListItem, ListState, Paragraph, Row, Table, TableState,
    },
    Frame,
};

use crate::app::App;

pub fn render(frame: &mut Frame, app: &App) {
    let area = frame.size();

    let chunks = Layout::vertical([
        Constraint::Length(1),  // title bar
        Constraint::Length(1),  // controls / current state
        Constraint::Min(8),     // entry table
        Constraint::Length(1),  // status bar
        Constraint::Length(12), // analysis panel
        Constraint::Length(1),  // footer keybindings
    ])
    .split(area);

    render_title(frame, chunks[0]);
    render_controls(frame, app, chunks[1]);
    render_table(frame, app, chunks[2]);
    render_status(frame, app, chunks[3]);
    render_analysis(frame, app, chunks[4]);
    render_footer(frame, chunks[5]);

    if app.show_index_picker {
        render_index_picker(frame, app, area);
    }

    if app.show_help {
        render_help(frame, area);
    }
}

// ─── Title ───────────────────────────────────────────────────────────────────

fn render_title(frame: &mut Frame, area: Rect) {
    let line = Line::from(vec![
        Span::styled(
            " ◈ SOC Terminal ",
            Style::default().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "  Select entries with Space, then press A to analyse",
            Style::default().fg(Color::DarkGray),
        ),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

// ─── Controls ────────────────────────────────────────────────────────────────

fn render_controls(frame: &mut Frame, app: &App, area: Rect) {
    let index = app.current_index.as_deref().unwrap_or("—");
    let llm_color = match app.llm_provider.as_str() {
        "claude" => Color::Magenta,
        _ => Color::Green,
    };

    let line = Line::from(vec![
        Span::raw(" "),
        kb("I"), Span::raw(" Index: "),
        Span::styled(index, Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
        Span::raw("   "),
        kb("+/-"), Span::raw(" Level ≥ "),
        Span::styled(app.filter_level.to_string(), Style::default().fg(Color::Cyan)),
        Span::raw("   "),
        Span::raw("Last "),
        Span::styled(format!("{}h", app.filter_hours), Style::default().fg(Color::Cyan)),
        Span::raw("   "),
        kb("L"), Span::raw(" LLM: "),
        Span::styled(&app.llm_provider, Style::default().fg(llm_color).add_modifier(Modifier::BOLD)),
    ]);

    frame.render_widget(
        Paragraph::new(line).style(Style::default().bg(Color::Reset)),
        area,
    );
}

// ─── Entry Table ─────────────────────────────────────────────────────────────

fn render_table(frame: &mut Frame, app: &App, area: Rect) {
    let header = Row::new(vec!["", "Timestamp", "Lvl", "Agent", "Rule", "Description"])
        .style(
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        )
        .height(1);

    let rows: Vec<Row> = app
        .entries
        .iter()
        .map(|e| {
            let selected = app.selected_ids.contains(&e.id);
            let mark_style = if selected {
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            };

            Row::new(vec![
                Cell::from(if selected { "■" } else { "□" }).style(mark_style),
                Cell::from(e.timestamp.as_str()).style(Style::default().fg(Color::DarkGray)),
                Cell::from(e.level.to_string()).style(level_style(e.level)),
                Cell::from(truncate(&e.agent, 22)).style(Style::default().fg(Color::Cyan)),
                Cell::from(truncate(&e.rule_id, 8)).style(Style::default().fg(Color::DarkGray)),
                Cell::from(e.description.as_str()),
            ])
        })
        .collect();

    let widths = [
        Constraint::Length(3),
        Constraint::Length(20),
        Constraint::Length(4),
        Constraint::Length(23),
        Constraint::Length(8),
        Constraint::Fill(1),
    ];

    let table = Table::new(rows, widths)
        .header(header)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" Entries ({} loaded / {} total) ", app.entries.len(), app.total_entries))
                .border_style(Style::default().fg(Color::DarkGray)),
        )
        .highlight_style(
            Style::default().bg(Color::DarkGray).add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ");

    let mut state = TableState::default();
    state.select(if app.entries.is_empty() { None } else { Some(app.table_cursor) });
    frame.render_stateful_widget(table, area, &mut state);
}

// ─── Status Bar ──────────────────────────────────────────────────────────────

fn render_status(frame: &mut Frame, app: &App, area: Rect) {
    let style = if app.is_analysing {
        Style::default().fg(Color::Yellow)
    } else if app.status.contains("Error") {
        Style::default().fg(Color::Red)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    frame.render_widget(Paragraph::new(app.status.as_str()).style(style), area);
}

// ─── Analysis Panel ──────────────────────────────────────────────────────────

fn render_analysis(frame: &mut Frame, app: &App, area: Rect) {
    let border_color = if app.is_analysing { Color::Yellow } else { Color::Blue };
    let title = if app.is_analysing {
        " Analysis  ⟳ streaming... "
    } else if app.analysis_text.is_empty() {
        " Analysis  (select entries, press A) "
    } else {
        " Analysis "
    };

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));

    // Get the exact inner area (inside the borders) for content rendering
    let inner = block.inner(area);
    let inner_h = inner.height as usize;
    let inner_w = inner.width as usize;

    // Render border block first
    frame.render_widget(block, area);

    if inner_h == 0 || inner_w == 0 || app.analysis_text.is_empty() {
        return;
    }

    // Word-wrap and compute scroll
    let wrapped = word_wrap(&app.analysis_text, inner_w);
    let total = wrapped.len();
    let max_scroll = total.saturating_sub(inner_h);
    app.analysis_max_scroll.set(max_scroll); // cache so key handler can clamp

    // analysis_scroll is offset from bottom (0 = bottom, -N = N lines above bottom)
    let scroll = if app.analysis_auto_scroll {
        max_scroll
    } else {
        let from_top = max_scroll as i64 + app.analysis_scroll as i64;
        from_top.clamp(0, max_scroll as i64) as usize
    };

    // Build explicit Line objects so ratatui handles them correctly
    let lines: Vec<Line> = wrapped[scroll..]
        .iter()
        .take(inner_h)
        .map(|s| Line::from(s.as_str()))
        .collect();

    frame.render_widget(Paragraph::new(Text::from(lines)), inner);
}

// ─── Footer ──────────────────────────────────────────────────────────────────

fn render_footer(frame: &mut Frame, area: Rect) {
    let line = Line::from(vec![
        kb("↑↓ jk"), Span::raw(" Nav  "),
        kb("Space"), Span::raw(" Select  "),
        kb("A"), Span::raw(" Analyse  "),
        kb("R"), Span::raw(" Reload  "),
        kb("N/P"), Span::raw(" Page  "),
        kb("C"), Span::raw(" Clear sel  "),
        kb("I"), Span::raw(" Index  "),
        kb("L"), Span::raw(" LLM  "),
        kb("+/-"), Span::raw(" Level  "),
        kb("[/]"), Span::raw(" Scroll analysis  "),
        kb("H"), Span::raw(" Help  "),
        kb("Q"), Span::raw(" Quit"),
    ]);
    frame.render_widget(
        Paragraph::new(line).style(Style::default().bg(Color::DarkGray)),
        area,
    );
}

// ─── Index Picker Popup ──────────────────────────────────────────────────────

fn render_index_picker(frame: &mut Frame, app: &App, area: Rect) {
    let w = 64.min(area.width.saturating_sub(4));
    let h = 22.min(area.height.saturating_sub(4));
    let x = (area.width.saturating_sub(w)) / 2;
    let y = (area.height.saturating_sub(h)) / 2;
    let popup = Rect::new(x, y, w, h);

    frame.render_widget(Clear, popup);

    let items: Vec<ListItem> = app
        .indices
        .iter()
        .map(|idx| {
            let is_current = app.current_index.as_deref() == Some(idx.as_str());
            let style = if is_current {
                Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            ListItem::new(idx.as_str()).style(style)
        })
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .title(" Select Index  (↑↓ navigate · Enter confirm · Esc cancel) ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Yellow)),
        )
        .highlight_style(
            Style::default().bg(Color::Blue).add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ");

    let mut state = ListState::default();
    state.select(Some(app.index_picker_cursor));
    frame.render_stateful_widget(list, popup, &mut state);
}

// ─── Help Popup ──────────────────────────────────────────────────────────────

fn render_help(frame: &mut Frame, area: Rect) {
    let w = 52.min(area.width.saturating_sub(4));
    let h = 26.min(area.height.saturating_sub(4));
    let x = (area.width.saturating_sub(w)) / 2;
    let y = (area.height.saturating_sub(h)) / 2;
    let popup = Rect::new(x, y, w, h);

    frame.render_widget(Clear, popup);

    let rows = vec![
        ("Navigation", vec![
            ("↑ / k",       "Move cursor up"),
            ("↓ / j",       "Move cursor down"),
            ("N / →",       "Next page"),
            ("P / ←",       "Previous page"),
        ]),
        ("Selection", vec![
            ("Space",       "Toggle select entry"),
            ("C",           "Clear all selections"),
        ]),
        ("Analysis", vec![
            ("A",           "Analyse selected with LLM"),
            ("L",           "Toggle LLM  (Claude ↔ Ollama)"),
            ("[  /  ]",     "Scroll analysis panel up / down"),
        ]),
        ("Filters & Index", vec![
            ("I",           "Open index picker"),
            ("+  /  =",     "Increase minimum alert level"),
            ("-",           "Decrease minimum alert level"),
            ("R",           "Reload entries"),
        ]),
        ("General", vec![
            ("H / ?",       "Show this help"),
            ("Q / Ctrl-C",  "Quit"),
        ]),
    ];

    let mut lines: Vec<Line> = vec![Line::from("")];
    for (section, bindings) in &rows {
        lines.push(Line::from(Span::styled(
            format!("  {section}"),
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        )));
        for (key, desc) in bindings {
            lines.push(Line::from(vec![
                Span::raw("    "),
                Span::styled(
                    format!("{:<12}", key),
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                ),
                Span::raw("  "),
                Span::styled(*desc, Style::default().fg(Color::White)),
            ]));
        }
        lines.push(Line::from(""));
    }
    lines.push(Line::from(Span::styled(
        "  Press any key to close",
        Style::default().fg(Color::DarkGray),
    )));

    let block = Block::default()
        .title(" Help  (H / ?) ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    frame.render_widget(
        Paragraph::new(lines).block(block),
        popup,
    );
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn level_style(level: u8) -> Style {
    match level {
        12..=255 => Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        8..=11 => Style::default().fg(Color::Red),
        4..=7 => Style::default().fg(Color::Yellow),
        1..=3 => Style::default().fg(Color::Green),
        _ => Style::default(),
    }
}

/// Word-wrap `text` to `width` columns, returning one string per visual line.
fn word_wrap(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return text.lines().map(String::from).collect();
    }
    let mut result = Vec::new();
    for line in text.lines() {
        if line.is_empty() {
            result.push(String::new());
            continue;
        }
        let mut current = String::new();
        for word in line.split_whitespace() {
            let wlen = word.chars().count();
            let sep = if current.is_empty() { 0 } else { 1 };
            if current.chars().count() + sep + wlen <= width {
                if !current.is_empty() {
                    current.push(' ');
                }
                current.push_str(word);
            } else {
                if !current.is_empty() {
                    result.push(std::mem::take(&mut current));
                }
                // Force-break words longer than width
                if wlen > width {
                    let chars: Vec<char> = word.chars().collect();
                    let mut i = 0;
                    while i < chars.len() {
                        let end = (i + width).min(chars.len());
                        let chunk: String = chars[i..end].iter().collect();
                        if end == chars.len() {
                            current = chunk;
                        } else {
                            result.push(chunk);
                        }
                        i = end;
                    }
                } else {
                    current = word.to_string();
                }
            }
        }
        if !current.is_empty() {
            result.push(current);
        }
    }
    if result.is_empty() {
        result.push(String::new());
    }
    result
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() > max {
        format!("{}…", s.chars().take(max.saturating_sub(1)).collect::<String>())
    } else {
        s.to_string()
    }
}

fn kb(s: &'static str) -> Span<'static> {
    Span::styled(s, Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))
}
