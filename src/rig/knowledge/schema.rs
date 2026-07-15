// src/rig/knowledge/schema.rs

use color_eyre::Result;
use surrealdb::Surreal;
use surrealdb::engine::local::Db;

pub(super) async fn initialize(db: &Surreal<Db>) -> Result<()> {
    let mut response = db
        .query(
            r#"
            DEFINE TABLE IF NOT EXISTS
                knowledge_document SCHEMALESS;

            DEFINE TABLE IF NOT EXISTS
                file_hashes SCHEMALESS;
            "#,
        )
        .await?;

    // Inspect every statement result so schema errors are surfaced.
    let _: Vec<serde_json::Value> = response.take(0)?;

    let _: Vec<serde_json::Value> = response.take(1)?;

    Ok(())
}
