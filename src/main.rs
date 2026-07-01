use color_eyre::Result;

pub mod events;
use crate::events::*;
pub mod tui;
use crate::tui::app::*;
pub mod rig;
use crate::rig::client::*;

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;

    // initialize tokio mpsc channels
    let mut channels = events::Channels::new();

    // initialize ollama client and agent
    let client = OllamaClient::new(
        "qwen3:30b-a3b-instruct-2507-q4_K_M",
        "I am building up a Rust stack wrapper around you. I am using rig.rs + ratatui.rs + ollama",
        0.5,
    );
    client.handle_prompts(channels.prompt_rx, channels.response_tx);

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
