use serde::{Deserialize, Serialize};
use std::path::Path;
use std::collections::HashMap;
use tokio::fs;

use color_eyre::Result;
use rig::client::EmbeddingsClient;
use rig::providers::ollama;
use rig::surrealdb::{SurrealVectorStore, SurrealDistanceFunction};
use rig::vector_store::{InsertDocuments, VectorSearchRequest, VectorStoreIndexDyn};
use rig::{Embed, embeddings::EmbeddingsBuilder};

use surrealdb::Surreal;
use surrealdb::engine::local::{Db, RocksDb};

use sha2::{Sha256, Digest};

use super::knowledge_source::Namespace;

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
    db: Surreal<Db>,
}

impl KnowledgeBase {
    pub async fn new(client: &ollama::Client, embedding_model_name: &str) -> Result<Self> {
        let db = Surreal::new::<RocksDb>("svarog_vectors.db").await?;
        db.use_ns("svarog_ns").use_db("svarog_db").await?;

        let embedding_model = client.embedding_model(embedding_model_name);

        Ok(Self {
            embedding_model,
            db,
        })
    }

    pub async fn get_all_hashes(&self) -> Result<HashMap<String, (String, String)>> {
        let mut result = self.db
            .query("SELECT VALUE [file_path, namespace, content_hash] FROM file_hashes")
            .await?;
        let rows: Vec<(String, String, String)> = result.take(0)?;
        Ok(rows
            .into_iter()
            .map(|(path, ns, hash)| (path, (ns, hash)))
            .collect())
    }

    /// Ingest a single file: delete old chunks → chunk → embed → insert → store hash.
    pub async fn ingest_file(&self, path: &str, text: &str, ns: Namespace) -> Result<()> {
        self.delete_chunks_for_file(path, ns).await?;

        let chunks: Vec<Chunk> = super::chunking::chunk_input(text, 400, 80)
            .into_iter()
            .enumerate()
            .map(|(idx, content)| Chunk {
                chunk_index: idx,
                file_path: path.to_string(),
                content,
            })
            .collect();

        if !chunks.is_empty() {
            let mut builder = EmbeddingsBuilder::new(self.embedding_model.clone());
            for chunk in chunks {
                builder = builder.document(chunk)?;
            }
            let embeddings = builder.build().await?;
            self.vector_store_for(ns).insert_documents(embeddings).await?;
        }

        let hash = Self::hash_content(text);
        self.store_hash(path, &hash, ns).await?;
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

    pub fn vector_store_for(&self, ns: Namespace) -> SurrealVectorStore<Db, ollama::EmbeddingModel> {
        SurrealVectorStore::new(self.embedding_model.clone(), self.db.clone(), Some(ns.table_name().to_string()), SurrealDistanceFunction::Cosine)
    }

    pub fn vector_store(&self) -> SurrealVectorStore<Db, ollama::EmbeddingModel> {
        self.vector_store_for(Namespace::Factual) // default to factual for general search
    }

    pub async fn search_namespace(&self, query: &str, ns: Namespace, top_k: u64) -> Result<Vec<SearchResult>> {
        let store = self.vector_store_for(ns);
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
}
