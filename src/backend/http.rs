use super::types::Tool;
use crate::config::AuthConfig;
use anyhow::{Context, Result};
use reqwest::Client;
use serde_json::{json, Value};
use tracing::debug;

pub struct HttpBackend {
    client: Client,
    base_url: String,
    auth: Option<AuthConfig>,
}

impl HttpBackend {
    pub async fn new(url: &str, auth: Option<AuthConfig>) -> Result<Self> {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .context("Failed to create HTTP client")?;

        Ok(Self {
            client,
            base_url: url.to_string(),
            auth,
        })
    }

    async fn send_request(&self, method: &str, params: Option<Value>) -> Result<Value> {
        let request = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": method,
            "params": params.unwrap_or(json!({}))
        });

        let mut req = self.client.post(&self.base_url).json(&request);

        if let Some(auth) = &self.auth {
            if auth.auth_type == "bearer" {
                req = req.bearer_auth(&auth.token);
            }
        }

        let response = req.send().await.context("HTTP request failed")?;

        let json_response: Value = response.json().await.context("Failed to parse response")?;

        if let Some(error) = json_response.get("error") {
            anyhow::bail!("MCP error: {}", error);
        }

        Ok(json_response
            .get("result")
            .cloned()
            .unwrap_or(Value::Null))
    }

    pub async fn list_tools(&self) -> Result<Vec<Tool>> {
        debug!("Listing tools from HTTP backend: {}", self.base_url);
        let response = self.send_request("tools/list", None).await?;

        let tools_array = response
            .get("tools")
            .and_then(|v| v.as_array())
            .ok_or_else(|| anyhow::anyhow!("Invalid tools/list response"))?;

        let tools: Vec<Tool> = serde_json::from_value(Value::Array(tools_array.clone()))?;
        debug!("Found {} tools from HTTP backend", tools.len());
        Ok(tools)
    }

    pub async fn call_tool(&self, tool_name: &str, arguments: &Value) -> Result<Value> {
        debug!(tool_name = %tool_name, "Calling HTTP tool");

        let params = json!({
            "name": tool_name,
            "arguments": arguments
        });

        let response = self.send_request("tools/call", Some(params)).await?;

        Ok(response
            .get("content")
            .cloned()
            .unwrap_or(Value::Null))
    }
}
