use anyhow::Result;
use chrono::Local;

use crate::opensearch::DailyStats;

/// Build the complete markdown report and write it to disk.
/// Returns the full path on success.
pub fn build_and_save(stats: &DailyStats, executive_summary: &str, output_dir: &str) -> Result<String> {
    let now  = Local::now();
    let date = now.format("%Y-%m-%d").to_string();
    let time = now.format("%H:%M:%S").to_string();

    let md = build_markdown(stats, executive_summary, &date, &time);

    let dir = std::path::Path::new(output_dir);
    if !dir.exists() {
        std::fs::create_dir_all(dir)?;
    }
    let path = dir.join(format!("soc-report-{date}.md"));
    std::fs::write(&path, &md)?;
    Ok(path.display().to_string())
}

fn build_markdown(stats: &DailyStats, executive_summary: &str, date: &str, time: &str) -> String {
    let mut md = String::new();

    // ── Title ─────────────────────────────────────────────────────────────────
    md.push_str(&format!("# SOC Daily Activity Report — {date}\n\n"));
    md.push_str(&format!("| | |\n|---|---|\n"));
    md.push_str(&format!("| **Date** | {date} |\n"));
    md.push_str(&format!("| **Report period** | Last {} hours |\n", stats.hours));
    md.push_str(&format!("| **Data source** | `{}` |\n", stats.index));
    md.push_str(&format!("| **Generated** | {date} {time} |\n\n"));
    md.push_str("---\n\n");

    // ── Executive Summary ─────────────────────────────────────────────────────
    md.push_str("## Executive Summary\n\n");
    md.push_str(executive_summary.trim());
    md.push_str("\n\n---\n\n");

    // ── KPIs ──────────────────────────────────────────────────────────────────
    md.push_str("## KPIs\n\n");

    let critical_high = stats.critical + stats.high;
    let ch_ratio = if stats.total > 0 {
        format!("{:.1}%", critical_high as f64 / stats.total as f64 * 100.0)
    } else {
        "N/A".to_string()
    };

    let trend_arrow = if stats.prev_period_total == 0 {
        "—".to_string()
    } else {
        let delta = stats.total as i64 - stats.prev_period_total as i64;
        let pct = if stats.prev_period_total > 0 {
            (delta.abs() as f64 / stats.prev_period_total as f64 * 100.0).round() as i64
        } else {
            0
        };
        if delta > 0 {
            format!("▲ {pct}% vs prev period ({} alerts)", stats.prev_period_total)
        } else if delta < 0 {
            format!("▼ {pct}% vs prev period ({} alerts)", stats.prev_period_total)
        } else {
            format!("= unchanged vs prev period ({} alerts)", stats.prev_period_total)
        }
    };

    md.push_str("| KPI | Value |\n|-----|-------|\n");
    md.push_str(&format!("| Total alerts | **{}** |\n", stats.total));
    md.push_str(&format!("| Alert trend | {} |\n", trend_arrow));
    md.push_str(&format!("| Critical (≥12) | {} |\n", stats.critical));
    md.push_str(&format!("| High (8–11) | {} |\n", stats.high));
    md.push_str(&format!("| Medium (4–7) | {} |\n", stats.medium));
    md.push_str(&format!("| Low (1–3) | {} |\n", stats.low));
    md.push_str(&format!("| Critical+High ratio | {} |\n", ch_ratio));
    md.push_str(&format!("| Unique reporting agents | {} |\n\n", stats.unique_agents));
    md.push_str("---\n\n");

    // ── Severity Breakdown ────────────────────────────────────────────────────
    md.push_str("## Alert Statistics\n\n");
    md.push_str("### Severity Breakdown\n\n");
    md.push_str("| Severity | Level Range | Count | Share |\n");
    md.push_str("|----------|-------------|-------|-------|\n");

    let severities = [
        ("Critical", "≥12",   stats.critical),
        ("High",     "8–11",  stats.high),
        ("Medium",   "4–7",   stats.medium),
        ("Low",      "1–3",   stats.low),
    ];
    for (label, range, count) in &severities {
        let share = if stats.total > 0 {
            format!("{:.1}%", *count as f64 / stats.total as f64 * 100.0)
        } else {
            "—".to_string()
        };
        md.push_str(&format!("| {label} | {range} | {count} | {share} |\n"));
    }
    md.push_str(&format!("| **Total** | — | **{}** | 100% |\n\n", stats.total));

    // ── Infrastructure Health ─────────────────────────────────────────────────
    md.push_str("---\n\n## Infrastructure Health\n\n");
    md.push_str(&format!(
        "**{} unique agent(s)** reported alerts during this period.\n\n",
        stats.unique_agents
    ));

    if !stats.top_agents.is_empty() {
        md.push_str("### Top 10 Agents by Alert Volume\n\n");
        md.push_str("| # | Agent | Alerts | Share |\n");
        md.push_str("|---|-------|--------|-------|\n");
        for (i, (name, count)) in stats.top_agents.iter().enumerate() {
            let share = if stats.total > 0 {
                format!("{:.1}%", *count as f64 / stats.total as f64 * 100.0)
            } else {
                "—".to_string()
            };
            md.push_str(&format!("| {} | `{}` | {} | {} |\n", i + 1, name, count, share));
        }
        md.push('\n');
    }

    // ── Top Rules ─────────────────────────────────────────────────────────────
    if !stats.top_rules.is_empty() {
        md.push_str("---\n\n## Top Triggered Rules\n\n");
        md.push_str("| # | Rule ID | Description | Count |\n");
        md.push_str("|---|---------|-------------|-------|\n");
        for (i, (rule_id, desc, count)) in stats.top_rules.iter().enumerate() {
            let desc_cell = desc.replace('|', "\\|");
            md.push_str(&format!("| {} | `{}` | {} | {} |\n", i + 1, rule_id, desc_cell, count));
        }
        md.push('\n');
    }

    // ── MITRE ATT&CK ─────────────────────────────────────────────────────────
    if !stats.top_mitre.is_empty() {
        md.push_str("---\n\n## MITRE ATT&CK Techniques Detected\n\n");
        md.push_str("| # | Technique | Event Count |\n");
        md.push_str("|---|-----------|-------------|\n");
        for (i, (technique, count)) in stats.top_mitre.iter().enumerate() {
            md.push_str(&format!("| {} | {} | {} |\n", i + 1, technique, count));
        }
        md.push('\n');
    }

    // ── Notable Events ────────────────────────────────────────────────────────
    if !stats.top_entries.is_empty() {
        md.push_str("---\n\n## Notable Events (Critical & High)\n\n");
        md.push_str("| Timestamp | Level | Agent | Rule | Description |\n");
        md.push_str("|-----------|-------|-------|------|-------------|\n");
        for e in &stats.top_entries {
            let desc = e.description.replace('|', "\\|");
            md.push_str(&format!(
                "| {} | {} | `{}` | {} | {} |\n",
                e.timestamp, e.level, e.agent, e.rule_id, desc
            ));
        }
        md.push('\n');
    }

    // ── Footer ────────────────────────────────────────────────────────────────
    md.push_str("---\n\n");
    md.push_str(&format!(
        "*Generated by SOC Terminal on {date} at {time}*\n"
    ));

    md
}
