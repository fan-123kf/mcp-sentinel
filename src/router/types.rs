use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RankedTool {
    pub tool_id: String,
    pub tool_name: String,
    pub server_name: String,
    pub description: String,
    pub semantic_score: f64,
    pub health_score: f64,
    pub final_score: f64,
    pub degraded: bool,
}
