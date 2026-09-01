mod debug_cosine;
mod embedding;
mod hybrid;
mod rerank;
mod tfidf;
mod types;

pub use types::RankedTool;

use self::embedding::EmbeddingIndex;
use self::hybrid::{expand_query, reciprocal_rank_fusion};
use self::rerank::{rerank_score, QueryFeatures, RerankWeights};
use crate::backend::Tool;
use crate::health::HealthManager;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use tfidf::TfIdfIndex;
use tracing::{debug, info, warn};

pub struct SemanticRouter {
    index: Arc<TfIdfIndex>,
    embedding: Arc<EmbeddingIndex>,
    embedding_enabled: bool,
    health_manager: HealthManager,
    rerank_weights: RerankWeights,
    /// tool_id -> input_schema, stashed at index time for rerank features.
    schemas: Arc<RwLock<HashMap<String, Value>>>,
    /// Servers used recently in this process (session coherence signal).
    last_servers: Arc<RwLock<Vec<String>>>,
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
            rerank_weights: RerankWeights::default(),
            schemas: Arc::new(RwLock::new(HashMap::new())),
            last_servers: Arc::new(RwLock::new(Vec::new())),
        }
    }

    pub async fn index_tools(&self, tools: Vec<Tool>) {
        debug!("Indexing {} tools for semantic search", tools.len());
        // Stash schemas for the rerank pass before moving the vec along.
        {
            let mut schemas = self.schemas.write().unwrap();
            schemas.clear();
            for t in &tools {
                schemas.insert(t.tool_id.clone(), t.input_schema.clone());
            }
        }
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

        // Feature rerank: blend the RRF rank with interpretable features
        // (name overlap, description/param coverage, server coherence) to
        // suppress confident-but-wrong semantic hits. Runs BEFORE the final
        // sort so the health formula above and the tie-break below both see
        // the reranked final_score.
        {
            let qf = QueryFeatures::new(query);
            let schemas = self.schemas.read().unwrap();
            let last_servers = self.last_servers.read().unwrap();
            let n = candidates.len().max(1) as f64;
            for (i, cand) in candidates.iter_mut().enumerate() {
                // Normalized rank: RRF position mapped to 0..1 (best=1).
                let norm_rank = 1.0 - (i as f64 / n);
                let schema = schemas.get(&cand.tool_id).cloned().unwrap_or(Value::Null);
                let input = rerank::RerankInput {
                    tool_name: cand.tool_name.clone(),
                    description: cand.description.clone(),
                    schema,
                    server_name: cand.server_name.clone(),
                };
                let features = input.features(&qf);
                let same_server = last_servers.iter().any(|s| s == &cand.server_name);
                cand.final_score =
                    rerank_score(norm_rank, &features, same_server, &self.rerank_weights);
            }
            debug!("rerank applied over {} candidates", candidates.len());
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

        // Record servers of the returned tools for the session-coherence
        // feature on subsequent searches.
        {
            let mut last = self.last_servers.write().unwrap();
            last.clear();
            for c in candidates.iter().take(top_k) {
                if !c.server_name.is_empty() && !last.contains(&c.server_name) {
                    last.push(c.server_name.clone());
                }
            }
        }

        candidates
            .into_iter()
            .filter(|c| c.final_score > 0.0)
            .take(top_k)
            .collect()
    }

    /// Fallback-2 support: name-only listing of every indexed tool, grouped
    /// by server. This is the deterministic escape hatch when semantic+lexical
    /// retrieval both fail -- ~800 tokens for 53 tools, vs 7.6K for full
    /// schemas. Governance/audit still apply on the subsequent invoke.
    pub async fn server_overview(&self) -> Vec<(String, Vec<String>)> {
        let mut groups: Vec<(String, Vec<String>)> = Vec::new();
        let docs = self.schemas.read().unwrap();
        let mut servers: Vec<String> = docs
            .keys()
            .filter_map(|id| id.split("::").next().map(|s| s.to_string()))
            .collect();
        servers.sort();
        servers.dedup();
        for server in servers {
            let mut names: Vec<String> = docs
                .keys()
                .filter(|id| id.starts_with(&format!("{}::", server)))
                .filter_map(|id| id.split("::").nth(1).map(|s| s.to_string()))
                .collect();
            names.sort();
            groups.push((server, names));
        }
        groups
    }

    /// Fallback-1 signal: is the top candidate trustworthy?
    ///
    /// Primary signal = lexical corroboration: re-run the lexical lane on the
    /// ORIGINAL query and require the top fused candidate to appear there.
    /// The RRF fusion overwrites semantic_score with the fusion score, so a
    /// nonzero value cannot prove the *lexical* lane matched (the embedding
    /// lane also contributes). An embedding-only hit (no lexical
    /// corroboration) is exactly the "confidently wrong" risk this flags.
    pub async fn low_confidence(&self, results: &[RankedTool], query: &str) -> bool {
        if results.is_empty() {
            return true;
        }
        let top = &results[0];
        !self.lexical_corroborated(top, query)
    }

    /// Does the lexical lane (re-run on the original query) return this tool?
    fn lexical_corroborated(&self, top: &RankedTool, query: &str) -> bool {
        self.index
            .search(query, 5)
            .iter()
            .any(|t| t.tool_id == top.tool_id)
    }
}
