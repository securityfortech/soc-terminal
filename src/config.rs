use serde::Deserialize;

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    pub opensearch: OpenSearchConfig,
    pub llm: LlmConfig,
    #[serde(default)]
    pub ui: UiConfig,
}

#[derive(Debug, Deserialize, Clone)]
pub struct OpenSearchConfig {
    pub url: String,
    pub username: String,
    pub password: String,
    #[serde(default)]
    pub verify_ssl: bool,
}

#[derive(Debug, Deserialize, Clone)]
pub struct LlmConfig {
    pub provider: String,
    pub openrouter: OpenRouterConfig,
    pub ollama: OllamaConfig,
}

#[derive(Debug, Deserialize, Clone)]
pub struct OllamaModel {
    pub id: String,   // e.g. "qwen3.5:0.8b"
    pub tag: String,  // e.g. "qwen3.5"
}

#[derive(Debug, Deserialize, Clone)]
pub struct OllamaConfig {
    #[serde(default = "default_ollama_url")]
    pub url: String,
    #[serde(default)]
    pub models: Vec<OllamaModel>,
}

fn default_ollama_url() -> String {
    "http://localhost:11434".to_string()
}

#[derive(Debug, Deserialize, Clone)]
pub struct OpenRouterModel {
    pub id: String,   // e.g. "anthropic/claude-sonnet-4-6"
    pub tag: String,  // e.g. "sonnet4.6"
}

#[derive(Debug, Deserialize, Clone)]
pub struct OpenRouterConfig {
    #[serde(default)]
    pub api_key: String,
    #[serde(default = "default_openrouter_url")]
    pub url: String,
    #[serde(default)]
    pub models: Vec<OpenRouterModel>,
}

fn default_openrouter_url() -> String {
    "https://openrouter.ai/api/v1".to_string()
}

#[derive(Debug, Deserialize, Clone)]
pub struct UiConfig {
    #[serde(default = "default_page_size")]
    pub page_size: usize,
    /// Directory for exports and reports. Defaults to ".".
    #[serde(default = "default_output_dir")]
    pub output_dir: String,
}

fn default_page_size() -> usize {
    50
}

fn default_output_dir() -> String {
    ".".to_string()
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            page_size: default_page_size(),
            output_dir: default_output_dir(),
        }
    }
}
