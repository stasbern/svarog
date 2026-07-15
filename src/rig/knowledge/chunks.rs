// src/rig/knowledge/chunks.rs

use std::path::Path;

use color_eyre::Result;
use rig::providers::ollama;
use rig::surrealdb::{SurrealDistanceFunction, SurrealVectorStore};
use rig::vector_store::{InsertDocuments, VectorSearchRequest, VectorStoreIndexDyn};
use rig::{Embed, embeddings::EmbeddingsBuilder};
use serde::{Deserialize, Serialize};
use surrealdb::engine::local::Db;
use tokio::fs;

use super::KnowledgeBase;
use crate::rig::chunking::chunk_input;
use crate::rig::knowledge_source::Namespace;

#[derive(Embed, Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
struct Chunk {
    chunk_index: usize,
    file_path: String,

    #[embed]
    content: String,
}

pub struct SearchResult {
    pub score: f64,
    pub id: String,
    pub content: String,
    pub namespace: Namespace,
}

impl KnowledgeBase {
    /// Ingests generated text whose hash is derived from the text.
    pub async fn ingest_file(&self, path: &str, text: &str, namespace: Namespace) -> Result<()> {
        let content_hash = Self::hash_content(text);

        self.ingest_file_with_hash(path, text, namespace, &content_hash)
            .await
    }

    /// Ingests extracted text while storing a caller-provided source
    /// or pipeline hash.
    pub async fn ingest_file_with_hash(
        &self,
        path: &str,
        text: &str,
        namespace: Namespace,
        source_hash: &str,
    ) -> Result<()> {
        self.delete_chunks_for_file(path, namespace).await?;

        let chunks = create_chunks(path, text);

        if !chunks.is_empty() {
            let mut builder = EmbeddingsBuilder::new(self.embedding_model.clone());

            for chunk in chunks {
                builder = builder.document(chunk)?;
            }

            let embeddings = builder.build().await?;

            self.vector_store_for(namespace)
                .insert_documents(embeddings)
                .await?;
        }

        self.store_hash(path, source_hash, namespace).await?;

        Ok(())
    }

    /// Legacy text-directory ingestion helper.
    ///
    /// The newer ingestion service should be preferred for PDF and
    /// descriptor-aware ingestion.
    pub async fn ingest_directory(&self, directory: &Path, namespace: Namespace) -> Result<()> {
        let stored = self.get_all_hashes().await.unwrap_or_default();

        let mut entries = match fs::read_dir(directory).await {
            Ok(entries) => entries,
            Err(_) => return Ok(()),
        };

        while let Some(entry) = entries.next_entry().await? {
            let text = match fs::read_to_string(entry.path()).await {
                Ok(text) => text,
                Err(_) => continue,
            };

            let path = entry.path().to_string_lossy().replace('\\', "/");

            let hash = Self::hash_content(&text);

            if stored
                .get(&path)
                .is_some_and(|(_, stored_hash)| stored_hash == &hash)
            {
                continue;
            }

            self.ingest_file(&path, &text, namespace).await?;
        }

        Ok(())
    }

    pub fn vector_store_for(
        &self,
        namespace: Namespace,
    ) -> SurrealVectorStore<Db, ollama::EmbeddingModel> {
        SurrealVectorStore::new(
            self.embedding_model.clone(),
            self.db.clone(),
            Some(namespace.table_name().to_string()),
            SurrealDistanceFunction::Cosine,
        )
    }

    pub fn vector_store(&self) -> SurrealVectorStore<Db, ollama::EmbeddingModel> {
        self.vector_store_for(Namespace::Factual)
    }

    pub async fn search_namespace(
        &self,
        query: &str,
        namespace: Namespace,
        top_k: u64,
    ) -> Result<Vec<SearchResult>> {
        let store = self.vector_store_for(namespace);

        let request = VectorSearchRequest::builder()
            .query(format!("query: {query}"))
            .samples(top_k)
            .build();

        let results = store
            .top_n(request)
            .await
            .map_err(|error| color_eyre::eyre::eyre!("{error}"))?;

        Ok(results
            .into_iter()
            .map(|(score, id, document)| SearchResult {
                score,
                id,
                content: content_from_document(&document),
                namespace,
            })
            .collect())
    }

    pub async fn search_multi(
        &self,
        query: &str,
        namespaces: &[Namespace],
        top_k: u64,
    ) -> Result<Vec<SearchResult>> {
        let mut all_results = Vec::new();

        for &namespace in namespaces {
            // A namespace table may not exist until it receives its
            // first document, so an absent table is not fatal.
            if let Ok(mut results) = self.search_namespace(query, namespace, top_k).await {
                all_results.append(&mut results);
            }
        }

        all_results.sort_by(|left, right| {
            right
                .score
                .partial_cmp(&left.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        all_results.truncate(top_k as usize);

        Ok(all_results)
    }

    pub async fn delete_chunks_for_file(
        &self,
        file_path: &str,
        namespace: Namespace,
    ) -> Result<()> {
        let needle = format!("\"file_path\":\"{file_path}\"");

        let query = format!(
            r#"
            DELETE FROM {}
            WHERE string::contains(
                document,
                $needle
            );
            "#,
            namespace.table_name(),
        );

        self.db.query(&query).bind(("needle", needle)).await?;

        Ok(())
    }
}

fn create_chunks(path: &str, text: &str) -> Vec<Chunk> {
    chunk_input(text, 400, 80)
        .into_iter()
        .enumerate()
        .map(|(chunk_index, content)| Chunk {
            chunk_index,
            file_path: path.to_string(),
            content,
        })
        .collect()
}

fn content_from_document(document: &serde_json::Value) -> String {
    if let Some(items) = document.as_array() {
        return items
            .iter()
            .filter_map(|item| item.get("content").and_then(|value| value.as_str()))
            .collect::<Vec<_>>()
            .join("\n");
    }

    document
        .get("content")
        .and_then(|value| value.as_str())
        .or_else(|| document.as_str())
        .unwrap_or_default()
        .to_string()
}
