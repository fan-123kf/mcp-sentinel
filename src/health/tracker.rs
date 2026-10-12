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
    zombie_threshold_days: u64,
    storage: Option<Arc<StorageManager>>,
}

impl HealthManager {
    pub fn new() -> Self {
        Self {
            tools: Arc::new(RwLock::new(HashMap::new())),
            consecutive_failure_limit: 5,
            zombie_threshold_days: 7,
            storage: None,
        }
    }

    pub fn with_failure_limit(mut self, limit: u32) -> Self {
        self.consecutive_failure_limit = limit;
        self
    }

    pub fn with_zombie_threshold(mut self, days: u64) -> Self {
        self.zombie_threshold_days = days;
        self
    }

    pub fn with_storage(mut self, storage: Arc<StorageManager>) -> Self {
        self.storage = Some(storage);
        self
    }

    /// Restore the in-memory view from persisted calls after a restart. This
    /// keeps routing and reports consistent with the SQLite history instead
    /// of treating every tool as brand new until its next invocation.
    pub async fn restore_from_storage(&self, tool_ids: &[String]) {
        let Some(storage) = &self.storage else { return };
        let registered = storage.get_all_registered_tools().await.unwrap_or_default();
        for tool_id in tool_ids {
            let Ok(calls) = storage.get_recent_calls(tool_id, 10_000).await else {
                continue;
            };
            let mut health = ToolHealth::new(tool_id.clone());
            if let Some(tool) = registered.iter().find(|t| &t.tool_id == tool_id) {
                health.first_seen = tool.first_seen.into();
            }
            for call in &calls {
                if call.success {
                    health.success_count += 1;
                    health.total_latency_ms =
                        health.total_latency_ms.saturating_add(call.latency_ms);
                } else {
                    health.failure_count += 1;
                }
            }
            health.call_count_7d = storage.get_call_count_window(tool_id, 7).await.unwrap_or(0);
            health.latency_p95 = storage.get_p95_latency(tool_id, 1000).await.unwrap_or(0.0);
            health.consecutive_failures = calls.iter().take_while(|c| !c.success).count() as u32;
            if let Some(last) = calls.first() {
                health.last_call = Some(last.called_at.into());
                if last.success {
                    health.last_success = Some(last.called_at.into());
                } else {
                    health.last_failure = Some(last.called_at.into());
                }
            }
            health.compute_health_score();
            self.tools.write().await.insert(tool_id.clone(), health);
        }
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

    pub async fn record_failure_with_details(
        &self,
        tool_id: &str,
        latency_ms: u64,
        error_type: Option<String>,
    ) {
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
                    latency_ms,
                    error_type,
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
        let mut tools = self.tools.write().await;
        for health in tools.values_mut() {
            health.compute_health_score();
        }
        tools.get(tool_id).map(|health| HealthScore {
            tool_id: health.tool_id.clone(),
            health_score: health.health_score,
            degraded: health.is_degraded(self.consecutive_failure_limit),
            zombie: health.idle_days() >= self.zombie_threshold_days,
        })
    }

    pub async fn get_all_scores(&self) -> Vec<HealthScore> {
        let mut tools = self.tools.write().await;
        for health in tools.values_mut() {
            health.compute_health_score();
        }
        tools
            .values()
            .map(|health| HealthScore {
                tool_id: health.tool_id.clone(),
                health_score: health.health_score,
                degraded: health.is_degraded(self.consecutive_failure_limit),
                zombie: health.idle_days() >= self.zombie_threshold_days,
            })
            .collect()
    }

    pub async fn is_degraded(&self, tool_id: &str) -> bool {
        let mut tools = self.tools.write().await;
        for health in tools.values_mut() {
            health.compute_health_score();
        }
        tools
            .get(tool_id)
            .map(|h| h.is_degraded(self.consecutive_failure_limit))
            .unwrap_or(false)
    }

    pub async fn get_detailed_health(&self, tool_id: &str) -> Option<ToolHealth> {
        let mut tools = self.tools.write().await;
        for health in tools.values_mut() {
            health.compute_health_score();
        }
        tools.get(tool_id).cloned()
    }
}

impl Default for HealthManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn restores_history_and_keeps_new_tools_discoverable() {
        let path = std::env::temp_dir().join(format!("sentinel-restore-{}.db", std::process::id()));
        let storage = Arc::new(StorageManager::new(path.to_str().unwrap()).await.unwrap());
        storage
            .record_tool_call("s::read", "s", "read", true, 123, None)
            .await
            .unwrap();
        storage
            .record_tool_call("s::read", "s", "read", false, 456, Some("Timeout".into()))
            .await
            .unwrap();
        let manager = HealthManager::new().with_storage(storage);
        manager
            .restore_from_storage(&["s::read".into(), "s::new".into()])
            .await;
        let health = manager.get_detailed_health("s::read").await.unwrap();
        assert!(health.success_count >= 1);
        assert_eq!(health.consecutive_failures, 1);
        assert_eq!(health.latency_p95, 123.0);
        assert!(!manager.get_health_score("s::new").await.unwrap().zombie);
        // Release SQLite handles before removing the temporary file on Windows.
        drop(manager);
        let _ = std::fs::remove_file(path);
    }
}
