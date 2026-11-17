use super::{meta_tools, AppState};
use rmcp::{
    model::{
        CallToolRequestParams, CallToolResponse, CallToolResult, Implementation, ListToolsResult,
        PaginatedRequestParams, ServerCapabilities, ServerConfig,
    },
    service::RequestContext,
    ErrorData, RoleServer, ServerHandler,
};
use serde_json::{json, Value};
use std::future::Future;

#[derive(Clone)]
pub(super) struct GatewayMcpServer {
    state: AppState,
}

impl GatewayMcpServer {
    pub(super) fn new(state: AppState) -> Self {
        Self { state }
    }
}

impl ServerHandler for GatewayMcpServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build()).with_server_info(
            Implementation::new("mcp-sentinel", env!("CARGO_PKG_VERSION")),
        )
    }

    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListToolsResult, ErrorData>> + Send + '_ {
        async move {
            let definitions = meta_tools::meta_tool_definitions();
            let tools: Vec<rmcp::model::Tool> =
                serde_json::from_value(Value::Array(definitions))
                    .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
            Ok(ListToolsResult::with_all_items(tools))
        }
    }

    fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<CallToolResponse, ErrorData>> + Send + '_ {
        async move {
            let params = json!({
                "name": request.name,
                "arguments": request.arguments.map(Value::Object).unwrap_or_else(|| json!({}))
            });
            let result = meta_tools::handle_tools_call(&self.state, Some(params))
                .await
                .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
            let result: CallToolResult = serde_json::from_value(result)
                .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
            Ok(CallToolResponse::Complete(result))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gateway_tool_catalog_conforms_to_mcp_tool_models() {
        let definitions = meta_tools::meta_tool_definitions();
        let tools: Vec<rmcp::model::Tool> =
            serde_json::from_value(Value::Array(definitions)).unwrap();
        assert_eq!(tools.len(), 6);
        assert!(tools
            .iter()
            .all(|tool| tool.input_schema.contains_key("type")));
        assert!(tools.iter().any(|tool| tool.name == "gateway_search_tools"));
    }
}
