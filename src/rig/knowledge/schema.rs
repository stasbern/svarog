use color_eyre::Result;
use surrealdb::Surreal;
use surrealdb::engine::local::Db;

pub(super) async fn initialize(db: &Surreal<Db>) -> Result<()> {
    db.query(
        r#"
        DEFINE TABLE IF NOT EXISTS
            knowledge_document SCHEMALESS;

        DEFINE TABLE IF NOT EXISTS
            knowledge_chunk SCHEMALESS;

        DEFINE TABLE IF NOT EXISTS
            file_hashes SCHEMALESS;

        DEFINE FIELD IF NOT EXISTS document
            ON TABLE knowledge_chunk
            TYPE record<knowledge_document>;

        DEFINE FIELD IF NOT EXISTS document_key
            ON TABLE knowledge_chunk
            TYPE string;

        DEFINE FIELD IF NOT EXISTS source_path
            ON TABLE knowledge_chunk
            TYPE string;

        DEFINE FIELD IF NOT EXISTS document_title
            ON TABLE knowledge_chunk
            TYPE string;

        DEFINE FIELD IF NOT EXISTS namespace
            ON TABLE knowledge_chunk
            TYPE string;

        DEFINE FIELD IF NOT EXISTS chunk_index
            ON TABLE knowledge_chunk
            TYPE int;

        DEFINE FIELD IF NOT EXISTS page_start
            ON TABLE knowledge_chunk
            TYPE int;

        DEFINE FIELD IF NOT EXISTS page_end
            ON TABLE knowledge_chunk
            TYPE int;

        DEFINE FIELD IF NOT EXISTS content
            ON TABLE knowledge_chunk
            TYPE string;

        DEFINE FIELD IF NOT EXISTS embedding_text
            ON TABLE knowledge_chunk
            TYPE string;

        DEFINE FIELD IF NOT EXISTS embedding
            ON TABLE knowledge_chunk
            TYPE array<float>;

        DEFINE INDEX IF NOT EXISTS
            idx_knowledge_document_key
            ON TABLE knowledge_document
            FIELDS document_key
            UNIQUE;

        DEFINE INDEX IF NOT EXISTS
            idx_knowledge_chunk_document
            ON TABLE knowledge_chunk
            FIELDS document;

        DEFINE INDEX IF NOT EXISTS
            idx_knowledge_chunk_document_key
            ON TABLE knowledge_chunk
            FIELDS document_key;

        DEFINE INDEX IF NOT EXISTS
            idx_knowledge_chunk_namespace
            ON TABLE knowledge_chunk
            FIELDS namespace
            
            DEFINE ANALYZER IF NOT EXISTS svarog_technical
            TOKENIZERS blank, class, camel, punct
            FILTERS lowercase, ascii;

        DEFINE INDEX IF NOT EXISTS idx_knowledge_chunk_fulltext
            ON TABLE knowledge_chunk
            FIELDS embedding_text
            FULLTEXT ANALYZER svarog_technical
            BM25(1.2, 0.75);
                "#,
    )
    .await?
    .check()?;

    Ok(())
}
