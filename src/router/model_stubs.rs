//! Explicit model-free build: never reports a model ranking as successful.
use crate::backend::Tool;
use crate::router::RankedTool;
#[cfg_attr(not(test), allow(dead_code))]
pub struct EmbeddingIndex;
impl EmbeddingIndex {
    pub fn new() -> Self {
        Self
    }
    pub fn is_empty(&self) -> bool {
        true
    }
    pub fn build_index(&self, _: &[Tool]) -> anyhow::Result<usize> {
        anyhow::bail!("models feature disabled")
    }
    pub fn search_dense(&self, _: &str, _: usize) -> anyhow::Result<Vec<(String, RankedTool)>> {
        anyhow::bail!("models feature disabled")
    }
    pub fn search_sparse(&self, _: &str, _: usize) -> anyhow::Result<Vec<(String, RankedTool)>> {
        anyhow::bail!("models feature disabled")
    }
}
pub struct CrossEncoder;
impl CrossEncoder {
    pub fn new() -> Self {
        Self
    }
    pub fn verify(&self) -> anyhow::Result<()> {
        anyhow::bail!("models feature disabled")
    }
    pub fn is_available(&self) -> bool {
        false
    }
    pub fn rerank(
        &self,
        _: &str,
        _: Vec<RankedTool>,
        _: &[Tool],
    ) -> anyhow::Result<Vec<RankedTool>> {
        anyhow::bail!("models feature disabled")
    }
}
