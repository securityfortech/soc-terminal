use anyhow::Result;
use reqwest::Client;
use serde_json::{json, Value};

use crate::config::OpenSearchConfig;

#[derive(Debug, Clone)]
pub struct DailyStats {
    pub index: String,
    pub hours: u32,

    // ── Alert volume ──────────────────────────────────────────────────────────
    pub total: usize,
    pub prev_period_total: usize,   // previous same-length window (for trend)

    // ── Severity breakdown ────────────────────────────────────────────────────
    pub critical: usize,  // level 12+
    pub high: usize,      // level 8–11
    pub medium: usize,    // level 4–7
    pub low: usize,       // level 1–3

    // ── Infrastructure health ─────────────────────────────────────────────────
    pub unique_agents: usize,                     // cardinality of reporting agents
    pub top_agents: Vec<(String, usize)>,         // (name, count)

    // ── Top rules / MITRE ─────────────────────────────────────────────────────
    pub top_rules: Vec<(String, String, usize)>,  // (rule_id, description, count)
    pub top_mitre: Vec<(String, usize)>,          // (technique, count)

    // ── Notable events ────────────────────────────────────────────────────────
    pub top_entries: Vec<Entry>,                  // top critical/high entries
}

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

    /// Count alerts between `from_hours` ago and `to_hours` ago.
    async fn count_alerts(&self, index: &str, from_hours: u32, to_hours: u32) -> Result<usize> {
        let url = format!("{}/{}/_count", self.config.url, index);
        let body = json!({
            "query": {
                "range": {
                    "@timestamp": {
                        "gte": format!("now-{from_hours}h"),
                        "lt":  format!("now-{to_hours}h")
                    }
                }
            }
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
        Ok(resp["count"].as_u64().unwrap_or(0) as usize)
    }

    /// Fetch aggregated statistics for the daily report skill.
    pub async fn get_daily_stats(&self, index: &str, hours: u32) -> Result<DailyStats> {
        let url = format!("{}/{}/_search", self.config.url, index);

        let body = json!({
            "query": {
                "range": { "@timestamp": { "gte": format!("now-{hours}h") } }
            },
            "size": 0,
            "aggs": {
                "by_severity": {
                    "range": {
                        "field": "rule.level",
                        "ranges": [
                            { "key": "low",      "from": 1,  "to": 4  },
                            { "key": "medium",   "from": 4,  "to": 8  },
                            { "key": "high",     "from": 8,  "to": 12 },
                            { "key": "critical", "from": 12 }
                        ]
                    }
                },
                "unique_agents": {
                    "cardinality": { "field": "agent.name" }
                },
                "top_agents": {
                    "terms": { "field": "agent.name", "size": 10 }
                },
                "top_rules": {
                    "terms": { "field": "rule.id", "size": 10 },
                    "aggs": {
                        "sample_desc": {
                            "top_hits": {
                                "size": 1,
                                "_source": ["rule.description"]
                            }
                        }
                    }
                },
                "top_mitre": {
                    "terms": { "field": "rule.mitre.technique", "size": 10 }
                }
            }
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

        let mut critical = 0usize;
        let mut high     = 0usize;
        let mut medium   = 0usize;
        let mut low      = 0usize;

        if let Some(buckets) = resp["aggregations"]["by_severity"]["buckets"].as_array() {
            for b in buckets {
                let count = b["doc_count"].as_u64().unwrap_or(0) as usize;
                match b["key"].as_str().unwrap_or("") {
                    "critical" => critical = count,
                    "high"     => high     = count,
                    "medium"   => medium   = count,
                    "low"      => low      = count,
                    _ => {}
                }
            }
        }

        let unique_agents = resp["aggregations"]["unique_agents"]["value"]
            .as_u64()
            .unwrap_or(0) as usize;

        let top_agents = resp["aggregations"]["top_agents"]["buckets"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .map(|b| {
                        let name  = b["key"].as_str().unwrap_or("?").to_string();
                        let count = b["doc_count"].as_u64().unwrap_or(0) as usize;
                        (name, count)
                    })
                    .collect()
            })
            .unwrap_or_default();

        let top_rules = resp["aggregations"]["top_rules"]["buckets"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .map(|b| {
                        let rule_id = match &b["key"] {
                            Value::String(s) => s.clone(),
                            Value::Number(n) => n.to_string(),
                            _ => "?".to_string(),
                        };
                        let count = b["doc_count"].as_u64().unwrap_or(0) as usize;
                        let desc  = b["sample_desc"]["hits"]["hits"]
                            .as_array()
                            .and_then(|h| h.first())
                            .and_then(|h| h["_source"]["rule"]["description"].as_str())
                            .unwrap_or("")
                            .to_string();
                        (rule_id, desc, count)
                    })
                    .collect()
            })
            .unwrap_or_default();

        let top_mitre = resp["aggregations"]["top_mitre"]["buckets"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .map(|b| {
                        let technique = b["key"].as_str().unwrap_or("?").to_string();
                        let count = b["doc_count"].as_u64().unwrap_or(0) as usize;
                        (technique, count)
                    })
                    .collect()
            })
            .unwrap_or_default();

        // Previous period: same window shifted back
        let prev_total = self.count_alerts(index, hours * 2, hours).await.unwrap_or(0);

        // Fetch top critical/high entries separately
        let filters = Filters { min_level: 8, agent: None, hours: Some(hours) };
        let (top_entries, _) = self.get_entries(index, &filters, 20, 0).await?;

        Ok(DailyStats {
            index: index.to_string(),
            hours,
            total,
            prev_period_total: prev_total,
            critical,
            high,
            medium,
            low,
            unique_agents,
            top_agents,
            top_rules,
            top_mitre,
            top_entries,
        })
    }
}
