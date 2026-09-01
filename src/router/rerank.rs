//! Feature-based reranker: the antidote to "confidently wrong" semantic hits.
//!
//! Embedding recall widens the candidate pool (recall@5 up), but cosine
//! similarity alone can promote superficially-related tools over the exact
//! match. This pass re-scores fused candidates with four interpretable,
//! data-backed features and blends them with the RRF rank:
//!
//! - name_overlap:  query tokens appearing in the tool's NAME (strongest
//!   intent signal -- users/tool authors name tools after their purpose)
//! - desc_overlap:  query tokens in the DESCRIPTION
//! - param_match:   query tokens in parameter names / param descriptions
//!   (eval round 5: github ships 79 param descriptions, previously unused)
//! - same_server:   candidate belongs to a server already used in this
//!   conversation (session coherence; set via `with_last_servers`)
//!
//! Weights default to the values calibrated on the 21-query benchmark; they
//! are overridable via [routing] rerank_* in sentinel.toml (via RerankWeights).

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RerankWeights {
    /// RRF rank contribution (already normalized 0..1 upstream).
    #[serde(default = "default_ranks")]
    pub rank: f64,
    #[serde(default = "default_name")]
    pub name_overlap: f64,
    #[serde(default = "default_desc")]
    pub desc_overlap: f64,
    #[serde(default = "default_param")]
    pub param_match: f64,
    #[serde(default = "default_server")]
    pub same_server: f64,
}

fn default_ranks() -> f64 {
    0.45
}
fn default_name() -> f64 {
    0.30
}
fn default_desc() -> f64 {
    0.10
}
fn default_param() -> f64 {
    0.10
}
fn default_server() -> f64 {
    0.05
}

impl Default for RerankWeights {
    fn default() -> Self {
        Self {
            rank: default_ranks(),
            name_overlap: default_name(),
            desc_overlap: default_desc(),
            param_match: default_param(),
            same_server: default_server(),
        }
    }
}

/// Tokenized query kept between calls in one search.
pub struct QueryFeatures {
    tokens: Vec<String>,
    token_set: HashSet<String>,
}

impl QueryFeatures {
    pub fn new(query: &str) -> Self {
        let tokens: Vec<String> = crate::router::tfidf::TfIdfIndex::tokenize(query);
        let token_set: HashSet<String> = tokens.iter().cloned().collect();
        Self { tokens, token_set }
    }

    /// Fraction of query tokens covered by the tool NAME (substring match on
    /// either side). 1.0 when every query token appears in the name.
    pub fn name_overlap(&self, tool_name: &str) -> f64 {
        if self.token_set.is_empty() {
            return 0.0;
        }
        let name_l = tool_name.to_lowercase();
        let name_compact = tool_name.to_lowercase().replace(['_', '-'], "");
        let hits = self
            .tokens
            .iter()
            .filter(|t| name_l.contains(t.as_str()) || name_compact.contains(t.as_str()))
            .count();
        hits as f64 / self.token_set.len() as f64
    }

    /// Fraction of query tokens covered by description + parameter
    /// descriptions + parameter names combined.
    pub fn desc_overlap(&self, description: &str, schema: &Value) -> f64 {
        if self.token_set.is_empty() {
            return 0.0;
        }
        let mut haystack = description.to_lowercase();
        if let Some(props) = schema.get("properties").and_then(Value::as_object) {
            for (pname, pv) in props {
                haystack.push(' ');
                haystack.push_str(&pname.to_lowercase());
                if let Some(d) = pv.get("description").and_then(Value::as_str) {
                    haystack.push(' ');
                    haystack.push_str(&d.to_lowercase());
                }
            }
        }
        let hits = self
            .tokens
            .iter()
            .filter(|t| haystack.contains(t.as_str()))
            .count();
        hits as f64 / self.token_set.len() as f64
    }

    /// Fraction of query tokens hitting parameter names / descriptions only.
    pub fn param_match(&self, schema: &Value) -> f64 {
        if self.token_set.is_empty() {
            return 0.0;
        }
        let mut haystack = String::new();
        if let Some(props) = schema.get("properties").and_then(Value::as_object) {
            for (pname, pv) in props {
                haystack.push(' ');
                haystack.push_str(&pname.to_lowercase());
                if let Some(d) = pv.get("description").and_then(Value::as_str) {
                    haystack.push(' ');
                    haystack.push_str(&d.to_lowercase());
                }
            }
        }
        if haystack.is_empty() {
            return 0.0;
        }
        let hits = self
            .tokens
            .iter()
            .filter(|t| haystack.contains(t.as_str()))
            .count();
        hits as f64 / self.token_set.len() as f64
    }
}

/// Per-candidate rerank input: the fields the features need, already cloned
/// out of the schema at index time (see Tool::rerank_context).
pub struct RerankInput {
    pub tool_name: String,
    pub description: String,
    pub schema: Value,
    pub server_name: String,
}

impl RerankInput {
    /// Compute the four features for this candidate.
    pub fn features(&self, qf: &QueryFeatures) -> [f64; 4] {
        [
            qf.name_overlap(&self.tool_name),
            qf.desc_overlap(&self.description, &self.schema),
            qf.param_match(&self.schema),
            0.0, // same_server filled by caller (session state)
        ]
    }
}

/// Blend RRF rank (normalized 0..1) with the four features into a single
/// rerank score in 0..1.
pub fn rerank_score(
    norm_rank: f64,
    features: &[f64; 4],
    same_server: bool,
    w: &RerankWeights,
) -> f64 {
    let server = if same_server { 1.0 } else { 0.0 };
    w.rank * norm_rank
        + w.name_overlap * features[0]
        + w.desc_overlap * features[1]
        + w.param_match * features[2]
        + w.same_server * server
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn name_overlap_full_and_partial() {
        let qf = QueryFeatures::new("create issue on github");
        // tokenize drops len<=2 words, so tokens = {create, issue, github}.
        // "create_issue" contains create + issue but not github => 2/3.
        let score = qf.name_overlap("create_issue");
        assert!((score - 2.0 / 3.0).abs() < 1e-9, "got {}", score);
        // Compact name match: query token split differently from name tokens.
        let qf2 = QueryFeatures::new("pull request");
        let score2 = qf2.name_overlap("create_pull_request");
        assert!(score2 > 0.0, "compact match should hit: {}", score2);
    }

    #[test]
    fn param_match_finds_parameter_semantics() {
        let qf = QueryFeatures::new("read first lines of file");
        let input = RerankInput {
            tool_name: "read_file".into(),
            description: "Read a file.".into(),
            schema: json!({"properties": {"head": {"description": "returns only the first N lines of the file"}}}),
            server_name: "filesystem".into(),
        };
        let f = input.features(&qf);
        // "first"/"lines"/"file" hit the param description; "read" hits name.
        assert!(f[2] > 0.3, "param_match should be high: {}", f[2]);
    }

    #[test]
    fn rerank_prefers_exact_name_match_over_strong_semantic_only() {
        // Candidate A: great RRF rank, zero name overlap.
        // Candidate B: slightly worse rank, perfect name overlap.
        let qf = QueryFeatures::new("create issue");
        let input_b = RerankInput {
            tool_name: "create_issue".into(),
            description: "Create an issue.".into(),
            schema: json!({}),
            server_name: "github".into(),
        };
        let fa = [0.0, 0.2, 0.0, 0.0];
        let fb = input_b.features(&qf);
        let sa = rerank_score(1.0, &fa, false, &RerankWeights::default());
        let sb = rerank_score(0.9, &fb, false, &RerankWeights::default());
        assert!(sb > sa, "exact-name candidate must win: A={} B={}", sa, sb);
    }

    #[test]
    fn weights_respect_config_shape() {
        let w: RerankWeights = serde_json::from_value(json!({"name_overlap": 0.5})).unwrap();
        assert_eq!(w.name_overlap, 0.5);
        assert_eq!(w.rank, 0.45); // default kept
    }
}
