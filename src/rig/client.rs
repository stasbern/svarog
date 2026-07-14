use color_eyre::Result;

use rig::client::{CompletionClient, ProviderClient};
use rig::completion::{Chat, Message, Prompt};
use rig::providers::ollama::{Client, CompletionModel};

use std::sync::Arc;
use tokio::sync::mpsc;

use super::knowledge::KnowledgeBase;
use super::knowledge_source::Namespace;
use super::document::{
    DocumentDescriptor,
    KnowledgeDocument,
};
use super::ingestion::{
    INGESTION_VERSION,
    extract_path,
};
use crate::events::*;

pub struct OllamaClient {
    client: Client,
    model: String,
    preamble: String,
    temperature: f64,
    additional_params: serde_json::Value,
}

impl OllamaClient {
    pub fn new(model: &str, preamble: &str, temperature: f64, additional_params: serde_json::Value) -> Self {
        Self {
            client: Client::from_env().expect("Failed to connect to Ollama — is OLLAMA_HOST set?"),
            model: String::from(model),
            preamble: String::from(preamble),
            temperature,
            additional_params,
        }
    }

    fn agent_with(&self, preamble: &str, temperature: f64) -> rig::agent::Agent<CompletionModel> {
        self.client
            .agent(&self.model)
            .preamble(preamble)
            .temperature(temperature)
            .additional_params(self.additional_params.clone())
            .build()
    }

    pub fn inner(&self) -> &Client {
        &self.client
    }

    pub async fn classify_text(&self, text: &str) -> Namespace {
        let classifier = self.agent_with(Namespace::classifier_prompt(), 0.0);

        let preview: String = text.chars().take(800).collect();
        match classifier.prompt(&preview).await {
            Ok(response) => Namespace::parse(&response),
            Err(_) => Namespace::Factual,
        }
    }

    pub fn handle_requests(
        &self,
        knowledge: Arc<KnowledgeBase>,
        mut prompt_rx: mpsc::Receiver<Request>,
        response_tx: mpsc::Sender<Response>,
    ) {
        let agent = self.agent_with(&self.preamble, self.temperature);
        let classifier = self.agent_with(Namespace::classifier_prompt(), 0.0);
        let descriptor_agent = self.agent_with(DocumentDescriptor::extractor_prompt(), 0.0);

        let descriptor_model_name = self.model.clone();

        tokio::spawn(async move {
            let mut chat_history: Vec<Message> = vec![];

            while let Some(req) = prompt_rx.recv().await {
                match req {
                    Request::Prompt(prompt) => {
                        // First find documents whose identities match the question.
                        let document_matches = knowledge
                            .search_documents(&prompt, 3)
                            .await
                            .unwrap_or_default();

                        if !document_matches.is_empty() {
                            let info = document_matches
                                .iter()
                                .map(|result| {
                                    (
                                        result.score,
                                        format!(
                                            "[document] {} — {}",
                                            result.document.descriptor.title,
                                            result.document.descriptor.summary
                                        ),
                                    )
                                })
                                .collect();

                            let _ = response_tx
                                .send(Response::ContextFound(info))
                                .await;
                        }

                        // Then retrieve precise passage evidence.
                        let relevant = match knowledge
                            .search_multi(&prompt, Namespace::searchable(), 6)
                            .await
                        {
                            Ok(results) => {
                                let filtered: Vec<_> = results
                                    .into_iter()
                                    .filter(|result| result.score >= 0.1)
                                    .collect();

                                if !filtered.is_empty() {
                                    let info = filtered
                                        .iter()
                                        .map(|result| {
                                            let preview = result
                                                .content
                                                .chars()
                                                .take(120)
                                                .collect::<String>();

                                            (
                                                result.score,
                                                format!("[{}] {}", result.namespace, preview),
                                            )
                                        })
                                        .collect();

                                    let _ = response_tx
                                        .send(Response::ContextFound(info))
                                        .await;
                                }

                                filtered
                            }

                            Err(error) => {
                                let _ = response_tx
                                    .send(Response::ContextFound(vec![(
                                        0.0,
                                        format!("KB error: {error}"),
                                    )]))
                                    .await;

                                vec![]
                            }
                        };

                        let catalog_context = document_matches
                            .iter()
                            .map(|result| result.document.catalog_context())
                            .collect::<Vec<_>>()
                            .join("\n\n--- DOCUMENT ---\n\n");

                        let evidence_context = relevant
                            .iter()
                            .map(|result| result.content.as_str())
                            .collect::<Vec<_>>()
                            .join("\n\n--- PASSAGE ---\n\n");

                        let final_prompt = if catalog_context.is_empty()
                            && evidence_context.is_empty()
                        {
                            prompt
                        } else {
                            format!(
                                r#"
                    Knowledge catalog:
                    {catalog_context}

                    Retrieved source passages:
                    {evidence_context}

                    Instructions:
                    - The catalog describes which documents and subjects are available.
                    - The passages are source evidence from those documents.
                    - For questions about what knowledge is available, answer primarily from the catalog.
                    - For technical claims, rely on the source passages.
                    - Do not claim that catalog metadata proves technical details not present in the passages.
                    - If the available context is insufficient, say so.

                    User: {prompt}
                    "#
                            )
                        };

                        match agent.chat(&final_prompt, &mut chat_history).await {
                            Ok(text) => {
                                let _ = response_tx
                                    .send(Response::CompleteResponse(text))
                                    .await;
                            }

                            Err(error) => {
                                let _ = response_tx
                                    .send(Response::Error(format!("{error:?}")))
                                    .await;
                            }
                        }
                    }
                    Request::Ingest => {
                        let dir = std::path::PathBuf::from("./input");

                        let stored_hashes = knowledge
                            .get_all_hashes()
                            .await
                            .unwrap_or_default();

                        match tokio::fs::read_dir(&dir).await {
                            Ok(mut entries) => {
                                while let Ok(Some(entry)) = entries.next_entry().await {
                                    let entry_path = entry.path();
                                    let fname = entry.file_name().to_string_lossy().to_string();

                                    let extracted = match extract_path(&entry_path).await {
                                        Ok(Some(document)) => document,

                                        Ok(None) => {
                                            let _ = response_tx
                                                .send(Response::Status(format!(
                                                    "{fname}: unsupported file type, skipping"
                                                )))
                                                .await;
                                            continue;
                                        }

                                        Err(error) => {
                                            let _ = response_tx
                                                .send(Response::Status(format!(
                                                    "{fname}: extraction failed: {error}"
                                                )))
                                                .await;
                                            continue;
                                        }
                                    };

                                    let path = extracted.source_path.clone();
                                    let pipeline_hash = extracted.pipeline_hash();

                                    // Compare the original file bytes, not extracted text.
                                    if let Some((_, stored_hash)) = stored_hashes.get(&path) {
                                        if *stored_hash == pipeline_hash {
                                            let _ = response_tx
                                                .send(Response::Status(format!(
                                                    "{fname} unchanged, skipping"
                                                )))
                                                .await;
                                            continue;
                                        }
                                    }

                                    let preview = extracted.preview(1_500);

                                    let ns = match classifier.prompt(&preview).await {
                                        Ok(response) => Namespace::parse(&response),
                                        Err(_) => Namespace::Factual,
                                    };

                                    let descriptor_input = extracted.descriptor_source(6, 16_000);

                                    let descriptor = match descriptor_agent
                                        .prompt(&descriptor_input)
                                        .await
                                    {
                                        Ok(response) => {
                                            match DocumentDescriptor::from_model_output(
                                                &response,
                                                &extracted.title,
                                            ) {
                                                Ok(descriptor) => descriptor,

                                                Err(error) => {
                                                    let _ = response_tx
                                                        .send(Response::Status(format!(
                                                            "{fname}: descriptor JSON was invalid ({error}); using fallback metadata"
                                                        )))
                                                        .await;

                                                    DocumentDescriptor::fallback(&extracted.title)
                                                }
                                            }
                                        }

                                        Err(error) => {
                                            let _ = response_tx
                                                .send(Response::Status(format!(
                                                    "{fname}: descriptor extraction failed ({error}); using fallback metadata"
                                                )))
                                                .await;

                                            DocumentDescriptor::fallback(&extracted.title)
                                        }
                                    };

                                    let knowledge_document = KnowledgeDocument::from_extracted(
                                        &extracted,
                                        ns,
                                        descriptor,
                                        descriptor_model_name.clone(),
                                        INGESTION_VERSION,
                                    );

                                    let _ = response_tx
                                        .send(Response::Status(format!(
                                            "{} → {} pages → {} → {}",
                                            knowledge_document.descriptor.title,
                                            extracted.page_count(),
                                            ns,
                                            knowledge_document
                                                .descriptor
                                                .document_type
                                                .as_deref()
                                                .unwrap_or("document")
                                        )))
                                        .await;

                                    // If classification changed, remove records from the old table.
                                    if let Some((old_ns_str, _)) = stored_hashes.get(&path) {
                                        let old_ns = Namespace::parse(old_ns_str);

                                        if old_ns != ns {
                                            if let Err(error) = knowledge
                                                .delete_chunks_for_file(&path, old_ns)
                                                .await
                                            {
                                                let _ = response_tx
                                                    .send(Response::Status(format!(
                                                        "{fname}: failed to remove old chunks: {error}"
                                                    )))
                                                    .await;
                                                continue;
                                            }
                                        }
                                    }

                                    let canonical_text = extracted.canonical_text();

                                    if canonical_text.trim().is_empty() {
                                        let _ = response_tx
                                            .send(Response::Status(format!(
                                                "{fname}: no usable text after extraction"
                                            )))
                                            .await;
                                        continue;
                                    }

                                    if let Err(error) = knowledge
                                        .upsert_document(&knowledge_document)
                                        .await
                                    {
                                        let _ = response_tx
                                            .send(Response::Status(format!(
                                                "{fname}: failed to store document catalog metadata: {error}"
                                            )))
                                            .await;

                                        continue;
                                    }

                                    if let Err(error) = knowledge
                                        .ingest_file_with_hash(
                                            &path,
                                            &canonical_text,
                                            ns,
                                            &pipeline_hash,
                                        )
                                        .await
                                    {
                                        let _ = response_tx
                                            .send(Response::Status(format!(
                                                "{fname}: ingestion failed: {error}"
                                            )))
                                            .await;
                                        continue;
                                    }

                                    let _ = response_tx
                                        .send(Response::Status(format!(
                                            "{fname}: stored {} pages",
                                            extracted.page_count()
                                        )))
                                        .await;
                                }

                                let _ = response_tx
                                    .send(Response::Status("Ingestion complete".into()))
                                    .await;
                            }

                            Err(error) => {
                                let _ = response_tx
                                    .send(Response::Status(format!(
                                        "Failed to read input directory: {error}"
                                    )))
                                    .await;
                            }
                        }
                    }
                }
            }
        });
    }
}
