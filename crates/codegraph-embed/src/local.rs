use std::path::Path;

use codegraph_core::Result;
use codegraph_core::error::Error;
use tracing::debug;

pub struct LocalProvider {
    session: ort::session::Session,
    tokenizer: tokenizers::Tokenizer,
    pub dimensions: usize,
}

impl LocalProvider {
    pub fn new(model_path: &str, tokenizer_path: &str) -> Result<Self> {
        let session = ort::session::Session::builder()
            .and_then(|b| b.with_model_from_file(Path::new(model_path)))
            .map_err(|e| Error::Embedding(format!("Failed to load ONNX model: {e}")))?;

        let tokenizer = tokenizers::Tokenizer::from_file(tokenizer_path)
            .map_err(|e| Error::Embedding(format!("Failed to load tokenizer: {e}")))?;

        // Infer dimensions from model output shape or default to 384 (MiniLM)
        // let dimensions = 384;
        
        // all-mpnet-base-v2 produces 768-dimensional embeddings
        let dimensions = 768;

        Ok(Self {
            session,
            tokenizer,
            dimensions,
        })
    }

    pub async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        // Run inference on a blocking thread since ONNX runtime is synchronous
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

            let input_ids_array =
                ndarray::Array2::from_shape_vec((1, seq_len), input_ids)
                    .map_err(|e| Error::Embedding(format!("Array shape error: {e}")))?;
            let attention_mask_array =
                ndarray::Array2::from_shape_vec((1, seq_len), attention_mask)
                    .map_err(|e| Error::Embedding(format!("Array shape error: {e}")))?;
            let token_type_ids_array =
                ndarray::Array2::from_shape_vec((1, seq_len), token_type_ids)
                    .map_err(|e| Error::Embedding(format!("Array shape error: {e}")))?;

            let outputs = self
                .session
                .run(ort::inputs![
                    "input_ids" => input_ids_array,
                    "attention_mask" => attention_mask_array,
                    "token_type_ids" => token_type_ids_array,
                ].map_err(|e| Error::Embedding(format!("Input error: {e}")))?)
                .map_err(|e| Error::Embedding(format!("Inference failed: {e}")))?;

            // Extract the [CLS] token embedding (first token of last_hidden_state)
            let output_tensor = outputs[0]
                .try_extract_tensor::<f32>()
                .map_err(|e| Error::Embedding(format!("Output extraction failed: {e}")))?;

            let embedding: Vec<f32> = output_tensor
                .slice(ndarray::s![0, 0, ..])
                .to_vec();

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
