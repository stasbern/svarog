// src/rig/chat_service.rs

use std::sync::Arc;

use rig::completion::{Chat, Message};
use rig::providers::ollama::CompletionModel;
use tokio::sync::mpsc;

use super::knowledge::KnowledgeBase;
use super::knowledge_source::Namespace;
use crate::events::Response;

const DOCUMENT_MATCH_LIMIT: usize = 3;
const DOCUMENT_SCORE_THRESHOLD: f64 = 0.20;

const PASSAGE_MATCH_LIMIT: u64 = 6;
const PASSAGE_SCORE_THRESHOLD: f64 = 0.10;

pub struct ChatService {
    agent: rig::agent::Agent<CompletionModel>,
    knowledge: Arc<KnowledgeBase>,
    response_tx: mpsc::Sender<Response>,
    chat_history: Vec<Message>,
}

impl ChatService {
    pub fn new(
        agent: rig::agent::Agent<CompletionModel>,
        knowledge: Arc<KnowledgeBase>,
        response_tx: mpsc::Sender<Response>,
    ) -> Self {
        Self {
            agent,
            knowledge,
            response_tx,
            chat_history: Vec::new(),
        }
    }

    pub async fn answer(&mut self, prompt: String) {
        let document_matches = self.find_documents(&prompt).await;
        let passages = self.find_passages(&prompt).await;

        self.report_document_matches(&document_matches).await;
        self.report_passage_matches(&passages).await;

        let final_prompt = build_grounded_prompt(&prompt, &document_matches, &passages);

        match self.agent.chat(&final_prompt, &mut self.chat_history).await {
            Ok(answer) => {
                let _ = self
                    .response_tx
                    .send(Response::CompleteResponse(answer))
                    .await;
            }

            Err(error) => {
                let _ = self
                    .response_tx
                    .send(Response::Error(format!("{error:?}")))
                    .await;
            }
        }
    }

    async fn find_documents(&self, prompt: &str) -> Vec<super::document::DocumentSearchResult> {
        match self
            .knowledge
            .search_documents(prompt, DOCUMENT_MATCH_LIMIT)
            .await
        {
            Ok(results) => results
                .into_iter()
                .filter(|result| result.score >= DOCUMENT_SCORE_THRESHOLD)
                .collect(),

            Err(error) => {
                self.report_kb_error("Document catalog search failed", error)
                    .await;

                Vec::new()
            }
        }
    }

    async fn find_passages(&self, prompt: &str) -> Vec<super::knowledge::SearchResult> {
        match self
            .knowledge
            .search_multi(prompt, Namespace::searchable(), PASSAGE_MATCH_LIMIT)
            .await
        {
            Ok(results) => results
                .into_iter()
                .filter(|result| result.score >= PASSAGE_SCORE_THRESHOLD)
                .collect(),

            Err(error) => {
                self.report_kb_error("Passage search failed", error).await;

                Vec::new()
            }
        }
    }

    async fn report_document_matches(&self, matches: &[super::document::DocumentSearchResult]) {
        if matches.is_empty() {
            return;
        }

        let contexts = matches
            .iter()
            .map(|result| {
                (
                    result.score,
                    format!(
                        "[document] {} — {}",
                        result.document.descriptor.title, result.document.descriptor.summary,
                    ),
                )
            })
            .collect();

        let _ = self
            .response_tx
            .send(Response::ContextFound(contexts))
            .await;
    }

    async fn report_passage_matches(&self, matches: &[super::knowledge::SearchResult]) {
        if matches.is_empty() {
            return;
        }

        let contexts = matches
            .iter()
            .map(|result| {
                let preview = result.content.chars().take(120).collect::<String>();

                (
                    result.score,
                    format!(
                        "[{}] {} — {}",
                        result.namespace,
                        result.source_label(),
                        preview,
                    ),
                )
            })
            .collect();

        let _ = self
            .response_tx
            .send(Response::ContextFound(contexts))
            .await;
    }

    async fn report_kb_error(&self, operation: &str, error: color_eyre::eyre::Report) {
        let message = format!("{operation}: {error}");

        let _ = self
            .response_tx
            .send(Response::ContextFound(vec![(0.0, message)]))
            .await;
    }
}

fn build_grounded_prompt(
    user_prompt: &str,
    document_matches: &[super::document::DocumentSearchResult],
    passages: &[super::knowledge::SearchResult],
) -> String {
    let catalog_context = document_matches
        .iter()
        .map(|result| result.document.catalog_context())
        .collect::<Vec<_>>()
        .join("\n\n--- DOCUMENT ---\n\n");

    let passage_context = passages
        .iter()
        .map(|result| format!("[Source: {}]\n{}", result.source_label(), result.content,))
        .collect::<Vec<_>>()
        .join("\n\n--- PASSAGE ---\n\n");

    if catalog_context.is_empty() && passage_context.is_empty() {
        return user_prompt.to_string();
    }

    format!(
        r#"Knowledge catalog:
{catalog_context}

Retrieved source passages:
{passage_context}

Instructions:
- The catalog describes which documents and subjects are available.
- The passages contain evidence extracted from those documents.
- For questions about available knowledge, answer primarily from the catalog.
- For technical claims, rely on the retrieved passages.
- Do not treat generated catalog metadata as proof of unsupported technical details.
- If the available context is insufficient, say so.
- When useful, cite the document title and physical page number supplied with a passage.

User: {user_prompt}"#
    )
}

#[cfg(test)]
mod tests {
    use super::build_grounded_prompt;

    #[test]
    fn empty_context_returns_original_prompt() {
        let prompt = build_grounded_prompt("Hello", &[], &[]);

        assert_eq!(prompt, "Hello");
    }
}
