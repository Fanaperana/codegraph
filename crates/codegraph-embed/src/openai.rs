use async_openai::config::OpenAIConfig;
use async_openai::types::embeddings::{CreateEmbeddingRequestArgs, EmbeddingInput};
use async_openai::Client;
use tracing::debug;

use codegraph_core::Result;
use codegraph_core::error::Error;

pub struct OpenAIProvider {
    client: Client<OpenAIConfig>,
    model: String,
    pub dimensions: usize,
}

impl OpenAIProvider {
    pub fn new(model: &str, dimensions: usize) -> Self {
        Self {
            client: Client::new(),
            model: model.to_string(),
            dimensions,
        }
    }

    pub async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(vec![]);
        }

        let mut all_embeddings = Vec::with_capacity(texts.len());

        // Batch in groups of 2048 (OpenAI limit)
        for chunk in texts.chunks(2048) {
            let input: Vec<&str> = chunk.iter().map(|s| s.as_str()).collect();

            let request = CreateEmbeddingRequestArgs::default()
                .model(&self.model)
                .input(EmbeddingInput::StringArray(
                    input.iter().map(|s| s.to_string()).collect(),
                ))
                .dimensions(self.dimensions as u32)
                .build()
                .map_err(|e: async_openai::error::OpenAIError| Error::Embedding(e.to_string()))?;

            let response = self
                .client
                .embeddings()
                .create(request)
                .await
                .map_err(|e: async_openai::error::OpenAIError| Error::Embedding(e.to_string()))?;

            for item in response.data {
                all_embeddings.push(item.embedding);
            }

            debug!("Embedded batch of {} texts", chunk.len());
        }

        Ok(all_embeddings)
    }
}
