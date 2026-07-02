use color_eyre::Result;

use rig::client::{CompletionClient, EmbeddingsClient, ProviderClient};
use rig::completion::Prompt;
use rig::providers::gemini::completion::gemini_api_types::Modality::Document;
use rig::providers::ollama::Client;
use rig::{Embed, vector_store};

use rig::embeddings::{EmbeddingModel, EmbeddingsBuilder};
use rig::vector_store::in_memory_store::InMemoryVectorStore;
use serde::Serialize;

use std::path::{Path, PathBuf};
use std::{io, vec};
use tokio::fs::{self, DirEntry};
use tokio::sync::mpsc;

use crate::events::*;

#[derive(Embed, Serialize, Clone, Debug, Default, PartialEq, Eq)]
struct Chunk {
    id: PathBuf,
    #[embed]
    content: String,
}

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

    pub fn handle_completion(
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

    async fn chunk_input(text: &str, max_chars: usize) -> Vec<String> {
        text.split("\n\n")
            .flat_map(|paragraph| {
                if paragraph.len() <= max_chars {
                    vec![paragraph.to_string()]
                } else {
                    paragraph
                        .chars()
                        .collect::<Vec<_>>()
                        .chunks(max_chars)
                        .map(|c| c.iter().collect::<String>())
                        .collect()
                }
            })
            .filter(|s| !s.trim().is_empty())
            .collect()
    }

    pub async fn ingest_embeddings(self) -> Result<()> {
        tokio::spawn(async move {
            let mut chunks: Vec<Chunk> = vec![];

            let inputs_dir = Path::new("/inputs");
            if let Ok(mut entries) = fs::read_dir(inputs_dir).await {
                while let Ok(Some(entry)) = entries.next_entry().await {
                    if let Ok(text) = fs::read_to_string(entry.path()).await {
                        let raw_chunks = Self::chunk_input(&text, 1000).await;

                        for chunk_text in raw_chunks {
                            chunks.push(Chunk {
                                id: entry.path(),
                                content: chunk_text,
                            });
                        }
                    }
                }
            }

            let embedding_model = self.client.embedding_model(&self.model);

            let embeddings = EmbeddingsBuilder::new(embedding_model)
                .documents(chunks)?
                .build()
                .await?;

            let vector_store = InMemoryVectorStore::from_documents(embeddings);
            Ok::<(), color_eyre::eyre::Error>(())
        });

        Ok(())
    }
}
