// src/rig/retrieval.rs

use std::collections::HashMap;
use std::sync::Arc;

use color_eyre::Result;

use super::knowledge::{KnowledgeBase, SearchResult};
use super::knowledge_source::Namespace;

const VECTOR_CANDIDATES: usize = 24;
const FULL_TEXT_CANDIDATES: usize = 24;
const RRF_K: f64 = 60.0;

pub struct RetrievalService {
    knowledge: Arc<KnowledgeBase>,
}

impl RetrievalService {
    pub fn new(knowledge: Arc<KnowledgeBase>) -> Self {
        Self { knowledge }
    }

    pub async fn retrieve(
        &self,
        query: &str,
        namespaces: &[Namespace],
        limit: usize,
    ) -> Result<Vec<SearchResult>> {
        let (vector_results, full_text_results) = tokio::try_join!(
            self.knowledge
                .search_vector(query, namespaces, VECTOR_CANDIDATES,),
            self.knowledge
                .search_full_text(query, namespaces, FULL_TEXT_CANDIDATES,),
        )?;

        Ok(reciprocal_rank_fusion(
            vector_results,
            full_text_results,
            limit,
        ))
    }
}

fn reciprocal_rank_fusion(
    vector_results: Vec<SearchResult>,
    full_text_results: Vec<SearchResult>,
    limit: usize,
) -> Vec<SearchResult> {
    let mut fused: HashMap<String, (SearchResult, f64)> = HashMap::new();

    add_ranking(&mut fused, vector_results);
    add_ranking(&mut fused, full_text_results);

    // Maximum possible score occurs when a result ranks first
    // in both retrieval lists. Normalizing produces a convenient
    // 0–1 display score. It is a ranking score, not confidence.
    let maximum_score = 2.0 / (RRF_K + 1.0);

    let mut results = fused
        .into_values()
        .map(|(mut result, score)| {
            result.score = score / maximum_score;
            result
        })
        .collect::<Vec<_>>();

    results.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    results.truncate(limit);
    results
}

fn add_ranking(fused: &mut HashMap<String, (SearchResult, f64)>, results: Vec<SearchResult>) {
    for (index, result) in results.into_iter().enumerate() {
        let rank = index as f64 + 1.0;
        let contribution = 1.0 / (RRF_K + rank);

        let entry = fused
            .entry(result.id.clone())
            .or_insert_with(|| (result, 0.0));

        entry.1 += contribution;
    }
}

#[cfg(test)]
mod tests {
    use super::reciprocal_rank_fusion;
    use crate::rig::knowledge::SearchResult;
    use crate::rig::knowledge_source::Namespace;

    fn result(id: &str) -> SearchResult {
        SearchResult {
            score: 0.0,
            id: id.to_string(),
            document_id: "document:test".into(),
            document_key: "test".into(),
            document_title: "Test".into(),
            content: id.to_string(),
            namespace: Namespace::Factual,
            chunk_index: 0,
            page_start: 1,
            page_end: 1,
        }
    }

    #[test]
    fn result_in_both_lists_ranks_first() {
        let vector = vec![result("semantic-only"), result("both")];

        let full_text = vec![result("exact-only"), result("both")];

        let fused = reciprocal_rank_fusion(vector, full_text, 3);

        assert_eq!(fused[0].id, "both");
    }

    #[test]
    fn result_ids_are_deduplicated() {
        let fused = reciprocal_rank_fusion(vec![result("same")], vec![result("same")], 10);

        assert_eq!(fused.len(), 1);
    }
}
