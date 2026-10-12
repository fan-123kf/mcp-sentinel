//! Hybrid retrieval pipeline:
//!
//!   1. BGE-M3 **sparse** lane   (learned lexical, replaces TF-IDF + synonyms)
//!   2. BGE-M3 **dense** lane    (semantic cosine)
//!   3. **Reciprocal Rank Fusion** of both rankings (k=60)
//!   4. **Cross-Encoder** rerank  (bge-reranker-v2-m3) over the fused top-N
//!   5. Health-aware penalty + zombie filter
//!
//! All embedding work happens off the async hot path via spawn_blocking,
//! since ONNX encode is CPU-bound. The cross-encoder is optional -- if the
//! model files aren't provisioned, we keep RRF order and log a warning so
//! the gateway still serves traffic.

#[cfg(feature = "models")]
mod cross_encoder;
#[cfg(feature = "models")]
mod embedding;
mod hybrid;
#[cfg(not(feature = "models"))]
mod model_stubs;
#[cfg(test)]
mod rerank;
#[cfg(test)]
mod simulation_test;
mod tfidf;
pub mod types;

pub use types::RankedTool;

use self::hybrid::reciprocal_rank_fusion;
#[cfg(not(feature = "models"))]
use self::model_stubs::{CrossEncoder, EmbeddingIndex};
#[cfg(feature = "models")]
use self::{cross_encoder::CrossEncoder, embedding::EmbeddingIndex};
use crate::backend::Tool;
use crate::health::HealthManager;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use tfidf::TfIdfIndex;
use tracing::{debug, info, warn};

/// Size of the candidate pool fed into the cross-encoder. Beyond ~20 the
/// cost/benefit curve flattens for our ~50-tool corpus.
const RERANK_CANDIDATE_POOL: usize = 20;

pub struct SemanticRouter {
    /// BGE-M3 hybrid index (dense + sparse from one forward pass).
    embedding: Arc<EmbeddingIndex>,
    /// Optional cross-encoder reranker; lazily initialized.
    cross_encoder: Arc<CrossEncoder>,
    /// Legacy TF-IDF kept as a final graceful-degradation lane when BGE-M3
    /// model files aren't provisioned. Enabled when `SENTINEL_LEGACY_TFIDF=1`
    /// or automatically when the embedding index reports empty.
    fallback_tfidf: Arc<TfIdfIndex>,
    /// Whether to attempt the embedding lane at all. When false we go
    /// straight to the TF-IDF lane (current default until the model is
    /// provisioned).
    embedding_enabled: bool,
    health_manager: HealthManager,
    /// tool_id -> input_schema, stashed at index time for the legacy
    /// feature-rerank path (kept so old behavior survives when neither
    /// embedding nor cross-encoder is available).
    schemas: Arc<RwLock<HashMap<String, Value>>>,
    /// Servers used recently in this process (session coherence signal).
    last_servers: Arc<RwLock<Vec<String>>>,
    /// All known tools, kept for the cross-encoder to rebuild the document
    /// text per candidate.
    tools: Arc<RwLock<Vec<Tool>>>,
    last_strategy: RwLock<String>,
}

impl SemanticRouter {
    pub fn new(health_manager: HealthManager) -> Self {
        let embedding_enabled = cfg!(feature = "models")
            && std::env::var("SENTINEL_EMBEDDING")
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(false);
        Self {
            embedding: Arc::new(EmbeddingIndex::new()),
            cross_encoder: Arc::new(CrossEncoder::new()),
            fallback_tfidf: Arc::new(TfIdfIndex::new()),
            embedding_enabled,
            health_manager,
            schemas: Arc::new(RwLock::new(HashMap::new())),
            last_servers: Arc::new(RwLock::new(Vec::new())),
            tools: Arc::new(RwLock::new(Vec::new())),
            last_strategy: RwLock::new("not_searched".into()),
        }
    }

    pub fn strategy(&self) -> String {
        self.last_strategy.read().unwrap().clone()
    }

    pub fn with_embedding(mut self, enabled: bool) -> Self {
        self.embedding_enabled = enabled && cfg!(feature = "models");
        self
    }

    pub fn require_models(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.embedding_enabled && !self.embedding.is_empty(),
            "BGE-M3 unavailable; cannot run model evaluation"
        );
        self.cross_encoder.verify()?;
        Ok(())
    }

    pub fn tool_count(&self) -> usize {
        self.tools.read().unwrap().len()
    }

    pub async fn index_tools(&self, tools: Vec<Tool>) {
        let tools: Vec<_> = tools
            .into_iter()
            .filter(|t| !crate::security::scan_tool(t).blocked)
            .collect();
        debug!("Indexing {} tools for hybrid search", tools.len());

        // Stash schemas and full tool list before moving on.
        {
            let mut schemas = self.schemas.write().unwrap();
            schemas.clear();
            for t in &tools {
                schemas.insert(t.tool_id.clone(), t.input_schema.clone());
            }
        }
        *self.tools.write().unwrap() = tools.clone();

        // Always rebuild the TF-IDF index so the legacy lane stays warm as
        // a safety net.
        self.fallback_tfidf.build_index(tools.clone());
        debug!("TF-IDF fallback index built");

        if self.embedding_enabled {
            let emb = Arc::clone(&self.embedding);
            // Block on build (startup path); fastembed downloads the model
            // on first call (~600MB for BGE-M3) then caches it.
            let n = tokio::task::spawn_blocking(move || emb.build_index(&tools)).await;
            match n {
                Ok(Ok(n)) => info!("BGE-M3 hybrid index built: {} tools", n),
                other => {
                    warn!(
                        "BGE-M3 index build failed ({:?}) -- falling back to TF-IDF",
                        other
                            .err()
                            .map(|e| e.to_string())
                            .or(Some("join error".into()))
                    );
                }
            }
        } else {
            debug!("Hybrid embedding lane disabled (SENTINEL_EMBEDDING not set)");
        }
    }

    pub async fn search_with_metadata(
        &self,
        query: &str,
        top_k: usize,
        health_weight: f64,
        server: Option<&str>,
    ) -> (Vec<RankedTool>, String) {
        let retrieval_k = if server.is_some() {
            self.tool_count()
        } else {
            top_k.saturating_mul(4).max(RERANK_CANDIDATE_POOL)
        };
        debug!(query = %query, top_k = top_k, "Searching tools");

        // --- Stage 1+2: dense + sparse lanes via BGE-M3 -------------------
        let rankings = if self.embedding_enabled && !self.embedding.is_empty() {
            let emb = Arc::clone(&self.embedding);
            let q = query.to_string();
            let k = retrieval_k;
            let (dense_res, sparse_res) = tokio::task::spawn_blocking(move || {
                let dense = emb.search_dense(&q, k);
                let sparse = emb.search_sparse(&q, k);
                (dense, sparse)
            })
            .await
            .unwrap_or_else(|e| {
                warn!("hybrid lane join error: {}", e);
                (
                    Err(anyhow::anyhow!("join error")),
                    Err(anyhow::anyhow!("join error")),
                )
            });

            let mut rankings = Vec::new();
            if let Ok(dense) = dense_res {
                if !dense.is_empty() {
                    rankings.push(dense.into_iter().collect::<Vec<_>>());
                }
            } else if let Err(e) = dense_res {
                warn!("dense lane error: {}", e);
            }
            if let Ok(sparse) = sparse_res {
                if !sparse.is_empty() {
                    rankings.push(sparse.into_iter().collect::<Vec<_>>());
                }
            } else if let Err(e) = sparse_res {
                warn!("sparse lane error: {}", e);
            }
            rankings
        } else {
            Vec::new()
        };

        let used_embedding = rankings.len();
        let rankings: Vec<_> = rankings
            .into_iter()
            .map(|lane| {
                lane.into_iter()
                    .filter(|(_, t)| server.is_none_or(|s| t.server_name.eq_ignore_ascii_case(s)))
                    .collect()
            })
            .collect();
        let mut used_cross_encoder = false;

        // --- Stage 3: RRF fusion (or fall back to TF-IDF if both lanes
        // failed) ----------------------------------------------------------
        let mut candidates: Vec<RankedTool> = if rankings.is_empty() {
            // TF-IDF-only path: single ranking, no fusion needed.
            self.fallback_tfidf
                .search(query, retrieval_k)
                .into_iter()
                .filter(|t| server.is_none_or(|s| t.server_name.eq_ignore_ascii_case(s)))
                .map(|mut t| {
                    t.final_score = t.semantic_score;
                    t
                })
                .collect()
        } else {
            let fused = reciprocal_rank_fusion(rankings, 60.0);
            fused
                .into_iter()
                .map(|(mut tool, fusion_score)| {
                    tool.semantic_score = fusion_score;
                    tool.final_score = fusion_score;
                    tool
                })
                .collect()
        };

        // Cap to the rerank pool size -- cross-encoder is the most expensive
        // step so we don't want to feed it more than we need.
        candidates.sort_by(|a, b| {
            b.final_score
                .partial_cmp(&a.final_score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.tool_id.cmp(&b.tool_id))
        });
        candidates.truncate(top_k.max(RERANK_CANDIDATE_POOL));

        // --- Stage 4: Cross-Encoder rerank --------------------------------
        if !candidates.is_empty() && self.cross_encoder.is_available() {
            let ce = Arc::clone(&self.cross_encoder);
            let q = query.to_string();
            let pool: Vec<RankedTool> = candidates.clone();
            let tools_snapshot = self.tools.read().unwrap().clone();
            let result =
                tokio::task::spawn_blocking(move || ce.rerank(&q, pool, &tools_snapshot)).await;
            match result {
                Ok(Ok(reranked)) if !reranked.is_empty() => {
                    used_cross_encoder = true;
                    debug!("cross-encoder rerank returned {} results", reranked.len());
                    candidates = reranked;
                }
                Ok(Ok(_)) => {}
                Ok(Err(e)) => {
                    warn!("cross-encoder rerank error: {} -- keeping RRF order", e);
                }
                Err(e) => warn!("cross-encoder join error: {}", e),
            }
        }

        let strategy: String = match (used_embedding, used_cross_encoder) {
            (2, true) => "bge_m3_dense_sparse_rrf_cross_encoder",
            (2, false) => "bge_m3_dense_sparse_rrf",
            (0, true) => "tfidf_cross_encoder",
            (0, false) => "tfidf_fallback",
            (_, true) => "bge_m3_partial_rrf_cross_encoder",
            (_, false) => "bge_m3_partial_rrf",
        }
        .into();
        *self.last_strategy.write().unwrap() = strategy.clone();

        // --- Stage 5: Health penalty + zombie filter ----------------------
        for candidate in &mut candidates {
            if let Some(health_score) = self
                .health_manager
                .get_health_score(&candidate.tool_id)
                .await
            {
                candidate.health_score = health_score.health_score;
                candidate.degraded = health_score.degraded;

                let health_penalty = if health_score.degraded {
                    0.1
                } else {
                    health_score.health_score
                };
                candidate.final_score = candidate.semantic_score
                    * (1.0 - health_weight + health_weight * health_penalty);

                if health_score.zombie {
                    candidate.final_score = 0.0;
                }
            }
        }

        // Final sort + top_k slice. Tie-break prefers tools whose NAME
        // contains a query token -- previously equal scores left ordering
        // to hash-iteration randomness.
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

        // Record servers of returned tools for session coherence.
        {
            let mut last = self.last_servers.write().unwrap();
            last.clear();
            for c in candidates.iter().take(top_k) {
                if !c.server_name.is_empty() && !last.contains(&c.server_name) {
                    last.push(c.server_name.clone());
                }
            }
        }

        let results = candidates
            .into_iter()
            .filter(|c| c.final_score > 0.0)
            .take(top_k)
            .collect();
        (results, strategy)
    }

    /// Fallback-2 support: name-only listing of every indexed tool, grouped
    /// by server. Deterministic escape hatch when semantic+lexical retrieval
    /// both fail.
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
    /// Primary signal = lexical corroboration: re-run the lexical lane on
    /// the ORIGINAL query and require the top fused candidate to appear
    /// there. The RRF fusion overwrites semantic_score with the fusion
    /// score, so a nonzero value cannot prove the *lexical* lane matched
    /// (the embedding lane also contributes). An embedding-only hit (no
    /// lexical corroboration) is exactly the "confidently wrong" risk this
    /// flags.
    pub async fn low_confidence(&self, results: &[RankedTool], query: &str) -> bool {
        if results.is_empty() {
            return true;
        }
        let top = &results[0];
        !self.lexical_corroborated(top, query)
    }

    /// Does the lexical lane (re-run on the original query) return this tool?
    fn lexical_corroborated(&self, top: &RankedTool, query: &str) -> bool {
        // Prefer the BGE-M3 sparse lane when it's loaded; fall back to
        // TF-IDF for network-free mode.
        if self.embedding_enabled && !self.embedding.is_empty() {
            if let Ok(sparse) = self.embedding.search_sparse(query, 5) {
                if sparse.iter().any(|(id, _)| id == &top.tool_id) {
                    return true;
                }
            }
        }
        self.fallback_tfidf
            .search(query, 5)
            .iter()
            .any(|t| t.tool_id == top.tool_id)
    }
}
