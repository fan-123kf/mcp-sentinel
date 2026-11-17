use super::types::Tool;
use crate::config::AuthConfig;
use anyhow::{Context, Result};
use rmcp::{
    model::{CallToolRequestParams, CallToolResponse},
    serve_client,
    service::RunningService,
    transport::{
        streamable_http_client::StreamableHttpClientTransportConfig, StreamableHttpClientTransport,
    },
    RoleClient,
};
use serde_json::{json, Value};
use tracing::debug;

type HttpMcpClient = RunningService<RoleClient, ()>;

pub struct HttpBackend {
    client: HttpMcpClient,
}

impl HttpBackend {
    pub async fn new(url: &str, auth: Option<AuthConfig>) -> Result<Self> {
        let mut transport_config = StreamableHttpClientTransportConfig::with_uri(url);
        if let Some(auth) = auth {
            if auth.auth_type == "bearer" {
                transport_config = transport_config.auth_header(auth.token);
            } else {
                anyhow::bail!("Unsupported HTTP backend auth type: {}", auth.auth_type);
            }
        }

        let transport = StreamableHttpClientTransport::from_config(transport_config);
        let client = serve_client((), transport)
            .await
            .context("MCP Streamable HTTP initialization failed")?;
        Ok(Self { client })
    }

    pub async fn initialize(&self) -> Result<()> {
        // The SDK performs initialize and notifications/initialized in `new`.
        Ok(())
    }

    pub async fn list_tools(&self) -> Result<Vec<Tool>> {
        debug!("Listing tools from HTTP backend via MCP Streamable HTTP");
        let tools = self
            .client
            .peer()
            .list_all_tools()
            .await
            .context("MCP tools/list failed")?;

        let tools = tools
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
            .collect::<Result<Vec<Tool>>>()?;
        debug!("Found {} tools from HTTP backend", tools.len());
        Ok(tools)
    }

    pub async fn call_tool(&self, tool_name: &str, arguments: &Value) -> Result<Value> {
        debug!(tool_name = %tool_name, "Calling HTTP MCP tool");
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
                anyhow::bail!(
                    "Remote MCP tool requires client input, which this gateway cannot relay yet"
                )
            }
            CallToolResponse::Task(_) => {
                anyhow::bail!("Remote MCP tool returned a task, which this gateway cannot poll yet")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{routing::get, Router};
    use rmcp::{
        model::{
            CallToolRequestParams, CallToolResponse, CallToolResult, Implementation,
            ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerConfig,
            Tool as McpTool,
        },
        service::RequestContext,
        transport::{
            streamable_http_server::session::local::LocalSessionManager,
            StreamableHttpServerConfig, StreamableHttpService,
        },
        ErrorData, RoleServer, ServerHandler,
    };
    use serde_json::json;
    use std::{future::Future, sync::Arc};

    #[derive(Clone)]
    struct EchoServer;

    impl ServerHandler for EchoServer {
        fn get_info(&self) -> ServerConfig {
            ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
                .with_server_info(Implementation::new("test-echo", "1.0.0"))
        }

        fn list_tools(
            &self,
            request: Option<PaginatedRequestParams>,
            _context: RequestContext<RoleServer>,
        ) -> impl Future<Output = Result<ListToolsResult, ErrorData>> + Send + '_ {
            async move {
                let schema = json!({
                    "type": "object",
                    "properties": {"value": {"type": "string"}},
                    "required": ["value"]
                })
                .as_object()
                .cloned()
                .expect("object schema");
                let tools = vec![
                    McpTool::new(
                        "echo",
                        "Return the provided value",
                        Arc::new(schema.clone()),
                    ),
                    McpTool::new("echo_extra", "Return the provided value", Arc::new(schema)),
                ];
                let cursor = request.and_then(|params| params.cursor);
                if cursor.is_none() {
                    Ok(ListToolsResult::with_all_items(vec![tools[0].clone()]).with_cursor("next"))
                } else {
                    Ok(ListToolsResult::with_all_items(vec![tools[1].clone()]))
                }
            }
        }

        fn call_tool(
            &self,
            request: CallToolRequestParams,
            _context: RequestContext<RoleServer>,
        ) -> impl Future<Output = Result<CallToolResponse, ErrorData>> + Send + '_ {
            async move {
                let value = request
                    .arguments
                    .and_then(|arguments| arguments.get("value").cloned())
                    .unwrap_or(Value::Null);
                Ok(CallToolResult::structured(json!({"echo": value})).into())
            }
        }
    }

    #[tokio::test]
    async fn streamable_http_backend_initializes_lists_and_calls() {
        let service = StreamableHttpService::new(
            || Ok(EchoServer),
            Arc::new(LocalSessionManager::default()),
            StreamableHttpServerConfig::default(),
        );
        let app = Router::new()
            .route("/health", get(|| async { "ok" }))
            .route_service("/mcp", service);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = shutdown_rx.await;
                })
                .await
                .unwrap();
        });

        let backend = HttpBackend::new(&format!("http://{address}/mcp"), None)
            .await
            .unwrap();
        let tools = backend.list_tools().await.unwrap();
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0].name, "echo");
        assert_eq!(tools[1].name, "echo_extra");
        assert_eq!(tools[0].input_schema["required"][0], "value");

        let result = backend
            .call_tool("echo", &json!({"value": "hello"}))
            .await
            .unwrap();
        assert_eq!(result["structuredContent"]["echo"], "hello");

        let _ = shutdown_tx.send(());
        server.await.unwrap();
    }
}
