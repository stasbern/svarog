// src/rig/ingestion_service.rs

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use rig::agent::Agent;
use tokio::sync::mpsc;

use super::client::{readable_error, split_reasoning};
use super::document::{DocumentDescriptor, KnowledgeDocument};
use super::ingestion::{ExtractedDocument, INGESTION_VERSION, extract_path};
use super::knowledge::KnowledgeBase;
use super::knowledge_source::Namespace;
use crate::events::Response;

type StoredHashes = HashMap<String, (String, String)>;

pub struct IngestionService {
    classifier: Agent,
    descriptor_agent: Agent,
    descriptor_model_name: String,

    knowledge: Arc<KnowledgeBase>,
    response_tx: mpsc::Sender<Response>,
}

impl IngestionService {
    pub fn new(
        classifier: Agent,
        descriptor_agent: Agent,
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

        let has_usable_text = extracted
            .pages
            .iter()
            .any(|page| !page.text.trim().is_empty());

        if !has_usable_text {
            self.status(format!("{file_name}: no usable text after extraction"))
                .await;

            return;
        }

        if let Err(error) = self.knowledge.upsert_document(&knowledge_document).await {
            self.status(format!("{file_name}: document catalog failed: {error}"))
                .await;

            return;
        }

        let chunk_count = match self
            .knowledge
            .replace_document_chunks(&knowledge_document, &extracted.pages, &pipeline_hash)
            .await
        {
            Ok(chunk_count) => chunk_count,

            Err(error) => {
                self.status(format!("{file_name}: chunk ingestion failed: {error}"))
                    .await;

                return;
            }
        };

        self.status(format!(
            "{file_name}: stored {} pages in {} chunks",
            extracted.page_count(),
            chunk_count,
        ))
        .await;

        self.status(format!(
            "{file_name}: stored {} pages",
            extracted.page_count(),
        ))
        .await;
    }

    async fn classify_document(&self, extracted: &ExtractedDocument) -> Namespace {
        let preview = extracted.preview(1_500);

        match self.classifier.prompt(&preview).await {
            Ok(response) => Namespace::parse(&split_reasoning(&response.output()).1),
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
            Ok(response) => split_reasoning(&response.output()).1,

            Err(error) => {
                self.status(format!(
                    "{file_name}: descriptor extraction failed \
                     ({}); using fallback metadata",
                    readable_error(&error)
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

    #[tokio::test]
    #[ignore]
    async fn profile_ingestion() {
        use crate::rig::chunking::chunk_input;
        use crate::rig::client::{connect, readable_error, split_reasoning};
        use crate::rig::document::DocumentDescriptor;
        use crate::rig::ingestion::extract_path;
        use crate::rig::knowledge_source::Namespace;
        use rig::agent::AgentBuilder;
        use std::time::Instant;

        let mut first = None;
        for name in ["ADOBE_2019_10K.pdf", "AMAZON_2019_10K.pdf", "AMAZON_2020_10K.pdf"] {
            let started = Instant::now();
            let doc = extract_path(&std::path::Path::new("input").join(name)).await.unwrap().unwrap();
            let elapsed = started.elapsed();
            let chars: usize = doc.pages.iter().map(|p| p.text.len()).sum();
            let chunks: usize = doc.pages.iter().map(|p| chunk_input(&p.text, 1800, 250).len()).sum();
            println!("PROFILE {name}: extract {elapsed:.1?}, {} pages, {chars} chars, {chunks} chunks", doc.page_count());
            first.get_or_insert(doc);
        }

        let db = surrealdb::Surreal::new::<surrealdb::engine::local::RocksDb>("svarog_vectors.db").await.unwrap();
        db.use_ns("svarog_ns").use_db("svarog_db").await.unwrap();
        let started = Instant::now();
        let mut response = db
            .query("SELECT id, vector::similarity::cosine($e, embedding) AS score FROM knowledge_chunk ORDER BY score DESC LIMIT 24")
            .bind(("e", vec![0.01_f64; 3072]))
            .await
            .unwrap();
        let rows: Vec<serde_json::Value> = response.take(0).unwrap();
        println!("PROFILE vector scan: {:.1?} for top {} rows", started.elapsed(), rows.len());

        let provider = connect("openai", &std::env::var("LLM_BASE_URL").unwrap(), &std::env::var("LLM_API_KEY").unwrap()).unwrap();
        let doc = first.unwrap();
        for (label, preamble, input) in [
            ("classify", Namespace::classifier_prompt(), doc.preview(1_500)),
            ("describe", DocumentDescriptor::extractor_prompt(), doc.descriptor_source(6, 16_000)),
        ] {
            let agent = AgentBuilder::new(provider.chat("gemma-4-26b-a4b-it")).preamble(preamble).temperature(0.0).build();
            let started = Instant::now();
            match agent.prompt(&input).await {
                Ok(r) => {
                    let (reasoning, answer) = split_reasoning(&r.output());
                    println!("PROFILE {label}: {:.1?}, reasoning {} chars, answer {} chars", started.elapsed(), reasoning.len(), answer.len());
                }
                Err(e) => println!("PROFILE {label}: error after {:.1?}: {}", started.elapsed(), readable_error(&e)),
            }
        }
    }
}
