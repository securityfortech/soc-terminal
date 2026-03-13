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
    pub claude: ClaudeConfig,
    pub ollama: OllamaConfig,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ClaudeConfig {
    #[serde(default)]
    pub api_key: String,
    #[serde(default = "default_claude_model")]
    pub model: String,
}

fn default_claude_model() -> String {
    "claude-sonnet-4-6".to_string()
}

#[derive(Debug, Deserialize, Clone)]
pub struct OllamaConfig {
    #[serde(default = "default_ollama_url")]
    pub url: String,
    #[serde(default = "default_ollama_model")]
    pub model: String,
}

fn default_ollama_url() -> String {
    "http://localhost:11434".to_string()
}

fn default_ollama_model() -> String {
    "llama3.2".to_string()
}

#[derive(Debug, Deserialize, Clone)]
pub struct UiConfig {
    #[serde(default = "default_page_size")]
    pub page_size: usize,
}

fn default_page_size() -> usize {
    50
}

impl Default for UiConfig {
    fn default() -> Self {
        Self { page_size: default_page_size() }
    }
}
