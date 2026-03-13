use anyhow::Result;
use futures_util::StreamExt;
use reqwest::Client;
use serde_json::{json, Value};
use tokio::sync::mpsc;

use crate::config::LlmConfig;
use crate::opensearch::Entry;

#[derive(Clone)]
pub struct LlmClient {
    client: Client,
    pub config: LlmConfig,
}

impl LlmClient {
    pub fn new(config: LlmConfig) -> Self {
        Self { client: Client::new(), config }
    }

    /// Streams analysis chunks via `tx`. Drops `tx` when done (signals completion to receiver).
    pub async fn analyse(&self, entries: &[Entry], tx: mpsc::Sender<String>) -> Result<()> {
        match self.config.provider.as_str() {
            "claude" => self.analyse_claude(entries, tx).await,
            "ollama" => self.analyse_ollama(entries, tx).await,
            p => anyhow::bail!("Unknown LLM provider: {p}"),
        }
    }

    fn build_prompt(&self, entries: &[Entry]) -> String {
        let mut out = String::from(
            "You are a senior SOC analyst. \
             Your job is to explain what these logs show in plain, clear terms. \
             Be descriptive and analytical: explain what each event means, what process or \
             system behaviour caused it, and how the events relate to each other. \
             Stay realistic — only describe what the data actually shows. \
             Do not invent attack scenarios or jump to dramatic conclusions. \
             If the logs look like normal system activity, say so plainly.\n\n",
        );
        out.push_str(&format!("Total entries selected: {}\n\n--- LOG ENTRIES ---\n\n", entries.len()));

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
                    .unwrap_or(&vec![])
                    .iter()
                    .filter_map(|v| v.as_str())
                    .map(String::from)
                    .collect();
                if !t.is_empty() {
                    out.push_str(&format!("    MITRE: {} ({})\n", t.join(", "), ids.join(", ")));
                }
            }
            if let Some(reasoning) = src["ai_analysis"]["reasoning"].as_str() {
                let confidence = src["ai_analysis"]["confidence"].as_f64().unwrap_or(0.0);
                out.push_str(&format!("    AI note ({:.0}%): {reasoning}\n", confidence * 100.0));
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

    async fn analyse_claude(&self, entries: &[Entry], tx: mpsc::Sender<String>) -> Result<()> {
        let api_key = if self.config.claude.api_key.is_empty() {
            std::env::var("ANTHROPIC_API_KEY").unwrap_or_default()
        } else {
            self.config.claude.api_key.clone()
        };

        let body = json!({
            "model": self.config.claude.model,
            "max_tokens": 2048,
            "stream": true,
            "messages": [{"role": "user", "content": self.build_prompt(entries)}]
        });

        let response = self
            .client
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Claude API error {status}: {body}");
        }

        let mut stream = response.bytes_stream();
        let mut buf = String::new();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            buf.push_str(std::str::from_utf8(&chunk)?);

            // Process complete SSE lines
            while let Some(newline) = buf.find('\n') {
                let line = buf[..newline].trim().to_string();
                buf = buf[newline + 1..].to_string();

                if let Some(data) = line.strip_prefix("data: ") {
                    if data == "[DONE]" {
                        return Ok(());
                    }
                    if let Ok(v) = serde_json::from_str::<Value>(data) {
                        if let Some(text) = v["delta"]["text"].as_str() {
                            if tx.send(text.to_string()).await.is_err() {
                                return Ok(()); // receiver dropped
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }

    async fn analyse_ollama(&self, entries: &[Entry], tx: mpsc::Sender<String>) -> Result<()> {
        let url = format!("{}/api/chat", self.config.ollama.url);
        let body = json!({
            "model": self.config.ollama.model,
            "messages": [{"role": "user", "content": self.build_prompt(entries)}],
            "stream": true,
            "think": false,   // disable Qwen3 reasoning/think block
        });

        let response = self.client.post(&url).json(&body).send().await?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            anyhow::bail!("Ollama error {status}: {text}");
        }

        let mut stream = response.bytes_stream();
        let mut buf = String::new();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            buf.push_str(std::str::from_utf8(&chunk)?);

            while let Some(newline) = buf.find('\n') {
                let line = buf[..newline].trim().to_string();
                buf = buf[newline + 1..].to_string();

                if line.is_empty() {
                    continue;
                }
                if let Ok(v) = serde_json::from_str::<Value>(&line) {
                    if let Some(content) = v["message"]["content"].as_str() {
                        if tx.send(content.to_string()).await.is_err() {
                            return Ok(());
                        }
                    }
                    if v["done"].as_bool().unwrap_or(false) {
                        return Ok(());
                    }
                }
            }
        }
        Ok(())
    }
}
