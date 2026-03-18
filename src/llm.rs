use anyhow::Result;
use futures_util::StreamExt;
use reqwest::Client;
use serde_json::{json, Value};
use tokio::sync::mpsc;

use crate::config::LlmConfig;
use crate::opensearch::{DailyStats, Entry};

#[derive(Clone)]
pub struct LlmClient {
    client: Client,
    pub config: LlmConfig,
}

impl LlmClient {
    pub fn new(config: LlmConfig) -> Self {
        Self { client: Client::new(), config }
    }

    /// Analyse a set of alert entries; streams text chunks via `tx`.
    pub async fn analyse(&self, entries: &[Entry], tx: mpsc::Sender<String>) -> Result<()> {
        let prompt = self.build_analysis_prompt(entries);
        self.stream(prompt, tx).await
    }

    /// Generate the executive-summary portion of a daily report; streams via `tx`.
    pub async fn generate_report_summary(
        &self,
        stats: &DailyStats,
        tx: mpsc::Sender<String>,
    ) -> Result<()> {
        let prompt = self.build_report_prompt(stats);
        self.stream(prompt, tx).await
    }

    // ── Provider resolution ───────────────────────────────────────────────────

    fn resolve_openrouter_model(&self) -> Option<String> {
        let tag = self.config.provider.strip_prefix("openrouter:")?;
        self.config.openrouter.models.iter()
            .find(|m| m.tag == tag)
            .map(|m| m.id.clone())
    }

    fn resolve_ollama_model(&self) -> Option<String> {
        let tag = self.config.provider.strip_prefix("local:")?;
        Some(
            self.config.ollama.models.iter()
                .find(|m| m.tag == tag)
                .map(|m| m.id.clone())
                .unwrap_or_else(|| tag.to_string()),
        )
    }

    // ── Prompt builders ───────────────────────────────────────────────────────

    fn build_analysis_prompt(&self, entries: &[Entry]) -> String {
        let mut out = String::from(
            "You are a senior SOC analyst. \
             Your job is to explain what these logs show in plain, clear terms. \
             Be descriptive and analytical: explain what each event means, what process or \
             system behaviour caused it, and how the events relate to each other. \
             Stay realistic — only describe what the data actually shows. \
             Do not invent attack scenarios or jump to dramatic conclusions. \
             If the logs look like normal system activity, say so plainly.\n\n",
        );
        out.push_str(&format!(
            "Total entries selected: {}\n\n--- LOG ENTRIES ---\n\n",
            entries.len()
        ));

        for (i, e) in entries.iter().enumerate() {
            let src = &e.raw["_source"];
            out.push_str(&format!(
                "[{}] {} | Level {} | Agent: {}\n",
                i + 1, e.timestamp, e.level, e.agent
            ));
            if e.rule_id.is_empty() {
                out.push_str(&format!("    {}\n", e.description));
            } else {
                out.push_str(&format!("    Rule {}: {}\n", e.rule_id, e.description));
            }
            if let Some(log) = src["full_log"].as_str() {
                let excerpt: String = log.chars().take(300).collect();
                out.push_str(&format!("    Log: {excerpt}\n"));
            }
            if let Some(techs) = src["rule"]["mitre"]["technique"].as_array() {
                let t: Vec<&str> = techs.iter().filter_map(|v| v.as_str()).collect();
                let ids: Vec<String> = src["rule"]["mitre"]["id"]
                    .as_array()
                    .map(|v| v.as_slice())
                    .unwrap_or(&[])
                    .iter()
                    .filter_map(|v| v.as_str())
                    .map(String::from)
                    .collect();
                if !t.is_empty() {
                    out.push_str(&format!(
                        "    MITRE: {} ({})\n",
                        t.join(", "), ids.join(", ")
                    ));
                }
            }
            if let Some(reasoning) = src["ai_analysis"]["reasoning"].as_str() {
                let confidence = src["ai_analysis"]["confidence"].as_f64().unwrap_or(0.0);
                out.push_str(&format!(
                    "    AI note ({:.0}%): {reasoning}\n",
                    confidence * 100.0
                ));
            }
            out.push('\n');
        }

        out.push_str(
            "--- END OF ENTRIES ---\n\n\
             Write one short paragraph explaining what the logs show. \
             No headers, no bullet points, no markdown. Stay factual.\n",
        );
        out
    }

    fn build_report_prompt(&self, stats: &DailyStats) -> String {
        let mut p = String::from(
            "You are a senior SOC analyst writing a daily security operations report.\n\
             Based on the statistics below, write a concise executive summary in markdown.\n\
             Cover:\n\
             1. Overall activity level and alert trend vs previous period.\n\
             2. Infrastructure health observations (agent coverage, noisy endpoints).\n\
             3. Top risk areas, notable rule patterns, and MITRE ATT&CK techniques detected.\n\
             4. Any recommended actions or watch items for the next shift.\n\
             Be factual and professional. Use 3–5 short paragraphs. Markdown bold/lists are fine.\n\n",
        );

        let trend = if stats.prev_period_total == 0 {
            "no previous period data".to_string()
        } else {
            let delta = stats.total as i64 - stats.prev_period_total as i64;
            let pct = (delta.abs() as f64 / stats.prev_period_total as f64 * 100.0).round() as i64;
            match delta.cmp(&0) {
                std::cmp::Ordering::Greater => format!("UP {pct}% vs previous {}-hour window ({} alerts)", stats.hours, stats.prev_period_total),
                std::cmp::Ordering::Less    => format!("DOWN {pct}% vs previous {}-hour window ({} alerts)", stats.hours, stats.prev_period_total),
                std::cmp::Ordering::Equal   => format!("UNCHANGED vs previous {}-hour window", stats.hours),
            }
        };

        p.push_str(&format!("**Period:** Last {} hours\n", stats.hours));
        p.push_str(&format!("**Index:** {}\n", stats.index));
        p.push_str(&format!("**Total Alerts:** {} ({})\n\n", stats.total, trend));

        let critical_high = stats.critical + stats.high;
        let ratio = if stats.total > 0 {
            format!("{:.1}%", critical_high as f64 / stats.total as f64 * 100.0)
        } else {
            "N/A".to_string()
        };
        p.push_str("**Severity Breakdown:**\n");
        p.push_str(&format!("- Critical (≥12): {}\n", stats.critical));
        p.push_str(&format!("- High (8–11): {}\n", stats.high));
        p.push_str(&format!("- Medium (4–7): {}\n", stats.medium));
        p.push_str(&format!("- Low (1–3): {}\n", stats.low));
        p.push_str(&format!("- Critical+High ratio: {ratio}\n\n"));

        p.push_str(&format!(
            "**Infrastructure:** {} unique agents reported alerts this period.\n\n",
            stats.unique_agents
        ));

        if !stats.top_agents.is_empty() {
            p.push_str("**Top Agents by Alert Volume:**\n");
            for (name, count) in &stats.top_agents {
                p.push_str(&format!("- {name}: {count} alerts\n"));
            }
            p.push('\n');
        }

        if !stats.top_rules.is_empty() {
            p.push_str("**Top Triggered Rules:**\n");
            for (id, desc, count) in &stats.top_rules {
                p.push_str(&format!("- Rule {id} ({count}x): {desc}\n"));
            }
            p.push('\n');
        }

        if !stats.top_mitre.is_empty() {
            p.push_str("**Top MITRE ATT&CK Techniques:**\n");
            for (technique, count) in &stats.top_mitre {
                p.push_str(&format!("- {technique}: {count} events\n"));
            }
            p.push('\n');
        }

        if !stats.top_entries.is_empty() {
            p.push_str("**Sample High/Critical Events:**\n");
            for e in stats.top_entries.iter().take(10) {
                p.push_str(&format!(
                    "- [{}  Lvl{}] {}: {}\n",
                    e.timestamp, e.level, e.agent, e.description
                ));
            }
            p.push('\n');
        }

        p.push_str("\nWrite the executive summary now:");
        p
    }

    // ── Shared streaming entry point ──────────────────────────────────────────

    async fn stream(&self, prompt: String, tx: mpsc::Sender<String>) -> Result<()> {
        if self.config.provider.starts_with("openrouter:") {
            let model_id = self.resolve_openrouter_model()
                .ok_or_else(|| anyhow::anyhow!("Unknown model tag: {}", self.config.provider))?;
            let api_key = if self.config.openrouter.api_key.is_empty() {
                std::env::var("OPENROUTER_API_KEY").unwrap_or_default()
            } else {
                self.config.openrouter.api_key.clone()
            };
            let url = format!("{}/chat/completions", self.config.openrouter.url);
            let body = json!({
                "model": model_id,
                "max_tokens": 2048,
                "stream": true,
                "messages": [{"role": "user", "content": prompt}]
            });
            let response = self.client.post(&url)
                .header("Authorization", format!("Bearer {api_key}"))
                .header("content-type", "application/json")
                .json(&body)
                .send()
                .await?;
            self.consume_sse(response, tx, |v| {
                v["choices"][0]["delta"]["content"].as_str().map(String::from)
            }).await
        } else if self.config.provider.starts_with("local:") {
            let model_id = self.resolve_ollama_model()
                .ok_or_else(|| anyhow::anyhow!("Unknown model tag: {}", self.config.provider))?;
            let url = format!("{}/api/chat", self.config.ollama.url);
            let body = json!({
                "model": model_id,
                "messages": [{"role": "user", "content": prompt}],
                "stream": true,
                "think": false,
            });
            let response = self.client.post(&url).json(&body).send().await?;
            self.consume_ndjson(response, tx, |v| {
                v["message"]["content"].as_str().map(String::from)
            }, |v| {
                v["done"].as_bool().unwrap_or(false)
            }).await
        } else {
            anyhow::bail!("Unknown LLM provider: {}", self.config.provider)
        }
    }

    // ── Shared stream consumers ───────────────────────────────────────────────

    /// Consume an SSE stream (OpenRouter / OpenAI format).
    /// `extract` pulls text from each parsed JSON event.
    async fn consume_sse<F>(
        &self,
        response: reqwest::Response,
        tx: mpsc::Sender<String>,
        extract: F,
    ) -> Result<()>
    where
        F: Fn(&Value) -> Option<String>,
    {
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("API error {status}: {body}");
        }

        let mut stream = response.bytes_stream();
        let mut buf = String::new();

        while let Some(chunk) = stream.next().await {
            buf.push_str(std::str::from_utf8(&chunk?)?);

            while let Some(pos) = buf.find('\n') {
                let line = buf[..pos].trim().to_owned();
                buf.drain(..=pos);

                if let Some(data) = line.strip_prefix("data: ") {
                    if data == "[DONE]" {
                        return Ok(());
                    }
                    if let Ok(v) = serde_json::from_str::<Value>(data) {
                        if let Some(text) = extract(&v) {
                            if tx.send(text).await.is_err() {
                                return Ok(());
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Consume a newline-delimited JSON stream (Ollama format).
    /// `extract` pulls text from each line; `is_done` detects the terminal message.
    async fn consume_ndjson<F, D>(
        &self,
        response: reqwest::Response,
        tx: mpsc::Sender<String>,
        extract: F,
        is_done: D,
    ) -> Result<()>
    where
        F: Fn(&Value) -> Option<String>,
        D: Fn(&Value) -> bool,
    {
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("API error {status}: {body}");
        }

        let mut stream = response.bytes_stream();
        let mut buf = String::new();

        while let Some(chunk) = stream.next().await {
            buf.push_str(std::str::from_utf8(&chunk?)?);

            while let Some(pos) = buf.find('\n') {
                let line = buf[..pos].trim().to_owned();
                buf.drain(..=pos);

                if line.is_empty() { continue; }

                if let Ok(v) = serde_json::from_str::<Value>(&line) {
                    if let Some(text) = extract(&v) {
                        if tx.send(text).await.is_err() {
                            return Ok(());
                        }
                    }
                    if is_done(&v) {
                        return Ok(());
                    }
                }
            }
        }
        Ok(())
    }
}
