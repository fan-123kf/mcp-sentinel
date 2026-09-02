//! Stub for the legacy feature-based reranker weights.
//!
//! The feature-based rerank (name_overlap / desc_overlap / param_match /
//! same_server) has been replaced by `cross_encoder::CrossEncoder`, which
//! scores [query, document] pairs jointly via a trained cross-encoder model.
//!
//! This module is kept so the module tree still compiles and so that
//! `RerankWeights` remains available for any future external config or
//! experiment that wants to blend cross-encoder scores with feature scores.
//! At present the router uses cross-encoder scores directly.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RerankWeights {
    /// Reserved for blending cross-encoder score with feature scores.
    /// Currently unused -- see module docs.
    #[serde(default = "default_ce")]
    pub cross_encoder: f64,
    #[serde(default = "default_name")]
    pub name_overlap: f64,
    #[serde(default = "default_desc")]
    pub desc_overlap: f64,
    #[serde(default = "default_param")]
    pub param_match: f64,
    #[serde(default = "default_server")]
    pub same_server: f64,
}

fn default_ce() -> f64 {
    1.0
}
fn default_name() -> f64 {
    0.0
}
fn default_desc() -> f64 {
    0.0
}
fn default_param() -> f64 {
    0.0
}
fn default_server() -> f64 {
    0.0
}

impl Default for RerankWeights {
    fn default() -> Self {
        Self {
            cross_encoder: default_ce(),
            name_overlap: default_name(),
            desc_overlap: default_desc(),
            param_match: default_param(),
            same_server: default_server(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weights_default_to_cross_encoder_only() {
        let w = RerankWeights::default();
        assert_eq!(w.cross_encoder, 1.0);
        assert_eq!(w.name_overlap, 0.0);
    }

    #[test]
    fn weights_respect_config_shape() {
        let w: RerankWeights =
            serde_json::from_value(serde_json::json!({"cross_encoder": 0.8})).unwrap();
        assert_eq!(w.cross_encoder, 0.8);
        // Legacy fields still parse and stay at their default.
        assert_eq!(w.name_overlap, 0.0);
    }
}
