use crate::health::HealthManager;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct BackendManager {
    pub(super) backends: Arc<RwLock<HashMap<String, super::Backend>>>,
    pub(super) health_manager: HealthManager,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tool {
    pub name: String,
    pub description: String,
    #[serde(rename = "inputSchema")]
    pub input_schema: serde_json::Value,
    /// MCP tool annotations from the server (readOnlyHint, destructiveHint,
    /// idempotentHint, openWorldHint). Optional: older/loose servers omit them.
    /// Used as an authoritative override for governance classification.
    #[serde(default)]
    pub annotations: Option<serde_json::Value>,
    #[serde(skip)]
    pub tool_id: String,
    #[serde(skip)]
    pub server_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub tool_id: String,
    pub arguments: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ToolCallResult {
    #[serde(rename = "success")]
    Success {
        content: serde_json::Value,
        latency_ms: u64,
    },
    #[serde(rename = "error")]
    Error { error: String, latency_ms: u64 },
}
