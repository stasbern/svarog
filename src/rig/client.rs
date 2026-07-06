use color_eyre::Result;

use rig::client::{CompletionClient, ProviderClient};
use rig::completion::{Chat, Message};
use rig::providers::ollama::Client;

use std::sync::Arc;
use tokio::sync::mpsc;

use super::knowledge::KnowledgeBase;
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

    pub fn inner(&self) -> &Client {
        &self.client
    }

    pub fn handle_completion(
        self,
        knowledge: Arc<KnowledgeBase>,
        mut prompt_rx: mpsc::Receiver<Request>,
        response_tx: mpsc::Sender<Response>,
    ) {
        tokio::spawn(async move {
            let vector_store = knowledge.vector_store();

            let agent = self
                .client
                .agent(&self.model)
                .preamble(&self.preamble)
                .temperature(self.temperature)
                .dynamic_context(2, vector_store)
                .build();

            let mut chat_history: Vec<Message> = vec![];

            while let Some(Request::Prompt(prompt)) = prompt_rx.recv().await {
                match knowledge.search(&prompt, 4).await {
                    Ok(results) if !results.is_empty() => {
                        let context_info: Vec<(f64, String)> = results
                            .iter()
                            .map(|r| {
                                let preview = r.content.chars().take(120).collect::<String>();
                                (r.score, preview)
                            })
                            .collect();
                        let _ = response_tx.send(Response::ContextFound(context_info)).await;
                    }
                    Ok(_) => {
                        let _ = response_tx
                            .send(Response::ContextFound(vec![(
                                0.0,
                                "no relevant context in KB".into(),
                            )]))
                            .await;
                    }
                    Err(e) => {
                        let _ = response_tx
                            .send(Response::ContextFound(vec![(
                                0.0,
                                format!("KB search error {e}"),
                            )]))
                            .await;
                    }
                }

                match agent.chat(&prompt, &mut chat_history).await {
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
