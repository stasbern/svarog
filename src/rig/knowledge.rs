use serde::{Deserialize, Serialize};
use std::collections;
use std::path::{Path, PathBuf};
use tokio::fs;

use color_eyre::Result;
use rig::client::EmbeddingsClient;
use rig::providers::ollama;
use rig::surrealdb::SurrealVectorStore;
use rig::vector_store::{InsertDocuments, VectorSearchRequest, VectorStoreIndexDyn};
use rig::{Embed, embeddings::EmbeddingsBuilder};

use surrealdb::Surreal;
use surrealdb::engine::local::{Db, RocksDb};

use crate::rig::client;

#[derive(Embed, Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
struct Chunk {
    id: PathBuf,
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
                if let Ok(text) = fs::read_to_string(entry.path()).await {
                    for chunk_text in Self::chunk_input(&text, 1000) {
                        chunks.push(Chunk {
                            id: entry.path(),
                            content: chunk_text,
                        });
                    }
                }
            }
        }

        if chunks.is_empty() {
            return Ok(());
        }

        let embeddings = EmbeddingsBuilder::new(self.embedding_model.clone())
            .document(chunks)?
            .build()
            .await?;

        self.vector_store().insert_documents(embeddings).await?;
        Ok(())
    }

    pub fn vector_store(&self) -> SurrealVectorStore<Db, ollama::EmbeddingModel> {
        SurrealVectorStore::with_defaults(self.embedding_model.clone(), self.db.clone())
    }

    fn chunk_input(text: &str, max_chars: usize) -> Vec<String> {
        text.split("\n\n")
            .flat_map(|paragraph| {
                if paragraph.len() <= max_chars {
                    vec![paragraph.to_string()]
                } else {
                    paragraph
                        .chars()
                        .collect::<Vec<_>>()
                        .chunks(max_chars)
                        .map(|c| c.iter().collect::<String>())
                        .collect()
                }
            })
            .filter(|s| !s.trim().is_empty())
            .collect()
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
                let content = doc
                    .get("content")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                SearchResult { score, id, content }
            })
            .collect())
    }
}
