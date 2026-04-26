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
            let attention_mask_u32 = encoding.get_attention_mask().to_vec();
            let attention_mask: Vec<i64> =
                attention_mask_u32.iter().map(|&m| m as i64).collect();

            let seq_len = input_ids.len();
            let shape = vec![1i64, seq_len as i64];

            let input_ids_tensor = Tensor::from_array((shape.clone(), input_ids))
                .map_err(|e| Error::Embedding(format!("Tensor error: {e}")))?;
            let attention_mask_tensor = Tensor::from_array((shape, attention_mask))
                .map_err(|e| Error::Embedding(format!("Tensor error: {e}")))?;

            let mut session_guard = self
                .session
                .lock()
                .map_err(|e| Error::Embedding(format!("Session lock error: {e}")))?;

            let outputs = session_guard
                .run(ort::inputs![
                    "input_ids" => input_ids_tensor,
                    "attention_mask" => attention_mask_tensor,
                ])
                .map_err(|e| Error::Embedding(format!("Inference failed: {e}")))?;

            // last_hidden_state has shape [1, seq_len, dims], flattened.
            let (_shape, data) = outputs[0]
                .try_extract_tensor::<f32>()
                .map_err(|e| Error::Embedding(format!("Output extraction failed: {e}")))?;

            // Mean pooling weighted by attention mask, as used by
            // sentence-transformers/all-mpnet-base-v2.
            let dims = self.dimensions;
            let mut pooled = vec![0f32; dims];
            let mut mask_sum: f32 = 0.0;
            for (token_idx, &mask) in attention_mask_u32.iter().enumerate() {
                if mask == 0 {
                    continue;
                }
                let offset = token_idx * dims;
                for d in 0..dims {
                    pooled[d] += data[offset + d];
                }
                mask_sum += 1.0;
            }
            if mask_sum > 0.0 {
                for v in &mut pooled {
                    *v /= mask_sum;
                }
            }
            let embedding = pooled;
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
