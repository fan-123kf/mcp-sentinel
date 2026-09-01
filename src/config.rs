use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    pub gateway: GatewayConfig,
    pub routing: RoutingConfig,
    pub health: HealthConfig,
    pub storage: StorageConfig,
    pub backends: HashMap<String, BackendConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GatewayConfig {
    pub port: u16,
    pub web_ui: bool,
    pub log_level: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RoutingConfig {
    pub strategy: String,
    pub top_k: usize,
    pub health_weight: f64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HealthConfig {
    pub zombie_threshold_days: u64,
    pub consecutive_failure_limit: u32,
    pub degraded_score_penalty: f64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct StorageConfig {
    pub db_path: String,
    pub retention_days: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "transport")]
pub enum BackendConfig {
    #[serde(rename = "stdio")]
    Stdio {
        command: String,
        args: Vec<String>,
        env: Option<HashMap<String, String>>,
    },
    #[serde(rename = "http")]
    Http {
        url: String,
        auth: Option<AuthConfig>,
    },
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AuthConfig {
    #[serde(rename = "type")]
    pub auth_type: String,
    pub token: String,
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read config file: {}", path.display()))?;

        let expanded =
            shellexpand::env(&content).with_context(|| "Failed to expand environment variables")?;

        let config: Config = toml::from_str(&expanded)
            .with_context(|| format!("Failed to parse config file: {}", path.display()))?;

        Ok(config)
    }
}

impl Default for GatewayConfig {
    fn default() -> Self {
        Self {
            port: 3000,
            web_ui: true,
            log_level: "info".to_string(),
        }
    }
}

impl Default for RoutingConfig {
    fn default() -> Self {
        Self {
            strategy: "tfidf".to_string(),
            top_k: 5,
            health_weight: 0.4,
        }
    }
}

impl Default for HealthConfig {
    fn default() -> Self {
        Self {
            zombie_threshold_days: 7,
            consecutive_failure_limit: 5,
            degraded_score_penalty: 0.1,
        }
    }
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            db_path: "~/.config/mcp-sentinel/sentinel.db".to_string(),
            retention_days: 30,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_defaults() {
        let gateway = GatewayConfig::default();
        assert_eq!(gateway.port, 3000);
        assert_eq!(gateway.log_level, "info");

        let routing = RoutingConfig::default();
        assert_eq!(routing.strategy, "tfidf");
        assert_eq!(routing.top_k, 5);
        assert_eq!(routing.health_weight, 0.4);

        let health = HealthConfig::default();
        assert_eq!(health.zombie_threshold_days, 7);
        assert_eq!(health.consecutive_failure_limit, 5);

        let storage = StorageConfig::default();
        assert_eq!(storage.retention_days, 30);
    }

    #[test]
    fn test_config_parsing() {
        let config_str = r#"
[gateway]
port = 3001
web_ui = true
log_level = "debug"

[routing]
strategy = "tfidf"
top_k = 10
health_weight = 0.5

[health]
zombie_threshold_days = 14
consecutive_failure_limit = 3
degraded_score_penalty = 0.2

[storage]
db_path = "/tmp/test.db"
retention_days = 60

[backends.test]
transport = "stdio"
command = "echo"
args = ["hello"]
        "#;

        let config: Config = toml::from_str(config_str).unwrap();
        assert_eq!(config.gateway.port, 3001);
        assert_eq!(config.routing.top_k, 10);
        assert_eq!(config.health.zombie_threshold_days, 14);
        assert_eq!(config.storage.retention_days, 60);
        assert_eq!(config.backends.len(), 1);
    }
}
