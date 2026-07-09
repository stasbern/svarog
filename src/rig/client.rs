use color_eyre::Result;

use rig::client::{CompletionClient, ProviderClient};
use rig::completion::{Chat, Message, Prompt};
use rig::providers::ollama::Client;

use std::sync::Arc;
use tokio::sync::mpsc;

use super::knowledge::KnowledgeBase;
use super::knowledge_source::Namespace;
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
            client: Client::from_env().expect("Failed to connect to Ollama — is OLLAMA_HOST set?"),
            model: String::from(model),
            preamble: String::from(preamble),
            temperature,
        }
    }

    pub fn inner(&self) -> &Client {
        &self.client
    }

     pub async fn classify_text(&self, text: &str) -> Namespace {
        let classifier = self.client
            .agent(&self.model)
            .preamble(Namespace::classifier_prompt())
            .temperature(0.0)
            .build();

        let preview: String = text.chars().take(800).collect();
        match classifier.prompt(&preview).await {
            Ok(response) => Namespace::parse(&response),
            Err(_) => Namespace::Factual,
        }
    }

    pub fn handle_requests(
        &self,
        knowledge: Arc<KnowledgeBase>,
        mut prompt_rx: mpsc::Receiver<Request>,
        response_tx: mpsc::Sender<Response>,
    ) {
        let client = self.client.clone();
        let model = self.model.clone();
        let preamble = self.preamble.clone();
        let temperature = self.temperature;

        tokio::spawn(async move {
            let agent = client
                .agent(&model)
                .preamble(&preamble)
                .temperature(temperature)
                .build();

            let mut chat_history: Vec<Message> = vec![];

            while let Some(req) = prompt_rx.recv().await {
                match req {
                    Request::Prompt(prompt) => {
                        let relevant = match knowledge
                            .search_multi(&prompt, Namespace::searchable(), 4)
                            .await
                        {
                            Ok(results) => {
                                let filtered: Vec<_> = results.into_iter()
                                    .filter(|r| r.score > 0.25)
                                    .collect();
                                if !filtered.is_empty() {
                                    let info: Vec<(f64, String)> = filtered.iter()
                                        .map(|r| {
                                            let preview = r.content.chars().take(120).collect::<String>();
                                            (r.score, format!("[{}] {}", r.namespace, preview))
                                        })
                                        .collect();
                                    let _ = response_tx.send(Response::ContextFound(info)).await;
                                }
                                filtered
                            }
                            Err(e) => {
                                let _ = response_tx
                                    .send(Response::ContextFound(vec![(0.0, format!("KB error: {e}"))]))
                                    .await;
                                vec![]
                            }
                        };

                        // Build prompt — inject context only if we found something
                        let final_prompt = if relevant.is_empty() {
                            prompt
                        } else {
                            let ctx = relevant.iter()
                                .map(|r| r.content.as_str())
                                .collect::<Vec<_>>()
                                .join("\n---\n");
                            format!("Relevant context:\n{ctx}\n\nUser: {prompt}")
                        };

                        match agent.chat(&final_prompt, &mut chat_history).await {
                            Ok(text) => {
                                let _ = response_tx.send(Response::CompleteResponse(text)).await;
                            }
                            Err(e) => {
                                eprintln!("{:?}", e);
                            }
                        }
                    }
                    Request::Ingest => {
                        // spawn sub-task so we don't block the loop
                        let kb = knowledge.clone();
                        let client_clone = client.clone();
                        let tx = response_tx.clone();
                        tokio::spawn(async move {
                            // read dir, classify_text, ingest_file, send Status updates
                            let _ = tx.send(Response::Status("Ingestion complete".into())).await;
                        });
                    }
                }
            }
        });
    }
}
