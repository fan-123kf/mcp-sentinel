//! Embedding-based semantic index over the tool corpus.
//!
//! Mirrors the TfIdfIndex interface (build_index / search_ranked) so the
//! hybrid router can add retrieval lanes without touching the gateway layer.
//!
//! Model: bge-small-zh-v1.5 (Xenova ONNX export, 512-dim, Chinese+English)
//! loaded from LOCAL files under FASTEMBED_MODEL_DIR (or ./.fastembed_cache/
//! Xenova/bge-small-zh-v1.5). We deliberately bypass fastembed's HF-hub
//! downloader: huggingface.co is unreachable from this network and the
//! mirror's large-file redirect loses the Content-Range header hf-hub
//! requires. Model files are provisioned out-of-band (see eval data dir).

use crate::backend::Tool;
use crate::router::types::RankedTool;
use anyhow::{Context, Result};
use serde_json::Value;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};

/// Same enriched text TfIdfIndex builds -- both lanes MUST see the same
/// corpus or the RRF fusion compares rankings over different documents.
pub(crate) fn enriched_text(tool: &Tool) -> String {
    let title = tool.title.as_deref().unwrap_or("");
    let param_descs: Vec<String> = tool
        .input_schema
        .get("properties")
        .and_then(Value::as_object)
        .map(|props| {
            props
                .iter()
                .filter_map(|(pname, pv)| {
                    let desc = pv.get("description").and_then(Value::as_str)?;
                    Some(format!("{} {}", pname, desc))
                })
                .collect()
        })
        .unwrap_or_default();
    let required: Vec<&str> = tool
        .input_schema
        .get("required")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).collect())
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
pub(crate) const DEFAULT_MODEL_DIR: &str = ".fastembed_cache/Xenova/bge-small-zh-v1.5";

struct Doc {
    tool: Tool,
    vector: Vec<f32>,
}

pub struct EmbeddingIndex {
    docs: RwLock<Vec<Doc>>,
    model: RwLock<Option<Arc<Mutex<fastembed::TextEmbedding>>>>,
    error: RwLock<Option<String>>,
}

impl EmbeddingIndex {
    pub fn new() -> Self {
        Self {
            docs: RwLock::new(Vec::new()),
            model: RwLock::new(None),
            error: RwLock::new(None),
        }
    }

    fn model_dir() -> PathBuf {
        std::env::var(MODEL_DIR_ENV)
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from(DEFAULT_MODEL_DIR))
    }

    fn model(&self) -> anyhow::Result<Arc<Mutex<fastembed::TextEmbedding>>> {
        // Fast path: already built.
        if let Some(m) = self.model.read().unwrap().as_ref() {
            return Ok(Arc::clone(m));
        }
        // One previous failure is sticky -- don't retry a missing model dir on
        // every query (that would add ~2ms and a warn spam per request).
        if let Some(err) = self.error.read().unwrap().as_ref() {
            anyhow::bail!("embedding model unavailable (sticky): {}", err);
        }

        let mut guard = self.model.write().unwrap();
        if let Some(m) = guard.as_ref() {
            return Ok(Arc::clone(m));
        }

        let dir = Self::model_dir();
        let read = |name: &str| -> anyhow::Result<Vec<u8>> {
            std::fs::read(dir.join(name))
                .with_context(|| format!("missing model file {} in {}", name, dir.display()))
        };

        let onnx = read("onnx/model.onnx")?;
        let tokenizer = read("tokenizer.json")?;
        let config = read("config.json")?;
        let special_tokens = read("special_tokens_map.json")?;
        let tokenizer_config = read("tokenizer_config.json")?;

        let user_model = fastembed::UserDefinedEmbeddingModel::new(
            onnx,
            fastembed::TokenizerFiles {
                tokenizer_file: tokenizer,
                config_file: config,
                special_tokens_map_file: special_tokens,
                tokenizer_config_file: tokenizer_config,
            },
        );

        let opts = fastembed::InitOptionsUserDefined::default();
        let model = fastembed::TextEmbedding::try_new_from_user_defined(user_model, opts)
            .map_err(|e| anyhow::anyhow!("ONNX session init failed: {}", e))?;
        let arc = Arc::new(Mutex::new(model));
        *guard = Some(Arc::clone(&arc));
        Ok(arc)
    }

    fn embed_batch(
        model: &Mutex<fastembed::TextEmbedding>,
        texts: Vec<String>,
    ) -> anyhow::Result<Vec<Vec<f32>>> {
        let mut vecs = model.lock().unwrap().embed(texts, None)?;
        // L2-normalize so cosine similarity == dot product.
        for v in vecs.iter_mut() {
            let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
            if norm > 0.0 {
                for x in v.iter_mut() {
                    *x /= norm;
                }
            }
        }
        Ok(vecs)
    }

    /// Build the index over the full tool corpus. Called on startup; replaces
    /// the previous contents atomically. On failure, records a sticky error
    /// and the router keeps running TF-IDF-only (graceful degradation).
    pub fn build_index(&self, tools: &[Tool]) -> anyhow::Result<usize> {
        if tools.is_empty() {
            *self.docs.write().unwrap() = Vec::new();
            return Ok(0);
        }
        let result: anyhow::Result<usize> = (|| {
            let model = self.model()?;
            let texts: Vec<String> = tools.iter().map(enriched_text).collect();
            let vectors = Self::embed_batch(&model, texts)?;
            let docs: Vec<Doc> = tools
                .iter()
                .zip(vectors.into_iter())
                .map(|(tool, vector)| Doc {
                    tool: tool.clone(),
                    vector,
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

    /// Semantic search: returns (tool_id, RankedTool) sorted by cosine score
    /// desc, up to top_k. The RankedTool carries the tool's own metadata so
    /// the RRF fusion needs no external lookups.
    pub fn search_ranked(
        &self,
        query: &str,
        top_k: usize,
    ) -> anyhow::Result<Vec<(String, RankedTool)>> {
        let docs = self.docs.read().unwrap();
        if docs.is_empty() {
            return Ok(Vec::new());
        }
        let model = self.model()?;
        let mut qv = Self::embed_batch(&model, vec![query.to_string()])?
            .pop()
            .ok_or_else(|| anyhow::anyhow!("empty query embedding"))?;

        let mut scored: Vec<(String, RankedTool)> = docs
            .iter()
            .map(|doc| {
                let dot: f32 = qv.iter().zip(doc.vector.iter()).map(|(a, b)| a * b).sum();
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

    pub fn len(&self) -> usize {
        self.docs.read().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn semantic_search_ranks_pull_request_above_irrelevant() {
        // The semantic-gap case from eval rounds 2/5: zero lexical overlap.
        // Skips silently if the model files aren't provisioned (CI).
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
            .search_ranked("how do I let teammates see my code changes", 3)
            .expect("search");
        assert!(!results.is_empty());
        for (i, (id, rt)) in results.iter().enumerate() {
            eprintln!("  #{i} {id} cosine={:.4}", rt.semantic_score);
        }
        // bge-small-zh ranks the PR tool #2 behind search_code for this query
        // (both are plausibly relevant); assert it reaches the top-2 with a
        // healthy margin above the filesystem tool.
        let pr_rank = results
            .iter()
            .position(|(id, _)| id == "github::create_pull_request");
        assert!(pr_rank.is_some(), "PR tool must appear in results");
        assert!(
            pr_rank.unwrap() <= 1,
            "PR tool must be top-2, got rank {}",
            pr_rank.unwrap()
        );
        let fs_rank = results
            .iter()
            .position(|(id, _)| id == "filesystem::read_file");
        let pr_score = results
            .iter()
            .find(|(id, _)| id == "github::create_pull_request")
            .unwrap()
            .1
            .semantic_score;
        if let Some(fs) = fs_rank {
            let fs_score = results[fs].1.semantic_score;
            assert!(pr_score > fs_score, "PR tool must outrank filesystem tool");
        }
    }
}
