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
        0.1,
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
