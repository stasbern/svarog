// src/rig/knowledge/documents.rs

use color_eyre::Result;
use rig::embeddings::EmbeddingModel;
use serde::Deserialize;
use surrealdb::types::SurrealValue;

use super::KnowledgeBase;
use crate::rig::document::{DocumentDescriptor, DocumentSearchResult, KnowledgeDocument};
use crate::rig::knowledge_source::Namespace;

/// Database projection used when loading catalog records.
///
/// This is deliberately kept in the persistence layer rather than
/// the document domain module.
#[derive(Debug, Deserialize, SurrealValue)]
struct KnowledgeDocumentRow {
    document_key: String,
    source_path: String,
    raw_hash: String,
    media_type: String,
    page_count: usize,
    namespace: String,

    descriptor_json: String,
    descriptor_text: String,

    descriptor_model: String,
    descriptor_version: String,
    ingestion_version: String,

    score: f64,
}

impl KnowledgeDocumentRow {
    fn into_search_result(self) -> Option<DocumentSearchResult> {
        let descriptor: DocumentDescriptor = serde_json::from_str(&self.descriptor_json).ok()?;

        Some(DocumentSearchResult {
            score: self.score,
            document: KnowledgeDocument {
                document_key: self.document_key,
                source_path: self.source_path,
                raw_hash: self.raw_hash,
                media_type: self.media_type,
                page_count: self.page_count,
                namespace: Namespace::parse(&self.namespace),
                descriptor,
                descriptor_text: self.descriptor_text,
                descriptor_model: self.descriptor_model,
                descriptor_version: self.descriptor_version,
                ingestion_version: self.ingestion_version,
            },
        })
    }
}

impl KnowledgeBase {
    /// Stores document metadata and its catalog embedding.
    pub async fn upsert_document(&self, document: &KnowledgeDocument) -> Result<()> {
        // Embed before deleting the previous record. If Ollama
        // fails, the existing catalog record remains available.
        let embedding = self
            .embedding_model
            .embed_text(&document.descriptor_text)
            .await?
            .vec;

        let descriptor_json = serde_json::to_string(&document.descriptor)?;

        let mut response = self
            .db
            .query(
                r#"
                DELETE FROM knowledge_document
                WHERE source_path = $source_path;

                CREATE knowledge_document SET
                    document_key = $document_key,
                    source_path = $source_path,
                    raw_hash = $raw_hash,
                    media_type = $media_type,
                    page_count = $page_count,
                    namespace = $namespace,
                    descriptor_json = $descriptor_json,
                    descriptor_text = $descriptor_text,
                    descriptor_model = $descriptor_model,
                    descriptor_version = $descriptor_version,
                    ingestion_version = $ingestion_version,
                    embedding_model = $embedding_model,
                    embedding = $embedding;
                "#,
            )
            .bind(("document_key", document.document_key.clone()))
            .bind(("source_path", document.source_path.clone()))
            .bind(("raw_hash", document.raw_hash.clone()))
            .bind(("media_type", document.media_type.clone()))
            .bind(("page_count", document.page_count))
            .bind(("namespace", document.namespace.to_string()))
            .bind(("descriptor_json", descriptor_json))
            .bind(("descriptor_text", document.descriptor_text.clone()))
            .bind(("descriptor_model", document.descriptor_model.clone()))
            .bind(("descriptor_version", document.descriptor_version.clone()))
            .bind(("ingestion_version", document.ingestion_version.clone()))
            .bind(("embedding_model", self.embedding_model_name.clone()))
            .bind(("embedding", embedding))
            .await?;

        let _: Vec<serde_json::Value> = response.take(0)?;

        let _: Vec<serde_json::Value> = response.take(1)?;

        Ok(())
    }

    /// Searches document identities rather than detailed passages.
    pub async fn search_documents(
        &self,
        query: &str,
        top_k: usize,
    ) -> Result<Vec<DocumentSearchResult>> {
        let embedding_query = format!("query: {query}");

        let query_embedding = self.embedding_model.embed_text(&embedding_query).await?.vec;

        let mut response = self
            .db
            .query(
                r#"
                SELECT
                    document_key,
                    source_path,
                    raw_hash,
                    media_type,
                    page_count,
                    namespace,
                    descriptor_json,
                    descriptor_text,
                    descriptor_model,
                    descriptor_version,
                    ingestion_version,
                    vector::similarity::cosine(
                        $embedding,
                        embedding
                    ) AS score
                FROM knowledge_document
                WHERE namespace != 'tmp'
                ORDER BY score DESC
                LIMIT $limit;
                "#,
            )
            .bind(("embedding", query_embedding))
            .bind(("limit", top_k))
            .await?;

        let rows: Vec<KnowledgeDocumentRow> = response.take(0)?;

        Ok(rows
            .into_iter()
            .filter_map(KnowledgeDocumentRow::into_search_result)
            .collect())
    }

    /// Returns the complete document inventory.
    pub async fn list_documents(&self) -> Result<Vec<KnowledgeDocument>> {
        let mut response = self
            .db
            .query(
                r#"
                SELECT
                    document_key,
                    source_path,
                    raw_hash,
                    media_type,
                    page_count,
                    namespace,
                    descriptor_json,
                    descriptor_text,
                    descriptor_model,
                    descriptor_version,
                    ingestion_version,
                    1.0 AS score
                FROM knowledge_document
                ORDER BY descriptor_text ASC;
                "#,
            )
            .await?;

        let rows: Vec<KnowledgeDocumentRow> = response.take(0)?;

        Ok(rows
            .into_iter()
            .filter_map(KnowledgeDocumentRow::into_search_result)
            .map(|result| result.document)
            .collect())
    }

    pub async fn delete_document_for_file(&self, source_path: &str) -> Result<()> {
        self.db
            .query(
                r#"
                DELETE FROM knowledge_document
                WHERE source_path = $source_path;
                "#,
            )
            .bind(("source_path", source_path.to_string()))
            .await?;

        Ok(())
    }
}
