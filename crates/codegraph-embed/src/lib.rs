#[cfg(feature = "local")]
pub mod local;
#[cfg(feature = "openai")]
pub mod openai;

use codegraph_core::config::EmbeddingConfig;
use codegraph_core::Result;

/// Embedding provider enum — avoids dyn dispatch issues with async traits.
pub enum Embedder {
    #[cfg(feature = "openai")]
    OpenAI(Box<openai::OpenAIProvider>),
    #[cfg(feature = "local")]
    Local(Box<local::LocalProvider>),
    Noop(NoopProvider),
}

impl Embedder {
    /// Generate embeddings for a batch of texts.
    pub async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        match self {
            #[cfg(feature = "openai")]
            Self::OpenAI(p) => p.embed(texts).await,
            #[cfg(feature = "local")]
            Self::Local(p) => p.embed(texts).await,
            Self::Noop(p) => p.embed(texts),
        }
    }

    /// The dimensionality of embedding vectors produced.
    pub fn dimensions(&self) -> usize {
        match self {
            #[cfg(feature = "openai")]
            Self::OpenAI(p) => p.dimensions,
            #[cfg(feature = "local")]
            Self::Local(p) => p.dimensions,
            Self::Noop(p) => p.dims,
        }
    }
}

/// Create an embedder from config.
pub fn from_config(config: &EmbeddingConfig) -> Result<Embedder> {
    match config.provider.as_str() {
        #[cfg(feature = "openai")]
        "openai" => {
            let openai_config = config.openai.as_ref().ok_or_else(|| {
                codegraph_core::Error::Embedding(
                    "OpenAI embedding config missing [embedding.openai] section".into(),
                )
            })?;
            Ok(Embedder::OpenAI(Box::new(openai::OpenAIProvider::new(
                &openai_config.model,
                config.dimensions,
            ))))
        }
        #[cfg(feature = "local")]
        "local" => {
            let local_config = config.local.as_ref().ok_or_else(|| {
                codegraph_core::Error::Embedding(
                    "Local embedding config missing [embedding.local] section".into(),
                )
            })?;
            Ok(Embedder::Local(Box::new(local::LocalProvider::new(
                &local_config.model_path,
                &local_config.tokenizer_path,
            )?)))
        }
        "none" | "disabled" => Ok(Embedder::Noop(NoopProvider::new(config.dimensions))),
        other => Err(codegraph_core::Error::Embedding(format!(
            "Unknown embedding provider: {other}. Available: openai, local, none"
        ))),
    }
}

/// A no-op provider that returns zero vectors. Used when embedding is disabled.
pub struct NoopProvider {
    dims: usize,
}

impl NoopProvider {
    pub fn new(dims: usize) -> Self {
        Self { dims }
    }

    pub fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        Ok(texts.iter().map(|_| vec![0.0; self.dims]).collect())
    }
}
