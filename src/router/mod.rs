mod embedding;
mod hybrid;
mod tfidf;
mod types;

pub use types::RankedTool;

use self::embedding::EmbeddingIndex;
use self::hybrid::{expand_query, reciprocal_rank_fusion};
use crate::backend::Tool;
use crate::health::HealthManager;
use std::sync::Arc;
use tfidf::TfIdfIndex;
use tracing::{debug, info, warn};

pub struct SemanticRouter {
    index: Arc<TfIdfIndex>,
    embedding: Arc<EmbeddingIndex>,
    embedding_enabled: bool,
    health_manager: HealthManager,
}

impl SemanticRouter {
    pub fn new(health_manager: HealthManager) -> Self {
        Self {
            index: Arc::new(TfIdfIndex::new()),
            embedding: Arc::new(EmbeddingIndex::new()),
            embedding_enabled: std::env::var("SENTINEL_EMBEDDING")
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(false),
            health_manager,
        }
    }

    pub async fn index_tools(&self, tools: Vec<Tool>) {
        debug!("Indexing {} tools for semantic search", tools.len());
        self.index.build_index(tools.clone());
        debug!("TF-IDF index built successfully");

        if self.embedding_enabled {
            let emb = Arc::clone(&self.embedding);
            // Block on build (startup path); fastembed downloads the model on
            // first call (~100MB) then caches it.
            let n = tokio::task::spawn_blocking(move || emb.build_index(&tools)).await;
            match n {
                Ok(Ok(n)) => info!("Embedding index built: {} tools", n),
                other => warn!(
                    "Embedding index build failed ({:?}) -- running TF-IDF only",
                    other
                        .err()
                        .map(|e| e.to_string())
                        .or(Some("join error".into()))
                ),
            }
        } else {
            debug!("Embedding lane disabled (SENTINEL_EMBEDDING not set)");
        }
    }

    pub async fn search(&self, query: &str, top_k: usize, health_weight: f64) -> Vec<RankedTool> {
        debug!(query = %query, top_k = top_k, "Searching tools");

        // Keep an exact lexical ranking and add a synonym-expanded ranking. RRF combines
        // rank positions, so the retrievers do not need comparable score scales.
        let lexical = self.index.search(query, top_k * 4);
        let expanded_query = expand_query(query);
        let expanded = self.index.search(&expanded_query, top_k * 4);

        // Semantic lane (optional). Embedding sees the same enriched corpus
        // as the lexical lane so RRF compares like rankings.
        let mut rankings = vec![
            lexical
                .into_iter()
                .map(|tool| (tool.tool_id.clone(), tool))
                .collect::<Vec<_>>(),
            expanded
                .into_iter()
                .map(|tool| (tool.tool_id.clone(), tool))
                .collect::<Vec<_>>(),
        ];
        if self.embedding_enabled && !self.embedding.is_empty() {
            let emb = Arc::clone(&self.embedding);
            let q = query.to_string();
            let k = top_k * 4;
            // OFF the async hot path: ONNX encode is CPU-bound.
            match tokio::task::spawn_blocking(move || emb.search_ranked(&q, k)).await {
                Ok(Ok(scored)) if !scored.is_empty() => {
                    debug!("semantic lane returned {} candidates", scored.len());
                    rankings.push(scored);
                }
                Ok(Err(e)) => warn!("semantic lane error: {}", e),
                Err(e) => warn!("semantic lane join error: {}", e),
                _ => {}
            }
        }

        let mut candidates = reciprocal_rank_fusion(rankings, 60.0)
            .into_iter()
            .map(|(mut tool, fusion_score)| {
                tool.semantic_score = fusion_score;
                tool.final_score = fusion_score;
                tool
            })
            .collect::<Vec<_>>();

        // Enrich with health scores
        for candidate in &mut candidates {
            if let Some(health_score) = self
                .health_manager
                .get_health_score(&candidate.tool_id)
                .await
            {
                candidate.health_score = health_score.health_score;
                candidate.degraded = health_score.degraded;

                // Apply health penalty to final score
                let health_penalty = if health_score.degraded {
                    0.1 // Heavily penalize degraded tools
                } else {
                    health_score.health_score
                };

                candidate.final_score = candidate.semantic_score
                    * (1.0 - health_weight + health_weight * health_penalty);

                // Filter out zombie tools
                if health_score.zombie {
                    candidate.final_score = 0.0;
                }
            }
        }

        // Re-sort by final score and take top_k. Tie-break: when scores are
        // (near-)equal, prefer tools whose NAME contains a query token --
        // previously equal scores left ordering to hash-iteration randomness,
        // which is why Chinese queries scored R@1=0% despite recall@5 hits.
        {
            let query_tokens: Vec<String> = TfIdfIndex::tokenize(query);
            candidates.sort_by(|a, b| {
                let ord = b
                    .final_score
                    .partial_cmp(&a.final_score)
                    .unwrap_or(std::cmp::Ordering::Equal);
                if ord != std::cmp::Ordering::Equal {
                    return ord;
                }
                let a_name_hit = query_tokens
                    .iter()
                    .any(|t| a.tool_name.to_lowercase().contains(t));
                let b_name_hit = query_tokens
                    .iter()
                    .any(|t| b.tool_name.to_lowercase().contains(t));
                b_name_hit.cmp(&a_name_hit)
            });
        }

        candidates
            .into_iter()
            .filter(|c| c.final_score > 0.0)
            .take(top_k)
            .collect()
    }
}
