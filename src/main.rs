use color_eyre::Result;
use std::env;
use std::sync::Arc;

pub mod events;

pub mod tui;
use crate::tui::app::*;

pub mod rig;
use crate::rig::client::*;
use crate::rig::knowledge::*;

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;
    dotenvy::dotenv().expect(".env file was not read successfully");

    // initialize tokio mpsc channels
    let channels = events::Channels::new()?;

    let llm_base_url = env::var("LLM_BASE_URL")?;
    // Local servers such as llama-server accept requests without a key.
    let llm_api_key = env::var("LLM_API_KEY").unwrap_or_default();
    // A Rig dialect name: `openai` covers Gemini; local servers use e.g. `llamacpp`.
    let llm_dialect = env::var("LLM_DIALECT").unwrap_or_else(|_| "openai".into());

    // Only send configured params; providers differ on non-standard fields.
    let mut sampling_params = serde_json::Map::new();
    for (env_key, param) in [
        ("TOP_P", "top_p"),
        ("TOP_K", "top_k"),
        ("REPEAT_PENALTY", "repeat_penalty"),
    ] {
        if let Some(value) = env::var(env_key)
            .ok()
            .and_then(|v| v.trim().parse::<serde_json::Number>().ok())
        {
            sampling_params.insert(param.to_string(), value.into());
        }
    }

    let client = LlmClient::new(
        connect(&llm_dialect, &llm_base_url, &llm_api_key)?,
        &env::var("BASE_MODEL")?,
        &env::var("BASE_MODEL_PREAMBLE")?,
        env::var("TEMPERATURE")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0.7),
        serde_json::Value::Object(sampling_params),
    );

    // Embeddings may live on a separate server, e.g. llama-server --embedding.
    let embedding_model = env::var("EMBEDDING_MODEL")?;
    let embedder = Embedder::new(
        &connect(
            &env::var("EMBEDDING_DIALECT").unwrap_or(llm_dialect),
            &env::var("EMBEDDING_BASE_URL").unwrap_or(llm_base_url),
            &env::var("EMBEDDING_API_KEY").unwrap_or(llm_api_key),
        )?,
        &embedding_model,
        env::var("EMBEDDING_BATCH_SIZE")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(DEFAULT_EMBEDDING_BATCH_SIZE),
    );

    let default_prompts = EmbeddingPrompts::default();
    let knowledge = Arc::new(
        KnowledgeBase::new(
            embedder,
            &embedding_model,
            EmbeddingPrompts {
                query_template: env::var("EMBEDDING_QUERY_TEMPLATE")
                    .unwrap_or(default_prompts.query_template),
                document_template: env::var("EMBEDDING_DOCUMENT_TEMPLATE")
                    .unwrap_or(default_prompts.document_template),
            },
        )
        .await?,
    );

    client.handle_requests(knowledge.clone(), channels.prompt_rx, channels.response_tx);

    // initialize tui with channels
    let mut terminal = ratatui::init();
    let app_result = App::new(channels.prompt_tx, channels.response_rx)
        .run(&mut terminal)
        .await;

    ratatui::restore();

    app_result
}

#[cfg(test)]
mod tests {}
