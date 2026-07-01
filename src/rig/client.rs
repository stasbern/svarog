use color_eyre::Result;
use rig::client::{CompletionClient, ProviderClient};
use rig::completion::Prompt;
use rig::providers::ollama::Client;
use tokio::sync::mpsc;

use crate::events::*;

pub struct OllamaClient {
    client: Client,
    model: String,
    preamble: String,
    temperature: f64,
}

impl OllamaClient {
    pub fn new(model: &str, preamble: &str, temperature: f64) -> Self {
        Self {
            client: Client::from_env().unwrap(),
            model: String::from(model),
            preamble: String::from(preamble),
            temperature,
        }
    }

    pub fn handle_prompts(
        self,
        mut prompt_rx: mpsc::Receiver<Request>,
        response_tx: mpsc::Sender<Response>,
    ) {
        tokio::spawn(async move {
            let agent = self
                .client
                .agent(&self.model)
                .preamble(&self.preamble)
                .temperature(self.temperature)
                .build();

            while let Some(Request::Prompt(prompt)) = prompt_rx.recv().await {
                match agent.prompt(prompt).await {
                    Ok(response_text) => {
                        response_tx
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
    }
}
