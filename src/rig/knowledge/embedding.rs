use std::time::{Duration, SystemTime};

use rig::DynModel;
use rig::ProviderError;
use rig::embeddings::EmbeddingResponse;
use rig::operation::Embedding;
use rig::providers::openai::OpenAI;

// Gemini's OpenAI-compatible endpoint rejects batches larger than 100 inputs.
pub const DEFAULT_BATCH_SIZE: usize = 100;
const MAX_RETRIES: u32 = 5;
const DEFAULT_RETRY_DELAY: Duration = Duration::from_secs(30);
const MAX_RETRY_DELAY: Duration = Duration::from_secs(120);

/// Embeds texts in sequential batches, waiting out rate limits instead of failing.
pub struct Embedder {
    model: DynModel<Embedding>,
    batch_size: usize,
}

impl Embedder {
    pub fn new(provider: &OpenAI, model: &str, batch_size: usize) -> Self {
        Self {
            model: provider.embedding(model, None).into(),
            batch_size: batch_size.max(1),
        }
    }

    pub async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f64>>, ProviderError> {
        let mut vectors = Vec::with_capacity(texts.len());

        for batch in texts.chunks(self.batch_size) {
            let response = self.call_with_retry(batch.to_vec()).await?;
            vectors.extend(response.embeddings.into_iter().map(|embedding| embedding.vec));
        }

        Ok(vectors)
    }

    pub async fn embed_one(&self, text: &str) -> Result<Vec<f64>, ProviderError> {
        self.embed(&[text.to_string()])
            .await?
            .pop()
            .ok_or_else(|| ProviderError::Response("empty embedding response".into()))
    }

    async fn call_with_retry(&self, batch: Vec<String>) -> Result<EmbeddingResponse, ProviderError> {
        let mut attempt = 0;

        loop {
            let error = match self.model.call(batch.clone()).await {
                Ok(response) => return Ok(response),
                Err(error) => error,
            };

            let delay = error
                .report()
                .retry_after(SystemTime::now())
                .unwrap_or(DEFAULT_RETRY_DELAY);

            if !error.is_retryable() || attempt >= MAX_RETRIES || delay > MAX_RETRY_DELAY {
                return Err(error);
            }

            attempt += 1;
            tokio::time::sleep(delay).await;
        }
    }
}
