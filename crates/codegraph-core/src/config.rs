use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::error::Result;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub neo4j: Neo4jConfig,
    pub indexing: IndexingConfig,
    pub embedding: EmbeddingConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Neo4jConfig {
    pub uri: String,
    pub username: String,
    pub password: String,
    #[serde(default = "default_database")]
    pub database: String,
    #[serde(default = "default_max_connections")]
    pub max_connections: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexingConfig {
    /// Paths to index
    pub paths: Vec<String>,
    /// Glob patterns to exclude
    #[serde(default = "default_excludes")]
    pub exclude: Vec<String>,
    /// Whether to compute embeddings during indexing
    #[serde(default = "default_true")]
    pub embed: bool,
    /// Batch size for Neo4j upserts
    #[serde(default = "default_batch_size")]
    pub batch_size: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingConfig {
    /// "openai" or "local"
    pub provider: String,
    /// Embedding vector dimensions
    #[serde(default = "default_dimensions")]
    pub dimensions: usize,
    #[serde(default)]
    pub openai: Option<OpenAIEmbeddingConfig>,
    #[serde(default)]
    pub local: Option<LocalEmbeddingConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAIEmbeddingConfig {
    #[serde(default = "default_openai_model")]
    pub model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalEmbeddingConfig {
    pub model_path: String,
    pub tokenizer_path: String,
}

impl Config {
    /// Load config from a TOML file.
    pub fn from_file(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let config: Config = toml::from_str(&content)?;
        Ok(config)
    }

    /// Search for config file in standard locations.
    pub fn discover(start_dir: &Path) -> Option<PathBuf> {
        let candidates = [
            "codegraph.toml",
            ".codegraph.toml",
            ".config/codegraph.toml",
        ];
        let mut dir = Some(start_dir);
        while let Some(d) = dir {
            for candidate in &candidates {
                let path = d.join(candidate);
                if path.exists() {
                    return Some(path);
                }
            }
            dir = d.parent();
        }
        None
    }

    /// Generate a default config TOML string.
    pub fn default_toml() -> &'static str {
        include_str!("../../../codegraph.toml")
    }
}

fn default_database() -> String {
    "neo4j".to_string()
}
fn default_max_connections() -> usize {
    10
}
fn default_excludes() -> Vec<String> {
    vec![
        "target/**".into(),
        "node_modules/**".into(),
        ".git/**".into(),
        "vendor/**".into(),
    ]
}
fn default_true() -> bool {
    true
}
fn default_batch_size() -> usize {
    500
}
fn default_dimensions() -> usize {
    768
}
fn default_openai_model() -> String {
    "text-embedding-3-small".to_string()
}
