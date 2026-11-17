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
    #[serde(default = "default_bind_address")]
    pub bind_address: String,
    #[serde(default)]
    pub auth_token: Option<String>,
    #[serde(default)]
    pub allowed_origins: Vec<String>,
    #[serde(default = "default_allowed_hosts")]
    pub allowed_hosts: Vec<String>,
}

fn default_bind_address() -> String {
    "127.0.0.1".to_string()
}

fn default_allowed_hosts() -> Vec<String> {
    vec!["localhost".into(), "127.0.0.1".into(), "::1".into()]
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

        config.validate()?;
        Ok(config)
    }

    /// Reject invalid policies at startup rather than silently misrouting calls.
    pub fn validate(&self) -> Result<()> {
        anyhow::ensure!(self.gateway.port > 0, "gateway.port must be non-zero");
        let bind_ip: std::net::IpAddr = self
            .gateway
            .bind_address
            .parse()
            .context("gateway.bind_address must be an IP address")?;
        anyhow::ensure!(
            bind_ip.is_loopback()
                || self
                    .gateway
                    .auth_token
                    .as_ref()
                    .is_some_and(|token| !token.trim().is_empty()),
            "gateway.auth_token is required when binding to a non-loopback address"
        );
        anyhow::ensure!(
            self.gateway
                .allowed_origins
                .iter()
                .all(|origin| !origin.trim().is_empty()),
            "gateway.allowed_origins cannot contain empty values"
        );
        anyhow::ensure!(
            self.gateway
                .allowed_hosts
                .iter()
                .all(|host| !host.trim().is_empty()),
            "gateway.allowed_hosts cannot contain empty values"
        );
        anyhow::ensure!(
            matches!(self.routing.strategy.as_str(), "tfidf" | "hybrid"),
            "routing.strategy must be tfidf or hybrid"
        );
        anyhow::ensure!(self.routing.top_k > 0, "routing.top_k must be positive");
        anyhow::ensure!(
            (0.0..=1.0).contains(&self.routing.health_weight),
            "routing.health_weight must be between 0 and 1"
        );
        anyhow::ensure!(
            (0.0..=1.0).contains(&self.health.degraded_score_penalty),
            "health.degraded_score_penalty must be between 0 and 1"
        );
        anyhow::ensure!(
            self.health.consecutive_failure_limit > 0,
            "health.consecutive_failure_limit must be positive"
        );
        anyhow::ensure!(
            self.health.zombie_threshold_days > 0,
            "health.zombie_threshold_days must be positive"
        );
        anyhow::ensure!(
            self.storage.retention_days > 0,
            "storage.retention_days must be positive"
        );
        for (name, backend) in &self.backends {
            anyhow::ensure!(
                !name.is_empty() && !name.contains("::"),
                "backend names must be non-empty and cannot contain ::"
            );
            if let BackendConfig::Http { url, auth } = backend {
                let parsed = url::Url::parse(url)
                    .with_context(|| format!("backend {name} has an invalid HTTP URL"))?;
                anyhow::ensure!(
                    matches!(parsed.scheme(), "http" | "https") && parsed.host().is_some(),
                    "backend {name} URL must use http or https and include a host"
                );
                anyhow::ensure!(
                    parsed.username().is_empty() && parsed.password().is_none(),
                    "backend {name} URL must not embed credentials"
                );
                if let Some(auth) = auth {
                    anyhow::ensure!(
                        auth.auth_type == "bearer",
                        "backend {name} auth.type must be bearer"
                    );
                    anyhow::ensure!(
                        !auth.token.trim().is_empty(),
                        "backend {name} bearer token cannot be empty"
                    );
                }
            }
        }
        Ok(())
    }
}

impl Default for GatewayConfig {
    fn default() -> Self {
        Self {
            port: 3000,
            web_ui: true,
            log_level: "info".to_string(),
            bind_address: default_bind_address(),
            auth_token: None,
            allowed_origins: Vec::new(),
            allowed_hosts: default_allowed_hosts(),
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
    fn rejects_invalid_routing_policy() {
        let mut config: Config = toml::from_str(include_str!("../examples/minimal.toml")).unwrap();
        assert!(config.validate().is_ok());
        config.routing.health_weight = f64::NAN;
        assert!(config.validate().is_err());
        config.routing.health_weight = 0.4;
        config.routing.strategy = "typo".into();
        assert!(config.validate().is_err());
        config.routing.strategy = "tfidf".into();
        config.routing.top_k = 0;
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_config_defaults() {
        let gateway = GatewayConfig::default();
        assert_eq!(gateway.port, 3000);
        assert_eq!(gateway.log_level, "info");
        assert_eq!(gateway.bind_address, "127.0.0.1");
        assert!(gateway.auth_token.is_none());

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

    #[test]
    fn remote_bind_requires_authentication() {
        let mut config: Config = toml::from_str(include_str!("../examples/minimal.toml")).unwrap();
        config.gateway.bind_address = "0.0.0.0".into();
        assert!(config.validate().is_err());

        config.gateway.auth_token = Some("secret".into());
        assert!(config.validate().is_ok());

        config.gateway.auth_token = Some("  ".into());
        assert!(config.validate().is_err());
    }

    #[test]
    fn bind_address_must_be_an_ip_address() {
        let mut config: Config = toml::from_str(include_str!("../examples/minimal.toml")).unwrap();
        config.gateway.bind_address = "localhost".into();
        assert!(config.validate().is_err());
    }

    #[test]
    fn validates_http_backend_urls_and_authentication() {
        let mut config: Config = toml::from_str(include_str!("../examples/minimal.toml")).unwrap();
        config.backends.insert(
            "remote".into(),
            BackendConfig::Http {
                url: "https://mcp.example.com/mcp".into(),
                auth: Some(AuthConfig {
                    auth_type: "bearer".into(),
                    token: "secret".into(),
                }),
            },
        );
        assert!(config.validate().is_ok());

        if let Some(BackendConfig::Http { url, .. }) = config.backends.get_mut("remote") {
            *url = "file:///etc/passwd".into();
        }
        assert!(config.validate().is_err());

        if let Some(BackendConfig::Http { url, auth }) = config.backends.get_mut("remote") {
            *url = "https://mcp.example.com/mcp".into();
            *auth = Some(AuthConfig {
                auth_type: "basic".into(),
                token: "secret".into(),
            });
        }
        assert!(config.validate().is_err());
    }
}
