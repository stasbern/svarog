use std::time::Duration;

use rig::embeddings::{self, EmbeddingError};
use rig::http_client::{self, HttpClientExt};
use rig::providers::openai;
use serde::Deserialize;
use serde_json::json;

// Gemini's OpenAI-compatible endpoint rejects batches larger than 100 inputs.
pub const DEFAULT_BATCH_SIZE: usize = 100;
const MAX_RETRIES: u32 = 5;
const DEFAULT_RETRY_DELAY: Duration = Duration::from_secs(30);
const MAX_RETRY_DELAY: Duration = Duration::from_secs(120);

/// OpenAI-compatible `/embeddings` client tolerant of missing `usage`, batch caps and rate limits.
#[derive(Clone)]
pub struct CompatEmbeddingModel {
    client: openai::Client,
    model: String,
    batch_size: usize,
}

#[derive(Deserialize)]
struct EmbeddingResponse {
    data: Vec<EmbeddingData>,
}

#[derive(Deserialize)]
struct EmbeddingData {
    embedding: Vec<f64>,
    #[serde(default)]
    index: usize,
}

impl CompatEmbeddingModel {
    pub fn new(client: openai::Client, model: impl Into<String>, batch_size: usize) -> Self {
        Self {
            client,
            model: model.into(),
            batch_size: batch_size.max(1),
        }
    }

    async fn embed_batch(&self, batch: &[String]) -> Result<Vec<Vec<f64>>, EmbeddingError> {
        let body = serde_json::to_vec(&json!({ "model": self.model, "input": batch }))?;

        let mut attempt = 0;

        loop {
            let request = self
                .client
                .post("/embeddings")?
                .body(body.clone())
                .map_err(|error| EmbeddingError::HttpError(error.into()))?;

            let response = self.client.send(request).await?;
            let status = response.status();

            if status.is_success() {
                let body: Vec<u8> = response.into_body().await?;
                let mut parsed: EmbeddingResponse = serde_json::from_slice(&body)?;

                if parsed.data.len() != batch.len() {
                    return Err(EmbeddingError::ResponseError(format!(
                        "expected {} embeddings, received {}",
                        batch.len(),
                        parsed.data.len(),
                    )));
                }

                parsed.data.sort_by_key(|data| data.index);

                return Ok(parsed.data.into_iter().map(|data| data.embedding).collect());
            }

            let error_body = http_client::text(response).await?;
            let delay = retry_delay(&error_body).unwrap_or(DEFAULT_RETRY_DELAY);
            let transient = matches!(status.as_u16(), 429 | 503);

            if !transient || attempt >= MAX_RETRIES || delay > MAX_RETRY_DELAY {
                return Err(EmbeddingError::ProviderError(format!(
                    "{status}: {error_body}"
                )));
            }

            attempt += 1;
            tokio::time::sleep(delay).await;
        }
    }
}

impl embeddings::EmbeddingModel for CompatEmbeddingModel {
    // A single builder call per document keeps requests sequential; see `embed_batch`.
    const MAX_DOCUMENTS: usize = 1024;

    type Client = openai::Client;

    fn make(client: &Self::Client, model: impl Into<String>, _dims: Option<usize>) -> Self {
        Self::new(client.clone(), model, DEFAULT_BATCH_SIZE)
    }

    fn ndims(&self) -> usize {
        0
    }

    async fn embed_texts(
        &self,
        texts: impl IntoIterator<Item = String>,
    ) -> Result<Vec<embeddings::Embedding>, EmbeddingError> {
        let texts: Vec<String> = texts.into_iter().collect();
        let mut results = Vec::with_capacity(texts.len());

        for batch in texts.chunks(self.batch_size) {
            let vectors = self.embed_batch(batch).await?;

            results.extend(
                batch
                    .iter()
                    .zip(vectors)
                    .map(|(document, vec)| embeddings::Embedding {
                        document: document.clone(),
                        vec,
                    }),
            );
        }

        Ok(results)
    }
}

/// Parses Google's "Please retry in 50.58s." hint from a rate-limit error.
fn retry_delay(error_body: &str) -> Option<Duration> {
    let after = error_body.split("retry in ").nth(1)?;
    let seconds: f64 = after.split('s').next()?.trim().parse().ok()?;

    Some(Duration::from_secs_f64(seconds.ceil() + 1.0))
}

#[cfg(test)]
mod tests {
    use super::retry_delay;
    use std::time::Duration;

    #[test]
    fn parses_google_retry_hint() {
        let body = "Quota exceeded. Please retry in 50.588247541s.";

        assert_eq!(retry_delay(body), Some(Duration::from_secs(52)));
        assert_eq!(retry_delay("Service unavailable"), None);
    }
}
