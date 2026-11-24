use std::sync::Arc;

use color_eyre::{Result, eyre::eyre};
use rig::agent::{Agent, AgentBuilder};
use rig::providers::openai::{OpenAI, OpenAIConfig, wire};
use tokio::sync::mpsc;

use super::chat_service::ChatService;
use super::document::DocumentDescriptor;
use super::ingestion_service::IngestionService;
use super::knowledge::KnowledgeBase;
use super::knowledge_source::Namespace;
use super::request_handler::spawn_request_handler;
use crate::events::{Request, Response};

/// Connects to an OpenAI-shaped server using a Rig dialect name (`openai`, `llamacpp`, ...).
pub fn connect(dialect: &str, base_url: &str, api_key: &str) -> Result<OpenAI> {
    let mut dialect = *wire::by_name(dialect).ok_or_else(|| eyre!("unknown dialect `{dialect}`"))?;

    // Gemini omits embedding `usage`, and nothing here reads it.
    dialect.quirks.embedding.requires_usage = false;

    Ok(OpenAIConfig::with_key(&dialect, api_key)
        .with_base_url(base_url)
        .client())
}

/// Builds the chat, classification and metadata agents on one Chat Completions model.
pub struct LlmClient {
    provider: OpenAI,
    model: String,
    preamble: String,
    temperature: f64,
    additional_params: serde_json::Value,
}

impl LlmClient {
    pub fn new(
        provider: OpenAI,
        model: &str,
        preamble: &str,
        temperature: f64,
        additional_params: serde_json::Value,
    ) -> Self {
        Self {
            provider,
            model: model.to_string(),
            preamble: preamble.to_string(),
            temperature,
            additional_params,
        }
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

    fn agent_with(&self, preamble: &str, temperature: f64) -> Agent {
        AgentBuilder::new(self.provider.chat(&self.model))
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

/// Shortens provider errors to the `error.message` of an embedded JSON body, if any.
pub fn readable_error(error: &impl std::fmt::Display) -> String {
    let text = error.to_string();

    let provider_message = text
        .find(['[', '{'])
        .and_then(|start| {
            serde_json::Deserializer::from_str(&text[start..])
                .into_iter::<serde_json::Value>()
                .next()?
                .ok()
        })
        .and_then(|json| {
            let error = json.get(0).unwrap_or(&json).get("error")?;
            let message = error.get("message")?.as_str()?;

            Some(match error.get("code") {
                Some(code) => format!("{code}: {message}"),
                None => message.to_string(),
            })
        });

    provider_message.unwrap_or(text)
}

#[cfg(test)]
mod tests {
    use super::{readable_error, split_reasoning};

    #[test]
    fn extracts_provider_error_message() {
        let raw = r#"HttpError: status 503: [{"error": {"code": 503, "message": "High demand."}}]"#;

        assert_eq!(readable_error(&raw), "503: High demand.");
        assert_eq!(readable_error(&"connection refused"), "connection refused");
    }

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
