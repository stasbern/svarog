use serde::{Deserialize, Serialize};
use std::collections;
use std::fmt::format;
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
                    for chunk_text in Self::chunk_input(&text, 400) {
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

    fn chunk_input(text: &str, max_chars: usize) -> Vec<String> {
        const SEPARATORS: &[&str] = &["\n\n\n", "\n\n", "\n", ". ", ", ", " "];

        fn split_recursive(text: &str, max_chars: usize, sep_idx: usize) -> Vec<String> {
            if text.len() <= max_chars || sep_idx >= SEPARATORS.len() {
                if text.len() <= max_chars {
                    return if text.trim().is_empty() {
                        vec![]
                    } else {
                        vec![text.to_string()]
                    };
                }
                return text
                    .chars()
                    .collect::<Vec<_>>()
                    .chunks(max_chars)
                    .map(|c| c.iter().collect::<String>())
                    .filter(|s| !s.trim().is_empty())
                    .collect();
            }

            let sep = SEPARATORS[sep_idx];
            let parts: Vec<&str> = text.split(sep).collect();

            if parts.len() == 1 {
                return split_recursive(text, max_chars, sep_idx + 1);
            }

            let mut chunks = Vec::new();
            let mut current = String::new();

            for part in parts {
                let candidate = if current.is_empty() {
                    part.to_string()
                } else {
                    format!("{}{}{}", current, sep, part)
                };
                if candidate.len() <= max_chars {
                    current = candidate;
                } else {
                    if !current.trim().is_empty() {
                        chunks.push(current);
                    }
                    if part.len() > max_chars {
                        chunks.extend(split_recursive(part, max_chars, sep_idx + 1));
                        current = String::new();
                    } else {
                        current = part.to_string();
                    }
                }
            }
            if !current.trim().is_empty() {
                chunks.push(current);
            }
            chunks
        }
        split_recursive(text, max_chars, 0)
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
}
