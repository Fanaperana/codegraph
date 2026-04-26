use std::path::Path;
use std::sync::Mutex;

use codegraph_core::Result;
use codegraph_core::error::Error;
use ort::session::Session;
use ort::value::Tensor;
use tracing::debug;

pub struct LocalProvider {
    session: Mutex<Session>,
    tokenizer: tokenizers::Tokenizer,
    pub dimensions: usize,
}

impl LocalProvider {
    pub fn new(model_path: &str, tokenizer_path: &str) -> Result<Self> {
        let session = Session::builder()
            .and_then(|mut b| b.commit_from_file(Path::new(model_path)))
            .map_err(|e| Error::Embedding(format!("Failed to load ONNX model: {e}")))?;

        let tokenizer = tokenizers::Tokenizer::from_file(tokenizer_path)
            .map_err(|e| Error::Embedding(format!("Failed to load tokenizer: {e}")))?;

        let dimensions = 768;

        Ok(Self {
            session: Mutex::new(session),
            tokenizer,
            dimensions,
        })
    }

    pub async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let mut results = Vec::with_capacity(texts.len());

        for text in texts {
            let encoding = self
                .tokenizer
                .encode(text.as_str(), true)
                .map_err(|e| Error::Embedding(format!("Tokenization failed: {e}")))?;

            let input_ids: Vec<i64> = encoding.get_ids().iter().map(|&id| id as i64).collect();
            let attention_mask: Vec<i64> = encoding
                .get_attention_mask()
                .iter()
                .map(|&m| m as i64)
                .collect();
            let token_type_ids: Vec<i64> = encoding
                .get_type_ids()
                .iter()
                .map(|&t| t as i64)
                .collect();

            let seq_len = input_ids.len();
            let shape = vec![1i64, seq_len as i64];

            let input_ids_tensor = Tensor::from_array((shape.clone(), input_ids))
                .map_err(|e| Error::Embedding(format!("Tensor error: {e}")))?;
            let attention_mask_tensor = Tensor::from_array((shape.clone(), attention_mask))
                .map_err(|e| Error::Embedding(format!("Tensor error: {e}")))?;
            let token_type_ids_tensor = Tensor::from_array((shape, token_type_ids))
                .map_err(|e| Error::Embedding(format!("Tensor error: {e}")))?;

            let mut session_guard = self
                .session
                .lock()
                .map_err(|e| Error::Embedding(format!("Session lock error: {e}")))?;

            let outputs = session_guard
                .run(ort::inputs![
                    "input_ids" => input_ids_tensor,
                    "attention_mask" => attention_mask_tensor,
                    "token_type_ids" => token_type_ids_tensor,
                ])
                .map_err(|e| Error::Embedding(format!("Inference failed: {e}")))?;

            // Extract the [CLS] token embedding (first token of last_hidden_state)
            // try_extract_tensor returns (&Shape, &[f32])
            let (_shape, data) = outputs[0]
                .try_extract_tensor::<f32>()
                .map_err(|e| Error::Embedding(format!("Output extraction failed: {e}")))?;

            // data is flattened [1, seq_len, dims] — take first dims elements for [CLS]
            let embedding: Vec<f32> = data[..self.dimensions].to_vec();
            drop(outputs);
            drop(session_guard);

            // L2 normalize
            let norm: f32 = embedding.iter().map(|x| x * x).sum::<f32>().sqrt();
            let normalized: Vec<f32> = if norm > 0.0 {
                embedding.iter().map(|x| x / norm).collect()
            } else {
                embedding
            };

            results.push(normalized);
        }

        debug!("Embedded {} texts locally", texts.len());
        Ok(results)
    }
}
