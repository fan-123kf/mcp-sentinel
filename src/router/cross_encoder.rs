//! Cross-encoder reranker.
//!
//! Replaces the hand-rolled feature-based reranker (name_overlap,
//! desc_overlap, param_match, same_server). Cross-encoders see the
//! [query, document] pair jointly and produce a relevance score that's
//! strictly stronger than rules or bi-encoder cosine similarity.
//!
//! Model: bge-reranker-v2-m3 (multilingual, ONNX, ~300MB). Loaded
//! either from local files (FASTEMBED_MODEL_DIR / reranker/ subdir) or
//! via hf-hub download when SENTINEL_DOWNLOAD_MODELS=1.
//!
//! Reranking is run over the top-N candidates from the dense+sparse RRF
//! fusion (default N = 20) -- beyond that the cost/benefit curve flattens
//! for our 50-tool corpus.

use crate::backend::Tool;
use crate::router::embedding::{enriched_text, MODEL_DIR_ENV};
use crate::router::types::RankedTool;
use anyhow::Context;
use fastembed::{RerankInitOptions, RerankerModel, TextRerank};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};

pub(crate) const DOWNLOAD_ENV: &str = "SENTINEL_DOWNLOAD_MODELS";
pub(crate) const DEFAULT_RERANK_DIR: &str = ".fastembed_cache/Xenova/bge-reranker-v2-m3";

/// Cross-encoder wrapper. Sits behind a Mutex because fastembed's
/// TextRerank::rerank takes &mut self (ONNX session is not Sync).
pub struct CrossEncoder {
    model: RwLock<Option<Arc<Mutex<TextRerank>>>>,
    error: RwLock<Option<String>>,
}

impl CrossEncoder {
    pub fn new() -> Self {
        Self {
            model: RwLock::new(None),
            error: RwLock::new(None),
        }
    }

    fn model_dir() -> PathBuf {
        std::env::var(MODEL_DIR_ENV)
            .map(|d| PathBuf::from(d).join("reranker"))
            .unwrap_or_else(|_| PathBuf::from(DEFAULT_RERANK_DIR))
    }

    fn download_enabled() -> bool {
        std::env::var(DOWNLOAD_ENV)
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false)
    }

    fn model(&self) -> anyhow::Result<Arc<Mutex<TextRerank>>> {
        if let Some(m) = self.model.read().unwrap().as_ref() {
            return Ok(Arc::clone(m));
        }
        if let Some(err) = self.error.read().unwrap().as_ref() {
            anyhow::bail!("cross-encoder unavailable (sticky): {}", err);
        }

        let mut guard = self.model.write().unwrap();
        if let Some(m) = guard.as_ref() {
            return Ok(Arc::clone(m));
        }

        let model: anyhow::Result<TextRerank> = if Self::download_enabled() {
            TextRerank::try_new(RerankInitOptions::new(RerankerModel::BGERerankerV2M3))
                .map_err(|e| anyhow::anyhow!("reranker download/init failed: {}", e))
        } else {
            let dir = Self::model_dir();
            let read = |name: &str| -> anyhow::Result<Vec<u8>> {
                std::fs::read(dir.join(name))
                    .with_context(|| format!("missing reranker file {} in {}", name, dir.display()))
            };

            // BGE reranker ONNX uses external data files (model.onnx + model.onnx_data).
            // ORT resolves relative paths from the current working directory at load time,
            // so we must change to the model dir before loading.
            let onnx = read("onnx/model.onnx")?;
            let tokenizer = read("tokenizer.json")?;
            let config = read("config.json")?;
            let special_tokens = read("special_tokens_map.json")?;
            let tokenizer_config = read("tokenizer_config.json")?;

            // BGE reranker ONNX uses external data files (model.onnx + model.onnx_data).
            // ONNX external data uses relative paths resolved from CWD at load time,
            // so we must change to the model dir before loading AND copy the
            // external data file there.
            let onnx = read("onnx/model.onnx")?;
            let tokenizer = read("tokenizer.json")?;
            let config = read("config.json")?;
            let special_tokens = read("special_tokens_map.json")?;
            let tokenizer_config = read("tokenizer_config.json")?;

            // Copy model.onnx_data to the model dir so ORT resolves it correctly.
            let onnx_data_src = dir.join("onnx/model.onnx_data");
            let onnx_data_dst = dir.join("model.onnx_data");
            if !onnx_data_dst.exists() {
                std::fs::copy(&onnx_data_src, &onnx_data_dst)
                    .with_context(|| "copy model.onnx_data to model dir")?;
            }

            // Save original CWD and change to model dir for ORT loading.
            let original_cwd = std::env::current_dir()
                .with_context(|| "get original cwd")?;
            std::env::set_current_dir(&dir)
                .with_context(|| format!("setcwd to {}", dir.display()))?;

            let user_model = fastembed::UserDefinedRerankingModel::new(
                onnx,
                fastembed::TokenizerFiles {
                    tokenizer_file: tokenizer,
                    config_file: config,
                    special_tokens_map_file: special_tokens,
                    tokenizer_config_file: tokenizer_config,
                },
            );
            let opts = fastembed::RerankInitOptionsUserDefined::default();
            let result = TextRerank::try_new_from_user_defined(user_model, opts)
                .map_err(|e| anyhow::anyhow!("reranker ONNX session init failed: {}", e));

            // Always restore CWD.
            let _ = std::env::set_current_dir(&original_cwd);
            result
        };

        match model {
            Ok(m) => {
                let arc = Arc::new(Mutex::new(m));
                *guard = Some(Arc::clone(&arc));
                Ok(arc)
            }
            Err(e) => {
                *self.error.write().unwrap() = Some(e.to_string());
                Err(e)
            }
        }
    }

    /// Re-score `candidates` for `query` and return them sorted by
    /// cross-encoder score desc. Returns Err (sticky) when the model is
    /// unavailable; caller should fall back to preserving RRF order.
    pub fn rerank(
        &self,
        query: &str,
        candidates: Vec<RankedTool>,
        docs: &[Tool],
    ) -> anyhow::Result<Vec<RankedTool>> {
        if candidates.is_empty() {
            return Ok(candidates);
        }

        // Build the text we feed to the cross-encoder per candidate. We use
        // the same enriched corpus text the embedding lanes indexed -- keeps
        // the semantics consistent across the pipeline.
        let mut tool_by_id: std::collections::HashMap<&str, &Tool> =
            std::collections::HashMap::with_capacity(docs.len());
        for t in docs {
            tool_by_id.insert(&t.tool_id, t);
        }

        let mut texts: Vec<String> = Vec::with_capacity(candidates.len());
        for cand in &candidates {
            let text = tool_by_id
                .get(cand.tool_id.as_str())
                .map(|t| enriched_text(t))
                .unwrap_or_else(|| {
                    format!(
                        "{} {} {}",
                        cand.server_name, cand.tool_name, cand.description
                    )
                });
            texts.push(text);
        }

        let model = self.model()?;
        let results = {
            let mut m = model.lock().unwrap();
            m.rerank(query.to_string(), texts, false, None)?
        };

        // Map fastembed's sorted (index, score) results back into our
        // candidates. fastembed sorts by score desc; we mirror that order
        // and overwrite final_score with the cross-encoder score.
        let mut rescored: Vec<(usize, f32)> =
            results.iter().map(|r| (r.index, r.score)).collect();
        // Re-sort defensively (fastembed already returns sorted, but be safe).
        rescored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        let mut out = Vec::with_capacity(candidates.len());
        for (orig_idx, score) in rescored {
            if orig_idx >= candidates.len() {
                continue;
            }
            let mut cand = candidates[orig_idx].clone();
            // Normalize score into 0..1 via sigmoid so it composes with
            // downstream health penalty the same way semantic_score did.
            let normalized = 1.0 / (1.0 + (-score as f64).exp());
            cand.semantic_score = normalized;
            cand.final_score = normalized;
            out.push(cand);
        }
        Ok(out)
    }

    pub fn is_available(&self) -> bool {
        // Fast probe: don't actually try to load the model -- the router
        // gates the rerank pass on this, and loading has a multi-second cost
        // on first call. The actual load happens lazily on the first rerank.
        !self.sticky_error_for_probe()
    }

    /// Returns true if a previous load attempt failed and we should skip
    /// the rerank step entirely (avoiding per-request warn spam).
    fn sticky_error_for_probe(&self) -> bool {
        self.error.read().unwrap().is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore] // Requires bge-reranker-v2-m3 model files.
    fn rerank_promotes_exact_match() {
        let dir = std::env::var(MODEL_DIR_ENV)
            .map(|d| PathBuf::from(d).join("reranker"))
            .unwrap_or_else(|_| PathBuf::from(DEFAULT_RERANK_DIR));
        if !dir.join("onnx/model.onnx").exists() {
            eprintln!("skip: reranker model files not provisioned at {}", dir.display());
            return;
        }

        let tools = vec![Tool {
            name: "create_issue".to_string(),
            description: "Create a new issue in a GitHub repository".to_string(),
            input_schema: serde_json::json!({}),
            annotations: None,
            title: None,
            tool_id: "github::create_issue".to_string(),
            server_name: Some("github".to_string()),
        }];

        let candidates = vec![RankedTool {
            tool_id: "github::create_issue".to_string(),
            tool_name: "create_issue".to_string(),
            server_name: "github".to_string(),
            description: "Create a new issue in a GitHub repository".to_string(),
            semantic_score: 0.3,
            health_score: 1.0,
            final_score: 0.3,
            degraded: false,
        }];

        let ce = CrossEncoder::new();
        let out = ce.rerank("create a GitHub issue", candidates, &tools).expect("rerank");
        assert!(!out.is_empty());
        assert!(out[0].final_score > 0.5, "exact-match should score high");
    }
}
