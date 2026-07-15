mod chunks;
mod documents;
mod hashes;
mod schema;

use color_eyre::Result;
use rig::client::EmbeddingsClient;
use rig::providers::ollama;
use surrealdb::Surreal;
use surrealdb::engine::local::{Db, RocksDb};

pub use chunks::SearchResult;

/// Public facade for persistent document and passage knowledge.
///
/// Implementation details are divided between:
/// - `schema`: database initialization
/// - `hashes`: incremental-ingestion state
/// - `documents`: document catalog
/// - `chunks`: passage embeddings and retrieval
pub struct KnowledgeBase {
    pub(super) embedding_model: ollama::EmbeddingModel,
    pub(super) embedding_model_name: String,
    pub(super) db: Surreal<Db>,
}

impl KnowledgeBase {
    pub async fn new(client: &ollama::Client, embedding_model_name: &str) -> Result<Self> {
        let db = Surreal::new::<RocksDb>("svarog_vectors.db").await?;

        db.use_ns("svarog_ns").use_db("svarog_db").await?;

        schema::initialize(&db).await?;

        Ok(Self {
            embedding_model: client.embedding_model(embedding_model_name),
            embedding_model_name: embedding_model_name.to_string(),
            db,
        })
    }
}
