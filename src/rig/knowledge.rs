use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use tokio::fs;

use color_eyre::Result;
use rig::client::EmbeddingsClient;
use rig::providers::ollama;
use rig::surrealdb::{SurrealDistanceFunction, SurrealVectorStore};
use rig::vector_store::{InsertDocuments, VectorSearchRequest, VectorStoreIndexDyn};
use rig::{
    Embed,
    embeddings::{EmbeddingModel, EmbeddingsBuilder},
};
use surrealdb::Surreal;
use surrealdb::engine::local::{Db, RocksDb};

use sha2::{Digest, Sha256};

use super::knowledge_source::Namespace;
use super::document::{
    DocumentSearchResult,
    KnowledgeDocument,
    KnowledgeDocumentRow,
};

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

pub struct KnowledgeBase {
    embedding_model: ollama::EmbeddingModel,
    embedding_model_name: String,
    db: Surreal<Db>,
}

impl KnowledgeBase {
    pub async fn new(client: &ollama::Client, embedding_model_name: &str) -> Result<Self> {
        let db = Surreal::new::<RocksDb>("svarog_vectors.db").await?;
        db.use_ns("svarog_ns").use_db("svarog_db").await?;

        let embedding_model = client.embedding_model(embedding_model_name);

        Ok(Self {
            embedding_model,
            embedding_model_name: embedding_model_name.to_string(),
            db,
        })
    }

    pub async fn get_all_hashes(&self) -> Result<HashMap<String, (String, String)>> {
        let mut result = self
            .db
            .query("SELECT VALUE [file_path, namespace, content_hash] FROM file_hashes")
            .await?;
        let rows: Vec<(String, String, String)> = result.take(0)?;
        Ok(rows
            .into_iter()
            .map(|(path, ns, hash)| (path, (ns, hash)))
            .collect())
    }

    /// Ingest text whose hash is derived from that text.
    ///
    /// Useful for generated text or callers that do not have the original bytes.
    pub async fn ingest_file(
        &self,
        path: &str,
        text: &str,
        ns: Namespace,
    ) -> Result<()> {
        let content_hash = Self::hash_content(text);

        self.ingest_file_with_hash(path, text, ns, &content_hash)
            .await
    }

    /// Ingest extracted text while tracking the hash of the original source bytes.
    ///
    /// PDF extraction can change as the extraction implementation evolves, while
    /// this hash continues to identify whether the original PDF itself changed.
    pub async fn ingest_file_with_hash(
        &self,
        path: &str,
        text: &str,
        ns: Namespace,
        source_hash: &str,
    ) -> Result<()> {
        self.delete_chunks_for_file(path, ns).await?;

        let chunks: Vec<Chunk> = super::chunking::chunk_input(text, 400, 80)
            .into_iter()
            .enumerate()
            .map(|(idx, chunk_text)| Chunk {
                chunk_index: idx,
                file_path: path.to_string(),
                content: chunk_text,
            })
            .collect();

        if !chunks.is_empty() {
            let mut builder = EmbeddingsBuilder::new(self.embedding_model.clone());

            for chunk in chunks {
                builder = builder.document(chunk)?;
            }

            let embeddings = builder.build().await?;

            self.vector_store_for(ns)
                .insert_documents(embeddings)
                .await?;
        }

        self.store_hash(path, source_hash, ns).await?;

        Ok(())
    }

    pub async fn ingest_directory(&self, dir: &Path, ns: Namespace) -> Result<()> {
        let stored = self.get_all_hashes().await.unwrap_or_default();

        if let Ok(mut entries) = fs::read_dir(&dir).await {
            while let Some(entry) = entries.next_entry().await? {
                let Ok(text) = fs::read_to_string(entry.path()).await else {
                    continue;
                };
                let path = entry.path().to_string_lossy().replace('\\', "/");
                let hash = Self::hash_content(&text);

                if let Some((_, stored_hash)) = stored.get(&path) {
                    if *stored_hash == hash {
                        continue;
                    }
                }

                self.ingest_file(&path, &text, ns).await?;
            }
        }
        Ok(())
    }

    pub fn vector_store_for(
        &self,
        ns: Namespace,
    ) -> SurrealVectorStore<Db, ollama::EmbeddingModel> {
        SurrealVectorStore::new(
            self.embedding_model.clone(),
            self.db.clone(),
            Some(ns.table_name().to_string()),
            SurrealDistanceFunction::Cosine,
        )
    }

    pub fn vector_store(&self) -> SurrealVectorStore<Db, ollama::EmbeddingModel> {
        self.vector_store_for(Namespace::Factual) // default to factual for general search
    }

    pub async fn search_namespace(
        &self,
        query: &str,
        ns: Namespace,
        top_k: u64,
    ) -> Result<Vec<SearchResult>> {
        let store = self.vector_store_for(ns);
        let req = VectorSearchRequest::builder()
            .query(format!("query: {query}"))
            .samples(top_k)
            .build();

        let results = store
            .top_n(req)
            .await
            .map_err(|e| color_eyre::eyre::eyre!("{e}"))?;

        Ok(results
            .into_iter()
            .map(|(score, id, doc)| {
                let content = if let Some(arr) = doc.as_array() {
                    arr.iter()
                        .filter_map(|item| item.get("content").and_then(|v| v.as_str()))
                        .collect::<Vec<_>>()
                        .join("\n")
                } else {
                    doc.get("content")
                        .and_then(|v| v.as_str())
                        .or_else(|| doc.as_str())
                        .unwrap_or("")
                        .to_string()
                };
                SearchResult { score, id, content, namespace: ns }
            })
            .collect())
    }

    pub async fn search_multi(&self, query: &str, namespaces: &[Namespace], top_k: u64) -> Result<Vec<SearchResult>> {
        let mut all = Vec::new();
        for &ns in namespaces {
            match self.search_namespace(query, ns, top_k).await {
                Ok(mut results) => all.append(&mut results),
                Err(_) => {}, // table may not exist yet
            }
        }
        all.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        all.truncate(top_k as usize);
        Ok(all)
    }

    pub fn hash_content(content: &str) -> String {
        let result = Sha256::digest(content.as_bytes());
        result.iter().map(|b| format!("{:02x}", b)).collect()
    }
    
    #[allow(dead_code)]
    async fn get_stored_hash(&self, file_path: &str, ns: Namespace) -> Result<Option<String>> {
        let mut result = self.db
            .query("SELECT VALUE content_hash FROM file_hashes WHERE file_path = $path AND namespace = $ns LIMIT 1")
            .bind(("path", file_path))
            .bind(("ns", ns.to_string()))
            .await?;
        let hash: Option<String> = result.take(0)?;
        Ok(hash)
    }

    async fn store_hash(&self, file_path: &str, hash: &str, ns: Namespace) -> Result<()> {
        self.db
            .query("DELETE FROM file_hashes WHERE file_path = $path")
            .bind(("path", file_path))
            .await?;
        self.db
            .query("CREATE file_hashes SET file_path = $path, content_hash = $hash, namespace = $ns")
            .bind(("path", file_path))
            .bind(("hash", hash))
            .bind(("ns", ns.to_string()))
            .await?;
        Ok(())
    }

    pub async fn delete_chunks_for_file(&self, file_path: &str, ns: Namespace) -> Result<()> {
        let needle = format!("\"file_path\":\"{}\"", file_path);
        let query = format!(
            "DELETE FROM {} WHERE string::contains(document, $needle)",
            ns.table_name()
        );
        self.db.query(&query).bind(("needle", needle)).await?;
        Ok(())
    }

    /// Stores document metadata and its catalog embedding as native SurrealDB
    /// fields rather than an opaque serialized vector-store document.
    pub async fn upsert_document(&self, document: &KnowledgeDocument) -> Result<()> {
        // Generate the embedding before deleting the prior record. If Ollama fails,
        // the existing catalog record remains intact.
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

        // Force SurrealDB to deserialize both statements so statement-level errors
        // are not silently ignored.
        let _: Vec<serde_json::Value> = response.take(0)?;
        let _: Vec<serde_json::Value> = response.take(1)?;

        Ok(())
    }

    /// Semantic search over document identity rather than passage contents.
    ///
    /// A brute-force cosine scan is acceptable while the document catalog is
    /// small. An HNSW index can replace this without changing the API later.
    pub async fn search_documents(
        &self,
        query: &str,
        top_k: usize,
    ) -> Result<Vec<DocumentSearchResult>> {
        let embedding_query = format!("query: {query}");

        let query_embedding = self
            .embedding_model
            .embed_text(&embedding_query)
            .await?
            .vec;

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
                    vector::similarity::cosine($embedding, embedding) AS score
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

    /// Deterministic inventory access for future commands such as `/documents`.
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
                "DELETE FROM knowledge_document WHERE source_path = $source_path",
            )
            .bind(("source_path", source_path.to_string()))
            .await?;

        Ok(())
    }
}
