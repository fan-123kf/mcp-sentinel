mod hybrid;
mod tfidf;
mod types;

pub use types::RankedTool;

use crate::backend::Tool;
use crate::health::HealthManager;
use self::hybrid::{expand_query, reciprocal_rank_fusion};
use std::sync::Arc;
use tfidf::TfIdfIndex;
use tracing::debug;

pub struct SemanticRouter {
    index: Arc<TfIdfIndex>,
    health_manager: HealthManager,
}

impl SemanticRouter {
    pub fn new(health_manager: HealthManager) -> Self {
        Self {
            index: Arc::new(TfIdfIndex::new()),
            health_manager,
        }
    }

    pub async fn index_tools(&self, tools: Vec<Tool>) {
        debug!("Indexing {} tools for semantic search", tools.len());
        self.index.build_index(tools);
        debug!("TF-IDF index built successfully");
    }

    pub async fn search(&self, query: &str, top_k: usize, health_weight: f64) -> Vec<RankedTool> {
        debug!(query = %query, top_k = top_k, "Searching tools");

        // Keep an exact lexical ranking and add a synonym-expanded ranking. RRF combines
        // rank positions, so the two retrievers do not need comparable score scales.
        let lexical = self.index.search(query, top_k * 4);
        let expanded_query = expand_query(query);
        let expanded = self.index.search(&expanded_query, top_k * 4);
        let mut candidates = reciprocal_rank_fusion(
            vec![
                lexical
                    .into_iter()
                    .map(|tool| (tool.tool_id.clone(), tool))
                    .collect(),
                expanded
                    .into_iter()
                    .map(|tool| (tool.tool_id.clone(), tool))
                    .collect(),
            ],
            60.0,
        )
        .into_iter()
        .map(|(mut tool, fusion_score)| {
            tool.semantic_score = fusion_score;
            tool.final_score = fusion_score;
            tool
        })
        .collect::<Vec<_>>();

        // Enrich with health scores
        for candidate in &mut candidates {
            if let Some(health_score) = self.health_manager.get_health_score(&candidate.tool_id).await {
                candidate.health_score = health_score.health_score;
                candidate.degraded = health_score.degraded;
                
                // Apply health penalty to final score
                let health_penalty = if health_score.degraded {
                    0.1 // Heavily penalize degraded tools
                } else {
                    health_score.health_score
                };
                
                candidate.final_score = candidate.semantic_score * 
                    (1.0 - health_weight + health_weight * health_penalty);

                // Filter out zombie tools
                if health_score.zombie {
                    candidate.final_score = 0.0;
                }
            }
        }

        // Re-sort by final score and take top_k
        candidates.sort_by(|a, b| {
            b.final_score
                .partial_cmp(&a.final_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        candidates
            .into_iter()
            .filter(|c| c.final_score > 0.0)
            .take(top_k)
            .collect()
    }
}
