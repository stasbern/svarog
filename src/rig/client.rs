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
                        let classifier = client
                            .agent(&model)
                            .preamble(Namespace::classifier_prompt())
                            .temperature(0.0)
                            .build();

                        let dir = std::path::PathBuf::from("./input");
                        let stored_hashes = knowledge.get_all_hashes().await.unwrap_or_default();

                        match tokio::fs::read_dir(&dir).await {
                            Ok(mut entries) => {
                                while let Ok(Some(entry)) = entries.next_entry().await {
                                    let Ok(text) = tokio::fs::read_to_string(entry.path()).await else {
                                        continue;
                                    };
                                    let fname = entry.file_name().to_string_lossy().to_string();
                                    let path = entry.path().to_string_lossy().replace('\\', "/");
                                    let hash = KnowledgeBase::hash_content(&text);

                                    // Skip unchanged files
                                    if let Some((_, stored_hash)) = stored_hashes.get(&path) {
                                        if *stored_hash == hash {
                                            let _ = response_tx.send(Response::Status(
                                                format!("{fname} unchanged, skipping")
                                            )).await;
                                            continue;
                                        }
                                    }

                                    // Classify via the single classifier agent
                                    let preview: String = text.chars().take(800).collect();
                                    let ns = match classifier.prompt(&preview).await {
                                        Ok(resp) => Namespace::parse(&resp),
                                        Err(_) => Namespace::Factual,
                                    };

                                    let _ = response_tx.send(Response::Status(
                                        format!("{fname} → {ns}")
                                    )).await;

                                    // Clean old namespace if it changed
                                    if let Some((old_ns_str, _)) = stored_hashes.get(&path) {
                                        let old_ns = Namespace::parse(old_ns_str);
                                        if old_ns != ns {
                                            let _ = knowledge.delete_chunks_for_file(&path, old_ns).await;
                                        }
                                    }

                                    if let Err(e) = knowledge.ingest_file(&path, &text, ns).await {
                                        let _ = response_tx.send(Response::Status(
                                            format!("Failed {fname}: {e}")
                                        )).await;
                                    }
                                }
                                let _ = response_tx.send(Response::Status("Ingestion complete".into())).await;
                            }
                            Err(e) => {
                                let _ = response_tx.send(Response::Status(
                                    format!("Failed to read input dir: {e}")
                                )).await;
                            }
                        }
                    }
                }
            }
        });
    }
}
