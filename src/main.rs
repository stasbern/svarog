use color_eyre::Result;
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

    // initialize tokio mpsc channels
    let channels = events::Channels::new();

    // initialize ollama client and agent
    let client = OllamaClient::new(
        "qwen3:30b-a3b-instruct-2507-q4_K_M",
        "You help me figure out this job posting, do not invent anything, if the info is not in the context, simply say you don't know.",
        0.1,
    );

    let knowledge = Arc::new(KnowledgeBase::new(client.inner(), "nomic-embed-text").await?);

    client.handle_completion(knowledge.clone(), channels.prompt_rx, channels.response_tx);

    // initialize tui with channels
    let mut terminal = ratatui::init();
    let app_result = App::new(channels.prompt_tx, channels.response_rx, knowledge.clone())
        .run(&mut terminal)
        .await;

    ratatui::restore();

    app_result
}

#[cfg(test)]
mod tests {}
