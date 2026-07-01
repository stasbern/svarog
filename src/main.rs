use rig::client::{CompletionClient, ProviderClient};
use rig::completion::Prompt;
use rig::providers::ollama;

use color_eyre::Result;

pub mod events;
use crate::events::*;
pub mod tui;
use crate::tui::app::*;

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;

    // initialize tokio mpsc channels
    let mut channels = events::Channels::new();

    // initialize ollama client and agent
    let ollama_client = ollama::Client::from_env()?;
    tokio::spawn(async move {
        let ollama_agent = ollama_client
            .agent("qwen3:30b-a3b-instruct-2507-q4_K_M")
            .preamble("I am building up a Rust stack wrapper around you. I am using rig.rs + ratatui.rs + ollama")
            .temperature(0.5)
            .build();
        while let Some(Request::Prompt(prompt)) = channels.prompt_rx.recv().await {
            match ollama_agent.prompt(prompt).await {
                Ok(response_text) => {
                    channels
                        .response_tx
                        .send(Response::CompleteResponse(response_text))
                        .await;
                }
                Err(e) => {
                    eprintln!("{:?}", e);
                    // channels.response_tx.send(Response::Error(e)).await;
                }
            }
        }
    });

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
