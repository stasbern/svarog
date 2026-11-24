use std::sync::Arc;

use color_eyre::Result;
use rig::client::CompletionClient;
use rig::providers::openai::{CompletionModel, CompletionsClient};
use tokio::sync::mpsc;

use super::chat_service::ChatService;
use super::document::DocumentDescriptor;
use super::ingestion_service::IngestionService;
use super::knowledge::KnowledgeBase;
use super::knowledge_source::Namespace;
use super::request_handler::spawn_request_handler;
use crate::events::{Request, Response};

/// Chat Completions client for any OpenAI-compatible endpoint
/// (Gemini, llama.cpp `llama-server`, Ollama `/v1`, ...).
pub struct LlmClient {
    client: CompletionsClient,
    model: String,
    preamble: String,
    temperature: f64,
    additional_params: serde_json::Value,
}

impl LlmClient {
    pub fn new(
        base_url: &str,
        api_key: &str,
        model: &str,
        preamble: &str,
        temperature: f64,
        additional_params: serde_json::Value,
    ) -> Result<Self> {
        let client = CompletionsClient::builder()
            .api_key(api_key)
            .base_url(base_url)
            .build()?;

        Ok(Self {
            client,
            model: model.to_string(),
            preamble: preamble.to_string(),
            temperature,
            additional_params,
        })
    }

    pub fn handle_requests(
        &self,
        knowledge: Arc<KnowledgeBase>,
        request_rx: mpsc::Receiver<Request>,
        response_tx: mpsc::Sender<Response>,
    ) {
        let chat_service = ChatService::new(
            self.agent_with(&self.preamble, self.temperature),
            knowledge.clone(),
            response_tx.clone(),
        );

        let ingestion_service = IngestionService::new(
            self.agent_with(Namespace::classifier_prompt(), 0.0),
            self.agent_with(DocumentDescriptor::extractor_prompt(), 0.0),
            self.model.clone(),
            knowledge,
            response_tx,
        );

        spawn_request_handler(request_rx, chat_service, ingestion_service);
    }

    fn agent_with(&self, preamble: &str, temperature: f64) -> rig::agent::Agent<CompletionModel> {
        self.client
            .agent(&self.model)
            .preamble(preamble)
            .temperature(temperature)
            .additional_params(self.additional_params.clone())
            .build()
    }
}

/// Separates reasoning that some endpoints (e.g. Gemma on Gemini) inline into the answer.
/// Returns `(reasoning, answer)`.
pub fn split_reasoning(text: &str) -> (String, String) {
    let mut answer = text.to_string();
    let mut reasoning = Vec::new();

    for (open, close) in [("<thought>", "</thought>"), ("<think>", "</think>")] {
        while let Some(start) = answer.find(open) {
            let (inner_end, block_end) = match answer[start..].find(close) {
                Some(offset) => (start + offset, start + offset + close.len()),
                None => (answer.len(), answer.len()),
            };

            reasoning.push(answer[start + open.len()..inner_end].trim().to_string());
            answer.replace_range(start..block_end, "");
        }
    }

    (reasoning.join("\n\n"), answer.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::split_reasoning;

    #[test]
    fn splits_inline_reasoning_blocks() {
        assert_eq!(
            split_reasoning("<thought>project? personal?</thought>factual"),
            ("project? personal?".into(), "factual".into())
        );
        assert_eq!(
            split_reasoning("<think>unterminated"),
            ("unterminated".into(), String::new())
        );
        assert_eq!(
            split_reasoning("plain answer"),
            (String::new(), "plain answer".into())
        );
    }
}
