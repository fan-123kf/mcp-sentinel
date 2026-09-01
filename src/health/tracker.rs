use super::types::{HealthScore, ToolHealth};
use crate::storage::StorageManager;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::debug;

#[derive(Clone)]
pub struct HealthManager {
    tools: Arc<RwLock<HashMap<String, ToolHealth>>>,
    consecutive_failure_limit: u32,
    storage: Option<Arc<StorageManager>>,
}

impl HealthManager {
    pub fn new() -> Self {
        Self {
            tools: Arc::new(RwLock::new(HashMap::new())),
            consecutive_failure_limit: 5,
            storage: None,
        }
    }

    pub fn with_failure_limit(mut self, limit: u32) -> Self {
        self.consecutive_failure_limit = limit;
        self
    }

    pub fn with_storage(mut self, storage: Arc<StorageManager>) -> Self {
        self.storage = Some(storage);
        self
    }

    pub async fn record_success(&self, tool_id: &str, latency_ms: u64) {
        // Parse tool_id to get server_name and tool_name
        let parts: Vec<&str> = tool_id.split("::").collect();
        let (server_name, tool_name) = if parts.len() == 2 {
            (parts[0], parts[1])
        } else {
            ("unknown", tool_id)
        };

        // Update in-memory health
        {
            let mut tools = self.tools.write().await;
            let health = tools
                .entry(tool_id.to_string())
                .or_insert_with(|| ToolHealth::new(tool_id.to_string()));

            health.record_success(latency_ms);
        }

        // Persist to database if storage is available
        if let Some(storage) = &self.storage {
            if let Err(e) = storage
                .record_tool_call(tool_id, server_name, tool_name, true, latency_ms, None)
                .await
            {
                debug!("Failed to persist tool call: {}", e);
            }

            // Update p95 from database (rolling window)
            if let Ok(p95) = storage.get_p95_latency(tool_id, 1000).await {
                let mut tools = self.tools.write().await;
                if let Some(health) = tools.get_mut(tool_id) {
                    health.latency_p95 = p95;
                    health.compute_health_score();
                }
            }

            // Update 7-day call count
            if let Ok(count) = storage.get_call_count_window(tool_id, 7).await {
                let mut tools = self.tools.write().await;
                if let Some(health) = tools.get_mut(tool_id) {
                    health.call_count_7d = count;
                    health.compute_health_score();
                }
            }
        }

        debug!(
            tool_id = %tool_id,
            latency_ms = latency_ms,
            "Recorded success"
        );
    }

    pub async fn record_failure(&self, tool_id: &str) {
        let parts: Vec<&str> = tool_id.split("::").collect();
        let (server_name, tool_name) = if parts.len() == 2 {
            (parts[0], parts[1])
        } else {
            ("unknown", tool_id)
        };

        // Update in-memory health
        {
            let mut tools = self.tools.write().await;
            let health = tools
                .entry(tool_id.to_string())
                .or_insert_with(|| ToolHealth::new(tool_id.to_string()));

            health.record_failure();
        }

        // Persist to database
        if let Some(storage) = &self.storage {
            if let Err(e) = storage
                .record_tool_call(
                    tool_id,
                    server_name,
                    tool_name,
                    false,
                    0,
                    Some("unknown".to_string()),
                )
                .await
            {
                debug!("Failed to persist tool call: {}", e);
            }

            // Update 7-day call count
            if let Ok(count) = storage.get_call_count_window(tool_id, 7).await {
                let mut tools = self.tools.write().await;
                if let Some(health) = tools.get_mut(tool_id) {
                    health.call_count_7d = count;
                    health.compute_health_score();
                }
            }
        }

        debug!(
            tool_id = %tool_id,
            "Recorded failure"
        );
    }

    pub async fn get_health_score(&self, tool_id: &str) -> Option<HealthScore> {
        let tools = self.tools.read().await;
        tools.get(tool_id).map(|health| HealthScore {
            tool_id: health.tool_id.clone(),
            health_score: health.health_score,
            degraded: health.is_degraded(self.consecutive_failure_limit),
            zombie: health.is_zombie(),
        })
    }

    pub async fn get_all_scores(&self) -> Vec<HealthScore> {
        let tools = self.tools.read().await;
        tools
            .values()
            .map(|health| HealthScore {
                tool_id: health.tool_id.clone(),
                health_score: health.health_score,
                degraded: health.is_degraded(self.consecutive_failure_limit),
                zombie: health.is_zombie(),
            })
            .collect()
    }

    pub async fn is_degraded(&self, tool_id: &str) -> bool {
        let tools = self.tools.read().await;
        tools
            .get(tool_id)
            .map(|h| h.is_degraded(self.consecutive_failure_limit))
            .unwrap_or(false)
    }

    pub async fn get_detailed_health(&self, tool_id: &str) -> Option<ToolHealth> {
        let tools = self.tools.read().await;
        tools.get(tool_id).cloned()
    }
}

impl Default for HealthManager {
    fn default() -> Self {
        Self::new()
    }
}
