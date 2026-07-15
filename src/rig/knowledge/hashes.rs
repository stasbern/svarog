// src/rig/knowledge/hashes.rs

use std::collections::HashMap;

use color_eyre::Result;
use sha2::{Digest, Sha256};

use super::KnowledgeBase;
use crate::rig::knowledge_source::Namespace;

impl KnowledgeBase {
    pub async fn get_all_hashes(&self) -> Result<HashMap<String, (String, String)>> {
        let mut response = self
            .db
            .query(
                r#"
                SELECT VALUE
                    [file_path, namespace, content_hash]
                FROM file_hashes;
                "#,
            )
            .await?;

        let rows: Vec<(String, String, String)> = response.take(0)?;

        Ok(rows
            .into_iter()
            .map(|(path, namespace, hash)| (path, (namespace, hash)))
            .collect())
    }

    pub fn hash_content(content: &str) -> String {
        let digest = Sha256::digest(content.as_bytes());

        digest.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    pub(super) async fn store_hash(
        &self,
        file_path: &str,
        hash: &str,
        namespace: Namespace,
    ) -> Result<()> {
        let mut response = self
            .db
            .query(
                r#"
                DELETE FROM file_hashes
                WHERE file_path = $path;

                CREATE file_hashes SET
                    file_path = $path,
                    content_hash = $hash,
                    namespace = $namespace;
                "#,
            )
            .bind(("path", file_path.to_string()))
            .bind(("hash", hash.to_string()))
            .bind(("namespace", namespace.to_string()))
            .await?;

        let _: Vec<serde_json::Value> = response.take(0)?;

        let _: Vec<serde_json::Value> = response.take(1)?;

        Ok(())
    }
}
