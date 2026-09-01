mod http;
mod stdio;
mod types;

pub use types::{BackendManager, Tool, ToolCall, ToolCallResult};

use crate::config::{BackendConfig, Config};
use crate::health::HealthManager;
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn};

pub struct Backend {
    name: String,
    tools: Vec<Tool>,
    backend_type: BackendType,
}

enum BackendType {
    Stdio(stdio::StdioBackend),
    Http(http::HttpBackend),
}

impl BackendManager {
    pub async fn new(config: &Config, health_manager: HealthManager) -> Result<Self> {
        let backends = Arc::new(RwLock::new(HashMap::new()));

        // Initialize all configured backends
        for (name, backend_config) in &config.backends {
            info!("Initializing backend: {}", name);

            match backend_config {
                BackendConfig::Stdio { command, args, env } => {
                    match stdio::StdioBackend::new(command, args, env.clone()).await {
                        Ok(backend) => {
                            // MCP spec: initialize handshake before any other request.
                            // Previously skipped -- broke protocol-strict servers.
                            backend.initialize().await.with_context(|| {
                                format!("Handshake failed for stdio backend {}", name)
                            })?;
                            let tools = backend.list_tools().await?;
                            info!("Backend {} loaded {} tools", name, tools.len());

                            backends.write().await.insert(
                                name.clone(),
                                Backend {
                                    name: name.clone(),
                                    tools,
                                    backend_type: BackendType::Stdio(backend),
                                },
                            );
                        }
                        Err(e) => {
                            warn!("Failed to initialize stdio backend {}: {}", name, e);
                        }
                    }
                }
                BackendConfig::Http { url, auth } => {
                    match http::HttpBackend::new(url, auth.clone()).await {
                        Ok(backend) => {
                            let tools = backend.list_tools().await?;
                            info!("Backend {} loaded {} tools", name, tools.len());

                            backends.write().await.insert(
                                name.clone(),
                                Backend {
                                    name: name.clone(),
                                    tools,
                                    backend_type: BackendType::Http(backend),
                                },
                            );
                        }
                        Err(e) => {
                            warn!("Failed to initialize HTTP backend {}: {}", name, e);
                        }
                    }
                }
            }
        }

        Ok(Self {
            backends,
            health_manager,
        })
    }

    pub async fn list_all_tools(&self) -> Vec<Tool> {
        let backends = self.backends.read().await;
        let mut all_tools = Vec::new();

        for (server_name, backend) in backends.iter() {
            for tool in &backend.tools {
                let mut tool_with_server = tool.clone();
                tool_with_server.tool_id = format!("{}::{}", server_name, tool.name);
                tool_with_server.server_name = Some(server_name.clone());
                all_tools.push(tool_with_server);
            }
        }

        all_tools
    }

    /// Look up MCP annotations for a tool_id ("server::tool"). Returns None
    /// when the backend/tool doesn't exist or the server sent no annotations.
    pub async fn tool_annotations(&self, tool_id: &str) -> Option<serde_json::Value> {
        let parts: Vec<&str> = tool_id.splitn(2, "::").collect();
        if parts.len() != 2 {
            return None;
        }
        let backends = self.backends.read().await;
        let backend = backends.get(parts[0])?;
        backend
            .tools
            .iter()
            .find(|t| t.name == parts[1])?
            .annotations
            .clone()
    }

    pub async fn invoke_tool(&self, tool_call: ToolCall) -> Result<ToolCallResult> {
        let start = std::time::Instant::now();

        // Parse tool_id to get server_name and tool_name
        let parts: Vec<&str> = tool_call.tool_id.split("::").collect();
        if parts.len() != 2 {
            anyhow::bail!("Invalid tool_id format: {}", tool_call.tool_id);
        }

        let server_name = parts[0];
        let tool_name = parts[1];

        let backends = self.backends.read().await;
        let backend = backends
            .get(server_name)
            .ok_or_else(|| anyhow::anyhow!("Backend not found: {}", server_name))?;

        let result = match &backend.backend_type {
            BackendType::Stdio(stdio) => stdio.call_tool(tool_name, &tool_call.arguments).await,
            BackendType::Http(http) => http.call_tool(tool_name, &tool_call.arguments).await,
        };

        let latency_ms = start.elapsed().as_millis() as u64;

        match result {
            Ok(content) => {
                self.health_manager
                    .record_success(&tool_call.tool_id, latency_ms)
                    .await;
                Ok(ToolCallResult::Success {
                    content,
                    latency_ms,
                })
            }
            Err(e) => {
                self.health_manager.record_failure(&tool_call.tool_id).await;
                Ok(ToolCallResult::Error {
                    error: e.to_string(),
                    latency_ms,
                })
            }
        }
    }
}
