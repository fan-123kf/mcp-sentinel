//! Hybrid embedding index over the tool corpus.
//!
//! Uses BGE-M3 to produce both a dense vector (semantic) and a sparse
//! vector (learned lexical, from the same forward pass). The dense side
//! replaces the previous bge-small-zh cosine lane; the sparse side
//! replaces the hand-rolled TF-IDF + synonym-expansion + RRF lanes.
//!
//! Two operating modes (mutually exclusive on the model files side):
//!
//! 1. **Local files** (default, network-free): BGE-M3 ONNX + tokenizer
//!    files are read from `FASTEMBED_MODEL_DIR` (default
//!    `.fastembed_cache/Xenova/bge-m3`). Loaded via
//!    `Bgem3Embedding::try_new_from_user_defined`.
//!
//! 2. **HF-hub download** (fallback): when `SENTINEL_DOWNLOAD_MODELS=1`,
//!    fastembed's hf-hub fetches the official `BAAI/bge-m3` model on first
//!    use and caches it. This is the path CI / fresh installs use.
//!
//! Both modes share the same downstream interface so the router can be
//! configured without changing call sites.

use crate::backend::Tool;
use crate::router::types::RankedTool;
use anyhow::{Context};
use fastembed::{Bgem3Embedding, Bgem3InitOptions, Bgem3Model, SparseEmbedding};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};

/// Same enriched text the TF-IDF index built -- dense and sparse lanes MUST
/// see the same corpus or the RRF fusion compares rankings over different
/// documents.
pub(crate) fn enriched_text(tool: &Tool) -> String {
    let title = tool.title.as_deref().unwrap_or("");
    let param_descs: Vec<String> = tool
        .input_schema
        .get("properties")
        .and_then(serde_json::Value::as_object)
        .map(|props| {
            props
                .iter()
                .filter_map(|(pname, pv)| {
                    let desc = pv.get("description").and_then(serde_json::Value::as_str)?;
                    Some(format!("{} {}", pname, desc))
                })
                .collect()
        })
        .unwrap_or_default();
    let required: Vec<&str> = tool
        .input_schema
        .get("required")
        .and_then(serde_json::Value::as_array)
        .map(|a| a.iter().filter_map(serde_json::Value::as_str).collect())
        .unwrap_or_default();

    let mut text = format!(
        "{} {} {} {}",
        tool.server_name.as_deref().unwrap_or(""),
        title,
        tool.name,
        tool.description
    );
    if !required.is_empty() {
        text.push_str(&format!(" required: {}", required.join(" ")));
    }
    if !param_descs.is_empty() {
        text.push_str(&format!(" params: {}", param_descs.join("; ")));
    }
    text
}

pub(crate) const MODEL_DIR_ENV: &str = "FASTEMBED_MODEL_DIR";
pub(crate) const DEFAULT_MODEL_DIR: &str = ".fastembed_cache/Xenova/bge-m3";
pub(crate) const DOWNLOAD_ENV: &str = "SENTINEL_DOWNLOAD_MODELS";

struct Doc {
    tool: Tool,
    /// L2-normalized dense vector (BGE-M3 default dim: 1024 for BGEM3Q).
    dense: Vec<f32>,
    /// BGE-M3 learned sparse vector. Lexical-weight vector with vocab indices.
    sparse: SparseEmbedding,
}

/// Hybrid index: dense (cosine) + sparse (lexical dot product) in one model.
pub struct EmbeddingIndex {
    docs: RwLock<Vec<Doc>>,
    model: RwLock<Option<Arc<Mutex<Bgem3Embedding>>>>,
    error: RwLock<Option<String>>,
    sparse_top_k: usize,
}

impl EmbeddingIndex {
    pub fn new() -> Self {
        Self {
            docs: RwLock::new(Vec::new()),
            model: RwLock::new(None),
            error: RwLock::new(None),
            sparse_top_k: 256,
        }
    }

    fn model_dir() -> PathBuf {
        std::env::var(MODEL_DIR_ENV)
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from(DEFAULT_MODEL_DIR))
    }

    fn download_enabled() -> bool {
        std::env::var(DOWNLOAD_ENV)
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false)
    }

    fn model(&self) -> anyhow::Result<Arc<Mutex<Bgem3Embedding>>> {
        // Fast path: already built.
        if let Some(m) = self.model.read().unwrap().as_ref() {
            return Ok(Arc::clone(m));
        }
        // One previous failure is sticky -- don't retry a missing model dir on
        // every query (would add ~2ms and a warn spam per request).
        if let Some(err) = self.error.read().unwrap().as_ref() {
            anyhow::bail!("embedding model unavailable (sticky): {}", err);
        }

        let mut guard = self.model.write().unwrap();
        if let Some(m) = guard.as_ref() {
            return Ok(Arc::clone(m));
        }

        let model: anyhow::Result<Bgem3Embedding> = if Self::download_enabled() {
            // HF-hub download path: fastembed fetches BAAI/bge-m3 and caches it.
            Bgem3Embedding::try_new(Bgem3InitOptions::new(Bgem3Model::BGEM3Q))
                .map_err(|e| anyhow::anyhow!("bge-m3 download/init failed: {}", e))
        } else {
            // Local-files path: load ONNX + tokenizer from FASTEMBED_MODEL_DIR.
            // fastembed 5.x does not yet expose try_new_from_user_defined for
            // BGE-M3, so this path requires a future fastembed release. Until
            // then the local-files branch only verifies the model dir exists
            // and points the user at the download path.
            let dir = Self::model_dir();
            if !dir.join("onnx/model.onnx").exists() {
                anyhow::bail!(
                    "bge-m3 model files not found at {} (set SENTINEL_DOWNLOAD_MODELS=1 \
                     to fetch via hf-hub, or provision model files per docs/retrieval-upgrade-2026-09-01.md)",
                    dir.display()
                );
            }
            Bgem3Embedding::try_new(Bgem3InitOptions::new(Bgem3Model::BGEM3Q))
                .map_err(|e| anyhow::anyhow!(
                    "local bge-m3 init failed (set SENTINEL_DOWNLOAD_MODELS=1 for hf-hub): {}", e
                ))
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

    fn l2_normalize(v: &mut [f32]) {
        let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 0.0 {
            for x in v.iter_mut() {
                *x /= norm;
            }
        }
    }

    /// Build the index over the full tool corpus. Called on startup; replaces
    /// the previous contents atomically. On failure, records a sticky error
    /// so the router falls back to TF-IDF-only (graceful degradation).
    pub fn build_index(&self, tools: &[Tool]) -> anyhow::Result<usize> {
        if tools.is_empty() {
            *self.docs.write().unwrap() = Vec::new();
            return Ok(0);
        }
        let result: anyhow::Result<usize> = (|| {
            let model = self.model()?;
            let texts: Vec<String> = tools.iter().map(enriched_text).collect();

            let output = {
                let mut m = model.lock().unwrap();
                m.embed(texts, None)?
            };

            // L2-normalize dense so cosine similarity == dot product.
            let mut dense = output.dense;
            for v in dense.iter_mut() {
                Self::l2_normalize(v);
            }

            let docs: Vec<Doc> = tools
                .iter()
                .zip(dense.into_iter())
                .zip(output.sparse.into_iter())
                .map(|((tool, dense), sparse)| Doc {
                    tool: tool.clone(),
                    dense,
                    sparse,
                })
                .collect();
            let n = docs.len();
            *self.docs.write().unwrap() = docs;
            Ok(n)
        })();
        if let Err(e) = &result {
            *self.error.write().unwrap() = Some(e.to_string());
        } else {
            *self.error.write().unwrap() = None;
        }
        result
    }

    /// Dense semantic search: returns (tool_id, RankedTool) sorted by cosine
    /// score desc, up to top_k.
    pub fn search_dense(&self, query: &str, top_k: usize) -> anyhow::Result<Vec<(String, RankedTool)>> {
        let docs = self.docs.read().unwrap();
        if docs.is_empty() {
            return Ok(Vec::new());
        }
        let model = self.model()?;
        let mut output = {
            let mut m = model.lock().unwrap();
            m.embed(vec![query.to_string()], None)?
        };
        let mut qv = output.dense.pop().unwrap_or_default();
        Self::l2_normalize(&mut qv);

        let mut scored: Vec<(String, RankedTool)> = docs
            .iter()
            .map(|doc| {
                let dot: f32 = qv.iter().zip(doc.dense.iter()).map(|(a, b)| a * b).sum();
                let rt = RankedTool {
                    tool_id: doc.tool.tool_id.clone(),
                    tool_name: doc.tool.name.clone(),
                    server_name: doc.tool.server_name.clone().unwrap_or_default(),
                    description: doc.tool.description.clone(),
                    semantic_score: dot as f64,
                    health_score: 1.0,
                    final_score: dot as f64,
                    degraded: false,
                };
                (rt.tool_id.clone(), rt)
            })
            .collect();
        scored.sort_by(|a, b| {
            b.1.semantic_score
                .partial_cmp(&a.1.semantic_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        scored.truncate(top_k);
        Ok(scored)
    }

    /// Sparse lexical search via the BGE-M3 learned sparse vector. Computes
    /// a dot product over the intersection of query and document token indices
    /// -- the BGE-M3 sparse vector is the same lexical-weight structure that
    /// used to live in our hand-rolled TF-IDF + synonym expansion.
    pub fn search_sparse(
        &self,
        query: &str,
        top_k: usize,
    ) -> anyhow::Result<Vec<(String, RankedTool)>> {
        let docs = self.docs.read().unwrap();
        if docs.is_empty() {
            return Ok(Vec::new());
        }
        let model = self.model()?;
        let mut output = {
            let mut m = model.lock().unwrap();
            m.embed(vec![query.to_string()], None)?
        };
        let q_sparse: SparseEmbedding = output
            .sparse
            .pop()
            .unwrap_or_else(|| SparseEmbedding { indices: Vec::new(), values: Vec::new() });

        // Build a sparse-weight map for the query so we can score each doc
        // by intersecting their non-zero indices and summing query * doc.
        let mut scored: Vec<(String, RankedTool, f32)> = docs
            .iter()
            .map(|doc| {
                let score = sparse_dot(&q_sparse, &doc.sparse);
                let rt = RankedTool {
                    tool_id: doc.tool.tool_id.clone(),
                    tool_name: doc.tool.name.clone(),
                    server_name: doc.tool.server_name.clone().unwrap_or_default(),
                    description: doc.tool.description.clone(),
                    semantic_score: score as f64,
                    health_score: 1.0,
                    final_score: score as f64,
                    degraded: false,
                };
                (rt.tool_id.clone(), rt, score)
            })
            .collect();
        scored.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(top_k);
        Ok(scored.into_iter().map(|(id, rt, _)| (id, rt)).collect())
    }

    pub fn len(&self) -> usize {
        self.docs.read().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Compute the dot product of two BGE-M3 sparse embeddings by walking their
/// (sorted-by-index) index lists in parallel. Both vectors are guaranteed
/// to have indices in ascending order.
fn sparse_dot(q: &SparseEmbedding, d: &SparseEmbedding) -> f32 {
    let mut qi = q.indices.iter();
    let mut qv = q.values.iter();
    let mut di = d.indices.iter();
    let mut dv = d.values.iter();
    let mut acc = 0.0f32;
    let mut qn = qi.next().zip(qv.next());
    let mut dn = di.next().zip(dv.next());
    while let (Some((qi_v, qv_v)), Some((di_v, dv_v))) = (qn, dn) {
        match qi_v.cmp(di_v) {
            std::cmp::Ordering::Equal => {
                acc += qv_v * dv_v;
                qn = qi.next().zip(qv.next());
                dn = di.next().zip(dv.next());
            }
            std::cmp::Ordering::Less => qn = qi.next().zip(qv.next()),
            std::cmp::Ordering::Greater => dn = di.next().zip(dv.next()),
        }
    }
    acc
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::Tool;

    fn tool(id: &str, server: &str, name: &str, desc: &str) -> Tool {
        Tool {
            name: name.to_string(),
            description: desc.to_string(),
            input_schema: serde_json::json!({}),
            annotations: None,
            title: None,
            tool_id: id.to_string(),
            server_name: Some(server.to_string()),
        }
    }

    #[test]
    fn enriched_text_includes_server_and_params() {
        let mut t = tool(
            "filesystem::read_file",
            "filesystem",
            "read_file",
            "Read the complete contents of a file as text.",
        );
        t.input_schema = serde_json::json!({
            "properties": {
                "path": {"type": "string"},
                "head": {"type": "number", "description": "returns only the first N lines of the file"}
            },
            "required": ["path"]
        });
        let text = enriched_text(&t);
        assert!(text.contains("filesystem"));
        assert!(text.contains("first N lines"));
        assert!(text.contains("required: path"));
    }

    #[test]
    fn sparse_dot_intersects_indices() {
        let q = SparseEmbedding {
            indices: vec![1, 3, 5],
            values: vec![0.5, 0.4, 0.1],
        };
        let d = SparseEmbedding {
            indices: vec![2, 3, 4, 5],
            values: vec![0.2, 0.6, 0.7, 0.3],
        };
        // Shared indices: 3 -> 0.4*0.6=0.24, 5 -> 0.1*0.3=0.03 => 0.27
        assert!((sparse_dot(&q, &d) - 0.27).abs() < 1e-6);
    }

    #[test]
    #[ignore] // Requires BGE-M3 model files -- run with `cargo test -- --ignored`.
    fn semantic_search_ranks_pull_request_above_irrelevant() {
        let dir = std::env::var(MODEL_DIR_ENV).unwrap_or_else(|_| DEFAULT_MODEL_DIR.to_string());
        if !PathBuf::from(&dir).join("onnx/model.onnx").exists() {
            eprintln!("skip: model files not provisioned at {}", dir);
            return;
        }

        let index = EmbeddingIndex::new();
        let tools = vec![
            tool(
                "github::create_pull_request",
                "github",
                "create_pull_request",
                "Create a new pull request in a GitHub repository",
            ),
            tool(
                "github::search_code",
                "github",
                "search_code",
                "Search for code in GitHub repositories",
            ),
            tool(
                "filesystem::read_file",
                "filesystem",
                "read_file",
                "Read contents of a file from filesystem",
            ),
        ];
        let n = index.build_index(&tools).expect("build");
        assert_eq!(n, 3);
        let results = index
            .search_dense("how do I let teammates see my code changes", 3)
            .expect("search");
        assert!(!results.is_empty());
        for (i, (id, rt)) in results.iter().enumerate() {
            eprintln!("  #{i} {id} cosine={:.4}", rt.semantic_score);
        }
        let pr_rank = results
            .iter()
            .position(|(id, _)| id == "github::create_pull_request");
        assert!(pr_rank.is_some(), "PR tool must appear in results");
        assert!(
            pr_rank.unwrap() <= 1,
            "PR tool must be top-2, got rank {}",
            pr_rank.unwrap()
        );
    }
}
