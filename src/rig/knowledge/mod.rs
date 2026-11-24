mod chunks;
mod documents;
mod embedding;
mod hashes;
mod schema;

use color_eyre::Result;
use rig::providers::openai;
use surrealdb::Surreal;
use surrealdb::engine::local::{Db, RocksDb};

use embedding::CompatEmbeddingModel;
pub use embedding::DEFAULT_BATCH_SIZE as DEFAULT_EMBEDDING_BATCH_SIZE;

pub use chunks::SearchResult;

/// Model-specific embedding instructions using `{text}` and `{title}` placeholders.
#[derive(Debug, Clone)]
pub struct EmbeddingPrompts {
    pub query_template: String,
    pub document_template: String,
}

impl Default for EmbeddingPrompts {
    fn default() -> Self {
        Self {
            query_template: "query: {text}".into(),
            document_template: "Document: {title}\n{text}".into(),
        }
    }
}

impl EmbeddingPrompts {
    fn query(&self, text: &str) -> String {
        self.query_template.replace("{text}", text)
    }

    // Title is substituted first so placeholders inside document text stay literal.
    fn document(&self, title: &str, text: &str) -> String {
        self.document_template
            .replace("{title}", title)
            .replace("{text}", text)
    }
}

/// Public facade for persistent document and passage knowledge.
///
/// Implementation details are divided between:
/// - `schema`: database initialization
/// - `hashes`: incremental-ingestion state
/// - `documents`: document catalog
/// - `chunks`: passage embeddings and retrieval
pub struct KnowledgeBase {
    pub(super) embedding_model: CompatEmbeddingModel,
    pub(super) embedding_model_name: String,
    pub(super) embedding_prompts: EmbeddingPrompts,
    pub(super) db: Surreal<Db>,
}

impl KnowledgeBase {
    pub async fn new(
        base_url: &str,
        api_key: &str,
        embedding_model_name: &str,
        embedding_batch_size: usize,
        embedding_prompts: EmbeddingPrompts,
    ) -> Result<Self> {
        let client = openai::Client::builder()
            .api_key(api_key)
            .base_url(base_url)
            .build()?;

        let db = Surreal::new::<RocksDb>("svarog_vectors.db").await?;

        db.use_ns("svarog_ns").use_db("svarog_db").await?;

        schema::initialize(&db).await?;

        Ok(Self {
            embedding_model: CompatEmbeddingModel::new(
                client,
                embedding_model_name,
                embedding_batch_size,
            ),
            embedding_model_name: embedding_model_name.to_string(),
            embedding_prompts,
            db,
        })
    }
}
