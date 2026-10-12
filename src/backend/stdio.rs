use super::types::Tool;
use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::process::{Child, ChildStdin};
use tokio::sync::{Mutex, RwLock};
use tracing::debug;

pub struct StdioBackend {
    // Keep the child alive; kill_on_drop terminates it with the backend.
    _process: Arc<Mutex<Child>>,
    stdin_writer: Arc<Mutex<ChildStdin>>,
    response_channels: Arc<RwLock<HashMap<u64, tokio::sync::oneshot::Sender<Value>>>>,
    next_id: Arc<Mutex<u64>>,
}

impl StdioBackend {
    pub async fn new(
        command: &str,
        args: &[String],
        env: Option<HashMap<String, String>>,
    ) -> Result<Self> {
        let mut cmd = Command::new(command);
        cmd.args(args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            // Inherit stderr so a verbose backend cannot fill an unread pipe.
            .stderr(std::process::Stdio::inherit())
            .kill_on_drop(true);

        if let Some(env_vars) = env {
            for (key, value) in env_vars {
                cmd.env(key, value);
            }
        }

        let mut child = cmd
            .spawn()
            .with_context(|| format!("Failed to spawn process: {} {:?}", command, args))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow::anyhow!("Failed to capture stdin"))?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("Failed to capture stdout"))?;

        let response_channels = Arc::new(RwLock::new(HashMap::<
            u64,
            tokio::sync::oneshot::Sender<Value>,
        >::new()));
        let response_channels_clone = response_channels.clone();

        // Spawn task to read responses from stdout
        tokio::spawn(async move {
            let reader = BufReader::new(stdout);
            let mut lines = reader.lines();

            while let Ok(Some(line)) = lines.next_line().await {
                if let Ok(response) = serde_json::from_str::<Value>(&line) {
                    if let Some(id) = response.get("id").and_then(|v| v.as_u64()) {
                        let mut channels = response_channels_clone.write().await;
                        if let Some(sender) = channels.remove(&id) {
                            let _ = sender.send(response);
                        }
                    }
                }
            }
            // EOF means no pending request can receive a response.
            response_channels_clone.write().await.clear();
        });

        Ok(Self {
            _process: Arc::new(Mutex::new(child)),
            stdin_writer: Arc::new(Mutex::new(stdin)),
            response_channels,
            next_id: Arc::new(Mutex::new(1)),
        })
    }

    /// Perform the MCP initialize handshake required by the spec before any
    /// other request. Previously skipped, which broke protocol-strict servers
    /// (and emitted "Unknown method" errors for the initialized notification).
    pub async fn initialize(&self) -> Result<Value> {
        // 1) initialize request
        let result = self
            .send_request(
                "initialize",
                Some(json!({
                    "protocolVersion": "2024-11-05",
                    "capabilities": {},
                    "clientInfo": { "name": "mcp-sentinel", "version": env!("CARGO_PKG_VERSION") }
                })),
            )
            .await?;

        // 2) initialized notification (no id, no response expected)
        {
            let notification = json!({
                "jsonrpc": "2.0",
                "method": "notifications/initialized"
            });
            let mut stdin = self.stdin_writer.lock().await;
            stdin
                .write_all(serde_json::to_string(&notification)?.as_bytes())
                .await
                .context("Failed to write initialized notification")?;
            stdin
                .write_all(b"\n")
                .await
                .context("Failed to write newline")?;
            stdin.flush().await.context("Failed to flush stdin")?;
        }

        debug!("stdio backend initialized: {:?}", result.get("serverInfo"));
        Ok(result)
    }

    async fn send_request(&self, method: &str, params: Option<Value>) -> Result<Value> {
        self.send_request_with_timeout(method, params, std::time::Duration::from_secs(30))
            .await
    }

    async fn send_request_with_timeout(
        &self,
        method: &str,
        params: Option<Value>,
        timeout: std::time::Duration,
    ) -> Result<Value> {
        let mut id_counter = self.next_id.lock().await;
        let id = *id_counter;
        *id_counter += 1;
        drop(id_counter);

        let request = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params.unwrap_or(json!({}))
        });

        let (tx, rx) = tokio::sync::oneshot::channel();
        {
            let mut channels = self.response_channels.write().await;
            channels.insert(id, tx);
        }

        let request_str = serde_json::to_string(&request)?;
        let write_result: Result<()> = async {
            let mut stdin = self.stdin_writer.lock().await;
            stdin
                .write_all(request_str.as_bytes())
                .await
                .context("Failed to write to stdin")?;
            stdin
                .write_all(b"\n")
                .await
                .context("Failed to write newline")?;
            stdin.flush().await.context("Failed to flush stdin")?;
            Ok(())
        }
        .await;
        if let Err(error) = write_result {
            self.response_channels.write().await.remove(&id);
            return Err(error);
        }

        // Wait for response with timeout
        let response = tokio::time::timeout(timeout, rx).await;
        self.response_channels.write().await.remove(&id);
        let response = response
            .context("Request timeout")?
            .context("Channel closed")?;

        if let Some(error) = response.get("error") {
            anyhow::bail!("MCP error: {}", error);
        }

        Ok(response.get("result").cloned().unwrap_or(Value::Null))
    }

    pub async fn list_tools(&self) -> Result<Vec<Tool>> {
        debug!("Listing tools from stdio backend");
        let response = self.send_request("tools/list", None).await?;

        let tools_array = response
            .get("tools")
            .and_then(|v| v.as_array())
            .ok_or_else(|| anyhow::anyhow!("Invalid tools/list response"))?;

        let tools: Vec<Tool> = serde_json::from_value(Value::Array(tools_array.clone()))?;
        debug!("Found {} tools", tools.len());
        Ok(tools)
    }

    pub async fn call_tool(&self, tool_name: &str, arguments: &Value) -> Result<Value> {
        debug!(tool_name = %tool_name, "Calling tool");

        let params = json!({
            "name": tool_name,
            "arguments": arguments
        });

        let response = self.send_request("tools/call", Some(params)).await?;

        Ok(response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn removes_pending_requests_after_timeout() {
        // A child that consumes stdin but never responds exercises timeout cleanup.
        #[cfg(windows)]
        let (command, args) = (
            "powershell",
            vec![
                "-NoProfile",
                "-Command",
                "$null = [Console]::In.ReadToEnd()",
            ],
        );
        #[cfg(not(windows))]
        let (command, args) = ("sh", vec!["-c", "cat >/dev/null"]);
        let args: Vec<String> = args.into_iter().map(str::to_string).collect();
        let backend = StdioBackend::new(command, &args, None).await.unwrap();
        let result = backend
            .send_request_with_timeout("tools/list", None, std::time::Duration::from_millis(50))
            .await;
        assert!(result.is_err());
        assert!(backend.response_channels.read().await.is_empty());
    }
}
