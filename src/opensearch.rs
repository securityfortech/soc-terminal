use anyhow::Result;
use reqwest::Client;
use serde_json::{json, Value};

use crate::config::OpenSearchConfig;

#[derive(Debug, Clone)]
pub struct Entry {
    pub id: String,
    pub timestamp: String,
    pub level: u8,
    pub agent: String,
    pub rule_id: String,
    pub description: String,
    pub raw: Value,
}

#[derive(Debug, Default, Clone)]
pub struct Filters {
    pub min_level: u8,
    pub agent: Option<String>,
    pub hours: Option<u32>,
}

pub struct OpenSearchClient {
    client: Client,
    config: OpenSearchConfig,
}

impl OpenSearchClient {
    pub fn new(config: OpenSearchConfig) -> Result<Self> {
        let client = Client::builder()
            .danger_accept_invalid_certs(!config.verify_ssl)
            .build()?;
        Ok(Self { client, config })
    }

    pub async fn list_indices(&self) -> Result<Vec<String>> {
        let url = format!("{}/_cat/indices?format=json", self.config.url);
        let resp: Vec<Value> = self
            .client
            .get(&url)
            .basic_auth(&self.config.username, Some(&self.config.password))
            .send()
            .await?
            .json()
            .await?;

        let mut indices: Vec<String> = resp
            .iter()
            .filter_map(|i| i["index"].as_str().map(String::from))
            .filter(|i| !i.starts_with('.'))
            .collect();
        indices.sort_by(|a, b| b.cmp(a));
        Ok(indices)
    }

    pub async fn get_entries(
        &self,
        index: &str,
        filters: &Filters,
        size: usize,
        from: usize,
    ) -> Result<(Vec<Entry>, usize)> {
        let url = format!("{}/{}/_search", self.config.url, index);

        let mut must: Vec<Value> = vec![];
        if filters.min_level > 1 {
            must.push(json!({"range": {"rule.level": {"gte": filters.min_level}}}));
        }
        if let Some(agent) = &filters.agent {
            must.push(json!({"wildcard": {"agent.name": format!("*{agent}*")}}));
        }
        if let Some(hours) = filters.hours {
            must.push(json!({"range": {"@timestamp": {"gte": format!("now-{hours}h")}}}));
        }

        let query = if must.is_empty() {
            json!({"match_all": {}})
        } else {
            json!({"bool": {"must": must}})
        };

        let body = json!({
            "query": query,
            "sort": [{"@timestamp": {"order": "desc", "unmapped_type": "date"}}],
            "size": size,
            "from": from,
        });

        let resp: Value = self
            .client
            .post(&url)
            .basic_auth(&self.config.username, Some(&self.config.password))
            .json(&body)
            .send()
            .await?
            .json()
            .await?;

        if let Some(err) = resp["error"].as_object() {
            anyhow::bail!("OpenSearch error: {}", err["reason"].as_str().unwrap_or("unknown"));
        }

        let total = resp["hits"]["total"]["value"].as_u64().unwrap_or(0) as usize;
        let hits = resp["hits"]["hits"].as_array().cloned().unwrap_or_default();

        let entries = hits
            .into_iter()
            .map(|hit| {
                let src = &hit["_source"];
                let ts = src["@timestamp"]
                    .as_str()
                    .or_else(|| src["timestamp"].as_str())
                    .unwrap_or("")
                    .chars()
                    .take(19)
                    .collect::<String>()
                    .replace('T', " ");

                let level = src["rule"]["level"]
                    .as_u64()
                    .or_else(|| src["rule_level"].as_u64())
                    .unwrap_or(0) as u8;

                let agent = src["agent"]["name"]
                    .as_str()
                    .or_else(|| src["host"].as_str())
                    .unwrap_or("?")
                    .to_string();

                let rule_id = match &src["rule"]["id"] {
                    Value::String(s) => s.clone(),
                    Value::Number(n) => n.to_string(),
                    _ => String::new(),
                };

                let description = src["rule"]["description"]
                    .as_str()
                    .or_else(|| src["rule_description"].as_str())
                    .unwrap_or("")
                    .to_string();

                Entry {
                    id: hit["_id"].as_str().unwrap_or("").to_string(),
                    timestamp: ts,
                    level,
                    agent,
                    rule_id,
                    description,
                    raw: hit.clone(),
                }
            })
            .collect();

        Ok((entries, total))
    }
}
