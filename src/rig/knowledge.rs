use serde::{Deserialize, Serialize};
use std::path::Path;
use tokio::fs;

use color_eyre::Result;
use rig::client::EmbeddingsClient;
use rig::providers::ollama;
use rig::surrealdb::SurrealVectorStore;
use rig::vector_store::{InsertDocuments, VectorSearchRequest, VectorStoreIndexDyn};
use rig::{Embed, embeddings::EmbeddingsBuilder};

use surrealdb::Surreal;
use surrealdb::engine::local::{Db, RocksDb};

use sha2::{Sha256, Digest};

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
}

pub struct KnowledgeBase {
    embedding_model: ollama::EmbeddingModel,
    db: Surreal<Db>,
}

impl KnowledgeBase {
    pub async fn new(client: &ollama::Client, embeddig_model_name: &str) -> Result<Self> {
        let db = Surreal::new::<RocksDb>("svarog_vectors.db").await?;
        db.use_ns("svarog_ns").use_db("svarog_db").await?;

        let embedding_model = client.embedding_model(embeddig_model_name);

        Ok(Self {
            embedding_model,
            db,
        })
    }

    pub async fn ingest_directory(&self, dir: &Path) -> Result<()> {
        let mut chunks: Vec<Chunk> = vec![];

        if let Ok(mut entries) = fs::read_dir(&dir).await {
            while let Some(entry) = entries.next_entry().await? {
                let Ok(text) = fs::read_to_string(entry.path()).await else {
                    continue;
                };

                // Normalize path: forward slashes
                let normalized_path = entry.path().to_string_lossy().replace('\\', "/");
                let hash = Self::hash_content(&text);

                // Skip unchanged files
                if let Ok(Some(stored_hash)) = self.get_stored_hash(&normalized_path).await {
                    if stored_hash == hash {
                        continue;
                    }
                }

                // File is new or changed — remove old chunks
                self.delete_chunks_for_file(&normalized_path).await?;

                // Build new chunks with overlap
                for (idx, chunk_text) in super::chunking::chunk_input(&text, 400, 80).iter().enumerate() {
                    chunks.push(Chunk {
                        chunk_index: idx,
                        file_path: normalized_path.clone(),
                        content: chunk_text.clone(),
                    });
                }

                // Update stored hash
                self.store_hash(&normalized_path, &hash).await?;
            }
        }

        if chunks.is_empty() {
            return Ok(());
        }

        let mut builder = EmbeddingsBuilder::new(self.embedding_model.clone());
        for chunk in chunks {
            builder = builder.document(chunk)?;
        }
        let embeddings = builder.build().await?;
        self.vector_store().insert_documents(embeddings).await?;

        Ok(())
    }

    pub fn vector_store(&self) -> SurrealVectorStore<Db, ollama::EmbeddingModel> {
        SurrealVectorStore::with_defaults(self.embedding_model.clone(), self.db.clone())
    }

    pub async fn search(&self, query: &str, top_k: u64) -> Result<Vec<SearchResult>> {
        let store = self.vector_store();
        let req = VectorSearchRequest::builder()
            .query(query)
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

                SearchResult { score, id, content }
            })
            .collect())
    }

    fn hash_content(content: &str) -> String {
        let result = Sha256::digest(content.as_bytes());
        result.iter().map(|b| format!("{:02x}", b)).collect()
    }
    
    async fn get_stored_hash(&self, file_path: &str) -> Result<Option<String>> {
        let mut result = self
            .db
            .query("SELECT content_hash FROM file_hashes WHERE file_path = $path LIMIT 1")
            .bind(("path", file_path))
            .await?;

        let hash: Option<String> = result.take(0)?;
        Ok(hash)
    }

    async fn store_hash(&self, file_path: &str, hash: &str) -> Result<()> {
        // Delete any existing record for this path, then create new one
        self.db
            .query("DELETE FROM file_hashes WHERE file_path = $path")
            .bind(("path", file_path))
            .await?;

        self.db
            .query("CREATE file_hashes SET file_path = $path, content_hash = $hash")
            .bind(("path", file_path))
            .bind(("hash", hash))
            .await?;

        Ok(())
    }

    async fn delete_chunks_for_file(&self, file_path: &str) -> Result<()> {
        // rig stores the serialized Chunk as a JSON string in the `document` column.
        // We match on the "file_path":"..." fragment inside that JSON string.
        let needle = format!("\"file_path\":\"{}\"", file_path);
        self.db
            .query("DELETE FROM documents WHERE string::contains(document, $needle)")
            .bind(("needle", needle))
            .await?;

        Ok(())
    }
}
