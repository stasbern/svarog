use std::sync::Arc;

use rig::client::{CompletionClient, ProviderClient};
use rig::providers::ollama::{Client, CompletionModel};
use tokio::sync::mpsc;

use super::chat_service::ChatService;
use super::document::DocumentDescriptor;
use super::ingestion_service::IngestionService;
use super::knowledge::KnowledgeBase;
use super::knowledge_source::Namespace;
use super::request_handler::spawn_request_handler;
use crate::events::{Request, Response};

pub struct OllamaClient {
    client: Client,
    model: String,
    preamble: String,
    temperature: f64,
    additional_params: serde_json::Value,
}

impl OllamaClient {
    pub fn new(
        model: &str,
        preamble: &str,
        temperature: f64,
        additional_params: serde_json::Value,
    ) -> Self {
        Self {
            client: Client::from_env().expect("Failed to connect to Ollama — is OLLAMA_HOST set?"),
            model: model.to_string(),
            preamble: preamble.to_string(),
            temperature,
            additional_params,
        }
    }

    pub fn inner(&self) -> &Client {
        &self.client
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
