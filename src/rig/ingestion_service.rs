// src/rig/ingestion_service.rs

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use rig::completion::Prompt;
use rig::providers::ollama::CompletionModel;
use tokio::sync::mpsc;

use super::document::{DocumentDescriptor, KnowledgeDocument};
use super::ingestion::{ExtractedDocument, INGESTION_VERSION, extract_path};
use super::knowledge::KnowledgeBase;
use super::knowledge_source::Namespace;
use crate::events::Response;

type StoredHashes = HashMap<String, (String, String)>;

pub struct IngestionService {
    classifier: rig::agent::Agent<CompletionModel>,
    descriptor_agent: rig::agent::Agent<CompletionModel>,
    descriptor_model_name: String,

    knowledge: Arc<KnowledgeBase>,
    response_tx: mpsc::Sender<Response>,
}

impl IngestionService {
    pub fn new(
        classifier: rig::agent::Agent<CompletionModel>,
        descriptor_agent: rig::agent::Agent<CompletionModel>,
        descriptor_model_name: String,
        knowledge: Arc<KnowledgeBase>,
        response_tx: mpsc::Sender<Response>,
    ) -> Self {
        Self {
            classifier,
            descriptor_agent,
            descriptor_model_name,
            knowledge,
            response_tx,
        }
    }

    pub async fn ingest_default_directory(&self) {
        self.ingest_directory(Path::new("./input")).await;
    }

    pub async fn ingest_directory(&self, directory: &Path) {
        let stored_hashes = self.knowledge.get_all_hashes().await.unwrap_or_default();

        let mut entries = match tokio::fs::read_dir(directory).await {
            Ok(entries) => entries,

            Err(error) => {
                self.status(format!("Failed to read input directory: {error}"))
                    .await;

                return;
            }
        };

        loop {
            let entry = match entries.next_entry().await {
                Ok(Some(entry)) => entry,
                Ok(None) => break,

                Err(error) => {
                    self.status(format!("Failed while reading input directory: {error}"))
                        .await;

                    break;
                }
            };

            let file_name = entry.file_name().to_string_lossy().to_string();

            self.ingest_path(&entry.path(), &file_name, &stored_hashes)
                .await;
        }

        self.status("Ingestion complete").await;
    }

    async fn ingest_path(&self, path: &Path, file_name: &str, stored_hashes: &StoredHashes) {
        let extracted = match extract_path(path).await {
            Ok(Some(document)) => document,

            Ok(None) => {
                self.status(format!("{file_name}: unsupported file type, skipping"))
                    .await;

                return;
            }

            Err(error) => {
                self.status(format!("{file_name}: extraction failed: {error}"))
                    .await;

                return;
            }
        };

        let source_path = extracted.source_path.clone();
        let pipeline_hash = extracted.pipeline_hash();

        if source_is_unchanged(&source_path, &pipeline_hash, stored_hashes) {
            self.status(format!("{file_name} unchanged, skipping"))
                .await;

            return;
        }

        let namespace = self.classify_document(&extracted).await;

        let descriptor = self.describe_document(&extracted, file_name).await;

        let knowledge_document = KnowledgeDocument::from_extracted(
            &extracted,
            namespace,
            descriptor,
            self.descriptor_model_name.clone(),
            INGESTION_VERSION,
        );

        self.report_document_identity(&knowledge_document).await;

        if !self
            .remove_old_namespace_chunks(&source_path, namespace, stored_hashes, file_name)
            .await
        {
            return;
        }

        let canonical_text = extracted.canonical_text();

        if canonical_text.trim().is_empty() {
            self.status(format!("{file_name}: no usable text after extraction"))
                .await;

            return;
        }

        if let Err(error) = self.knowledge.upsert_document(&knowledge_document).await {
            self.status(format!("{file_name}: document catalog failed: {error}"))
                .await;

            return;
        }

        if let Err(error) = self
            .knowledge
            .ingest_file_with_hash(&source_path, &canonical_text, namespace, &pipeline_hash)
            .await
        {
            self.status(format!("{file_name}: ingestion failed: {error}"))
                .await;

            return;
        }

        self.status(format!(
            "{file_name}: stored {} pages",
            extracted.page_count(),
        ))
        .await;
    }

    async fn classify_document(&self, extracted: &ExtractedDocument) -> Namespace {
        let preview = extracted.preview(1_500);

        match self.classifier.prompt(&preview).await {
            Ok(response) => Namespace::parse(&response),
            Err(_) => Namespace::Factual,
        }
    }

    async fn describe_document(
        &self,
        extracted: &ExtractedDocument,
        file_name: &str,
    ) -> DocumentDescriptor {
        let input = extracted.descriptor_source(6, 16_000);

        let response = match self.descriptor_agent.prompt(&input).await {
            Ok(response) => response,

            Err(error) => {
                self.status(format!(
                    "{file_name}: descriptor extraction failed \
                     ({error}); using fallback metadata"
                ))
                .await;

                return DocumentDescriptor::fallback(&extracted.title);
            }
        };

        match DocumentDescriptor::from_model_output(&response, &extracted.title) {
            Ok(descriptor) => descriptor,

            Err(error) => {
                self.status(format!(
                    "{file_name}: descriptor JSON was invalid \
                     ({error}); using fallback metadata"
                ))
                .await;

                DocumentDescriptor::fallback(&extracted.title)
            }
        }
    }

    async fn remove_old_namespace_chunks(
        &self,
        source_path: &str,
        namespace: Namespace,
        stored_hashes: &StoredHashes,
        file_name: &str,
    ) -> bool {
        let Some((old_namespace, _)) = stored_hashes.get(source_path) else {
            return true;
        };

        let old_namespace = Namespace::parse(old_namespace);

        if old_namespace == namespace {
            return true;
        }

        match self
            .knowledge
            .delete_chunks_for_file(source_path, old_namespace)
            .await
        {
            Ok(()) => true,

            Err(error) => {
                self.status(format!(
                    "{file_name}: failed to remove old chunks: \
                     {error}"
                ))
                .await;

                false
            }
        }
    }

    async fn report_document_identity(&self, document: &KnowledgeDocument) {
        self.status(format!(
            "{} → {} pages → {} → {}",
            document.descriptor.title,
            document.page_count,
            document.namespace,
            document
                .descriptor
                .document_type
                .as_deref()
                .unwrap_or("document"),
        ))
        .await;
    }

    async fn status(&self, message: impl Into<String>) {
        let _ = self
            .response_tx
            .send(Response::Status(message.into()))
            .await;
    }
}

fn source_is_unchanged(
    source_path: &str,
    pipeline_hash: &str,
    stored_hashes: &StoredHashes,
) -> bool {
    stored_hashes
        .get(source_path)
        .is_some_and(|(_, stored_hash)| stored_hash == pipeline_hash)
}

#[cfg(test)]
mod tests {
    use super::source_is_unchanged;
    use std::collections::HashMap;

    #[test]
    fn identifies_matching_pipeline_hash() {
        let stored = HashMap::from([(
            "manual.pdf".to_string(),
            ("factual".to_string(), "pipeline:hash".to_string()),
        )]);

        assert!(source_is_unchanged("manual.pdf", "pipeline:hash", &stored,));

        assert!(!source_is_unchanged("manual.pdf", "different", &stored,));
    }
}
