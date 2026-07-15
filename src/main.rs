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

    // initialize ollama client and agent
    let client = OllamaClient::new(
        &env::var("OLLAMA_BASE_MODEL")?,
        &env::var("BASE_MODEL_PREAMBLE")?,
        env::var("TEMPERATURE")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0.7),
        serde_json::json!({
            "top_p": env::var("TOP_P").ok().and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.95),
            "top_k": env::var("TOP_K").ok().and_then(|v| v.parse::<u32>().ok()).unwrap_or(40),
            "repeat_penalty": env::var("REPEAT_PENALTY").ok().and_then(|v| v.parse::<f64>().ok()).unwrap_or(1.0),
            "num_ctx": env::var("NUM_CTX").ok().and_then(|v| v.parse::<u32>().ok()).unwrap_or(8192),
        }),
    );

    let knowledge =
        Arc::new(KnowledgeBase::new(client.inner(), &env::var("OLLAMA_EMBEDDING_MODEL")?).await?);

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
