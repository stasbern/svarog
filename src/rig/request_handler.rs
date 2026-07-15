// src/rig/request_handler.rs

use tokio::sync::mpsc;

use super::chat_service::ChatService;
use super::ingestion_service::IngestionService;
use crate::events::Request;

pub fn spawn_request_handler(
    mut request_rx: mpsc::Receiver<Request>,
    mut chat_service: ChatService,
    ingestion_service: IngestionService,
) {
    tokio::spawn(async move {
        while let Some(request) = request_rx.recv().await {
            match request {
                Request::Prompt(prompt) => {
                    chat_service.answer(prompt).await;
                }

                Request::Ingest => {
                    ingestion_service.ingest_default_directory().await;
                }
            }
        }
    });
}
