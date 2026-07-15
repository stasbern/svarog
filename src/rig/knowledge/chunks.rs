use color_eyre::Result;
use rig::{
    Embed,
    embeddings::{EmbeddingModel, EmbeddingsBuilder},
};
use serde::{Deserialize, Serialize};
use surrealdb::types::{RecordId, SurrealValue, ToSql};

use super::KnowledgeBase;
use crate::rig::chunking::chunk_input;
use crate::rig::document::KnowledgeDocument;
use crate::rig::ingestion::ExtractedPage;
use crate::rig::knowledge_source::Namespace;

const CHUNK_MAX_CHARACTERS: usize = 1_800;
const CHUNK_OVERLAP_CHARACTERS: usize = 250;

/// Intermediate document given to Rig for embedding.
#[derive(Debug, Serialize, Embed)]
struct ChunkDraft {
    id: RecordId,
    document: RecordId,
    document_key: String,
    source_path: String,
    document_title: String,
    namespace: String,
    chunk_index: usize,
    page_start: usize,
    page_end: usize,
    content: String,

    #[embed]
    embedding_text: String,
}

/// Native SurrealDB representation.
#[derive(Debug, Serialize, SurrealValue)]
struct StoredChunk {
    id: RecordId,
    document: RecordId,
    document_key: String,
    source_path: String,
    document_title: String,
    namespace: String,
    chunk_index: usize,
    page_start: usize,
    page_end: usize,
    content: String,
    embedding_text: String,
    embedding: Vec<f64>,
}

/// Projection returned by native vector search.
#[derive(Debug, Deserialize, SurrealValue)]
struct ChunkSearchRow {
    id: RecordId,
    document: RecordId,
    document_key: String,
    document_title: String,
    namespace: String,
    chunk_index: usize,
    page_start: usize,
    page_end: usize,
    content: String,
    score: f64,
}

impl ChunkSearchRow {
    fn into_search_result(self) -> SearchResult {
        SearchResult {
            score: self.score,
            id: self.id.to_sql(),
            document_id: self.document.to_sql(),
            document_key: self.document_key,
            document_title: self.document_title,
            content: self.content,
            namespace: Namespace::parse(&self.namespace),
            chunk_index: self.chunk_index,
            page_start: self.page_start,
            page_end: self.page_end,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SearchResult {
    pub score: f64,
    pub id: String,
    pub document_id: String,
    pub document_key: String,
    pub document_title: String,
    pub content: String,
    pub namespace: Namespace,
    pub chunk_index: usize,
    pub page_start: usize,
    pub page_end: usize,
}

impl SearchResult {
    pub fn source_label(&self) -> String {
        if self.page_start == self.page_end {
            format!("{}, page {}", self.document_title, self.page_start,)
        } else {
            format!(
                "{}, pages {}–{}",
                self.document_title, self.page_start, self.page_end,
            )
        }
    }
}

impl KnowledgeBase {
    /// Replaces every persisted chunk belonging to one document.
    pub async fn replace_document_chunks(
        &self,
        document: &KnowledgeDocument,
        pages: &[ExtractedPage],
        pipeline_hash: &str,
    ) -> Result<usize> {
        let drafts = create_chunk_drafts(document, pages);

        if drafts.is_empty() {
            return Err(color_eyre::eyre::eyre!(
                "document produced no non-empty chunks"
            ));
        }

        let records = self.embed_chunk_drafts(drafts).await?;

        let chunk_count = records.len();

        self.replace_stored_chunks(document, records).await?;

        self.store_hash(&document.source_path, pipeline_hash, document.namespace)
            .await?;

        Ok(chunk_count)
    }

    async fn embed_chunk_drafts(&self, drafts: Vec<ChunkDraft>) -> Result<Vec<StoredChunk>> {
        let mut builder = EmbeddingsBuilder::new(self.embedding_model.clone());

        for draft in drafts {
            builder = builder.document(draft)?;
        }

        let embedded_documents = builder.build().await?;

        let mut records = Vec::new();

        for (draft, embeddings) in embedded_documents {
            // `ChunkDraft` has exactly one #[embed] field, so this
            // normally produces exactly one embedding.
            for embedding in embeddings {
                records.push(StoredChunk {
                    id: draft.id.clone(),
                    document: draft.document.clone(),
                    document_key: draft.document_key.clone(),
                    source_path: draft.source_path.clone(),
                    document_title: draft.document_title.clone(),
                    namespace: draft.namespace.clone(),
                    chunk_index: draft.chunk_index,
                    page_start: draft.page_start,
                    page_end: draft.page_end,
                    content: draft.content.clone(),
                    embedding_text: embedding.document,
                    embedding: embedding.vec,
                });
            }
        }

        Ok(records)
    }

    async fn replace_stored_chunks(
        &self,
        document: &KnowledgeDocument,
        records: Vec<StoredChunk>,
    ) -> Result<()> {
        let document_id = RecordId::new("knowledge_document", document.document_key.clone());

        let mut delete_response = self
            .db
            .query(
                r#"
                DELETE FROM knowledge_chunk
                WHERE document = $document;
                "#,
            )
            .bind(("document", document_id))
            .await?;

        let _: Vec<serde_json::Value> = delete_response.take(0)?;

        // Each chunk carries a deterministic ID, so reingestion cannot
        // accidentally duplicate an unchanged chunk index.
        let _: Vec<serde_json::Value> = self.db.insert("knowledge_chunk").content(records).await?;

        Ok(())
    }

    pub async fn search_namespace(
        &self,
        query: &str,
        namespace: Namespace,
        top_k: u64,
    ) -> Result<Vec<SearchResult>> {
        self.search_vector(query, &[namespace], top_k as usize)
            .await
    }

    pub async fn search_vector(
        &self,
        query: &str,
        namespaces: &[Namespace],
        top_k: usize,
    ) -> Result<Vec<SearchResult>> {
        let embedding_query = format!("query: {query}");

        let query_embedding = self.embedding_model.embed_text(&embedding_query).await?.vec;

        let namespaces = namespaces
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>();

        let mut response = self
            .db
            .query(
                r#"
            SELECT
                id,
                document,
                document_key,
                document_title,
                namespace,
                chunk_index,
                page_start,
                page_end,
                content,
                vector::similarity::cosine(
                    $embedding,
                    embedding
                ) AS score
            FROM knowledge_chunk
            WHERE namespace IN $namespaces
            ORDER BY score DESC
            LIMIT $limit;
            "#,
            )
            .bind(("embedding", query_embedding))
            .bind(("namespaces", namespaces))
            .bind(("limit", top_k))
            .await?;

        let rows: Vec<ChunkSearchRow> = response.take(0)?;

        Ok(rows
            .into_iter()
            .map(ChunkSearchRow::into_search_result)
            .collect())
    }

    pub async fn search_full_text(
        &self,
        query: &str,
        namespaces: &[Namespace],
        top_k: usize,
    ) -> Result<Vec<SearchResult>> {
        let namespaces = namespaces
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>();

        let mut response = self
            .db
            .query(
                r#"
            SELECT
                id,
                document,
                document_key,
                document_title,
                namespace,
                chunk_index,
                page_start,
                page_end,
                content,
                search::score(1) AS score
            FROM knowledge_chunk
            WHERE embedding_text @1@ $query
              AND namespace IN $namespaces
            ORDER BY score DESC
            LIMIT $limit;
            "#,
            )
            .bind(("query", query.to_string()))
            .bind(("namespaces", namespaces))
            .bind(("limit", top_k))
            .await?;

        let rows: Vec<ChunkSearchRow> = response.take(0)?;

        Ok(rows
            .into_iter()
            .map(ChunkSearchRow::into_search_result)
            .collect())
    }

    /// Compatibility helper for callers that only know a source path.
    ///
    /// New ingestion code should prefer replacing chunks by document.
    pub async fn delete_chunks_for_file(
        &self,
        source_path: &str,
        namespace: Namespace,
    ) -> Result<()> {
        let mut response = self
            .db
            .query(
                r#"
                DELETE FROM knowledge_chunk
                WHERE source_path = $source_path
                  AND namespace = $namespace;
                "#,
            )
            .bind(("source_path", source_path.to_string()))
            .bind(("namespace", namespace.to_string()))
            .await?;

        let _: Vec<serde_json::Value> = response.take(0)?;

        Ok(())
    }
}

fn create_chunk_drafts(document: &KnowledgeDocument, pages: &[ExtractedPage]) -> Vec<ChunkDraft> {
    let document_id = RecordId::new("knowledge_document", document.document_key.clone());

    let identifiers = if document.descriptor.identifiers.is_empty() {
        String::new()
    } else {
        format!(
            "\nIdentifiers: {}",
            document.descriptor.identifiers.join("; "),
        )
    };

    let mut chunk_index = 0;
    let mut drafts = Vec::new();

    for page in pages {
        if page.text.trim().is_empty() {
            continue;
        }

        let page_chunks = chunk_input(&page.text, CHUNK_MAX_CHARACTERS, CHUNK_OVERLAP_CHARACTERS);

        for content in page_chunks {
            let content = content.trim().to_string();

            if content.is_empty() {
                continue;
            }

            let embedding_text = format!(
                "Document: {}{}\
                 \nPage: {}\
                 \n\n{}",
                document.descriptor.title, identifiers, page.number, content,
            );

            let chunk_id = RecordId::new(
                "knowledge_chunk",
                format!("{}-{chunk_index}", document.document_key,),
            );

            drafts.push(ChunkDraft {
                id: chunk_id,
                document: document_id.clone(),
                document_key: document.document_key.clone(),
                source_path: document.source_path.clone(),
                document_title: document.descriptor.title.clone(),
                namespace: document.namespace.to_string(),
                chunk_index,
                page_start: page.number,
                page_end: page.number,
                content,
                embedding_text,
            });

            chunk_index += 1;
        }
    }

    drafts
}

#[cfg(test)]
mod tests {
    use super::create_chunk_drafts;
    use crate::rig::document::{DocumentDescriptor, KnowledgeDocument};
    use crate::rig::ingestion::ExtractedPage;
    use crate::rig::knowledge_source::Namespace;

    #[test]
    fn chunks_are_linked_to_document_and_page() {
        let document = KnowledgeDocument {
            document_key: "document-key".into(),
            source_path: "manual.pdf".into(),
            raw_hash: "hash".into(),
            media_type: "application/pdf".into(),
            page_count: 2,
            namespace: Namespace::Factual,
            descriptor: DocumentDescriptor {
                title: "Manual".into(),
                identifiers: vec!["ABC123".into()],
                ..Default::default()
            },
            descriptor_text: String::new(),
            descriptor_model: "test".into(),
            descriptor_version: "test".into(),
            ingestion_version: "test".into(),
        };

        let pages = vec![
            ExtractedPage {
                number: 1,
                text: "First page content".into(),
            },
            ExtractedPage {
                number: 2,
                text: "Second page content".into(),
            },
        ];

        let chunks = create_chunk_drafts(&document, &pages);

        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].page_start, 1);
        assert_eq!(chunks[1].page_start, 2);
        assert_eq!(chunks[0].document_key, "document-key");
        assert!(chunks[0].embedding_text.contains("ABC123"));
    }
}
