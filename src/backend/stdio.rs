use super::types::Tool;
use anyhow::{Context, Result};
use rmcp::{
    model::{CallToolRequestParams, CallToolResponse},
    serve_client,
    service::RunningService,
    transport::TokioChildProcess,
    RoleClient,
};
use serde_json::Value;
use tokio::process::Command;
use tracing::debug;

type StdioMcpClient = RunningService<RoleClient, ()>;

pub struct StdioBackend {
    client: StdioMcpClient,
}

impl StdioBackend {
    pub async fn new(
        command: &str,
        args: &[String],
        env: Option<std::collections::HashMap<String, String>>,
    ) -> Result<Self> {
        let mut process = Command::new(command);
        process.args(args);
        if let Some(env_vars) = env {
            process.envs(env_vars);
        }

        let (transport, _stderr) = TokioChildProcess::builder(process)
            .spawn()
            .with_context(|| format!("Failed to spawn MCP process: {command} {args:?}"))?;
        let client = serve_client((), transport)
            .await
            .with_context(|| format!("MCP stdio initialization failed: {command}"))?;

        Ok(Self { client })
    }

    pub async fn initialize(&self) -> Result<()> {
        // The SDK completes initialize and notifications/initialized in `new`.
        Ok(())
    }

    pub async fn list_tools(&self) -> Result<Vec<Tool>> {
        debug!("Listing tools from MCP stdio backend");
        let tools = self
            .client
            .peer()
            .list_all_tools()
            .await
            .context("MCP tools/list failed")?;

        tools
            .into_iter()
            .map(|tool| {
                let mut value = serde_json::to_value(tool)?;
                if let Some(object) = value.as_object_mut() {
                    object
                        .entry("description")
                        .or_insert_with(|| Value::String(String::new()));
                }
                serde_json::from_value(value).context("Invalid MCP tool definition")
            })
            .collect()
    }

    pub async fn call_tool(&self, tool_name: &str, arguments: &Value) -> Result<Value> {
        debug!(tool_name = %tool_name, "Calling MCP stdio tool");
        let arguments = arguments
            .as_object()
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("MCP tool arguments must be a JSON object"))?;
        let params = CallToolRequestParams::new(tool_name.to_owned()).with_arguments(arguments);
        let response = self
            .client
            .peer()
            .call_tool_once(params)
            .await
            .context("MCP tools/call failed")?;

        match response {
            CallToolResponse::Complete(result) => {
                serde_json::to_value(result).context("Failed to serialize MCP tool result")
            }
            CallToolResponse::InputRequired(_) => {
                anyhow::bail!("MCP tool requires client input, which this gateway cannot relay yet")
            }
            CallToolResponse::Task(_) => {
                anyhow::bail!("MCP tool returned a task, which this gateway cannot poll yet")
            }
        }
    }
}
