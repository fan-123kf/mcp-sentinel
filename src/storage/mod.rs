mod sqlite;

pub use sqlite::StorageManager;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallRecord {
    pub id: i64,
    pub tool_id: String,
    pub server_name: String,
    pub tool_name: String,
    pub success: bool,
    pub latency_ms: u64,
    pub error_type: Option<String>,
    pub called_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DailyStat {
    pub tool_id: String,
    pub date: String,
    pub call_count: i64,
    pub success_count: i64,
    pub avg_latency: f64,
    pub p95_latency: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolRegistry {
    pub tool_id: String,
    pub server_name: String,
    pub tool_name: String,
    pub description: String,
    pub schema_json: String,
    pub first_seen: DateTime<Utc>,
    pub last_seen: DateTime<Utc>,
}
