use super::AppState;
use crate::backend::{ToolCall, ToolCallResult};
use crate::decision_trace::InvocationTrace;
use crate::governance::{classify_error, is_transient, ToolPolicy};
use crate::health::{generate_cleanup_suggestions, generate_health_report};
use anyhow::Result;
use serde::Deserialize;
use serde_json::{json, Value};
use tracing::{debug, info};

#[derive(Debug, Deserialize)]
struct SearchToolsParams {
    query: String,
    #[serde(default = "default_top_k")]
    top_k: usize,
    server_filter: Option<String>,
}

#[derive(Debug, Deserialize)]
struct InvokeParams {
    tool_id: String,
    arguments: Value,
    #[serde(default = "default_true")]
    allow_fallback: bool,
    #[serde(default)]
    confirmed: bool,
}

#[derive(Debug, Deserialize)]
struct HealthReportParams {
    #[serde(default = "default_scope")]
    scope: String,
    #[serde(default = "default_time_window")]
    time_window_days: u64,
}

#[derive(Debug, Deserialize)]
struct SuggestCleanupParams {
    #[serde(default)]
    aggressive: bool,
}

fn default_top_k() -> usize {
    5
}

fn default_true() -> bool {
    true
}

fn default_scope() -> String {
    "all".to_string()
}

fn default_time_window() -> u64 {
    7
}

pub async fn handle_initialize(state: &AppState) -> Result<Value> {
    info!("Client initializing connection");

    Ok(json!({
        "protocolVersion": "2024-11-05",
        "serverInfo": {
            "name": "mcp-sentinel",
            "version": env!("CARGO_PKG_VERSION")
        },
        "capabilities": {
            "tools": {}
        }
    }))
}

pub async fn handle_tools_list(state: &AppState) -> Result<Value> {
    debug!("Client requesting tools list (meta-tools only)");

    let meta_tools = vec![
        json!({
            "name": "gateway_search_tools",
            "description": "Search available tools by natural language query. Returns top 5 tools ranked by semantic relevance and health score. Unhealthy or zombie tools are automatically deprioritized.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Natural language description of what you need"
                    },
                    "top_k": {
                        "type": "integer",
                        "description": "Max results to return",
                        "default": 5
                    },
                    "server_filter": {
                        "type": "string",
                        "description": "Optional: limit to specific server name"
                    }
                },
                "required": ["query"]
            }
        }),
        json!({
            "name": "gateway_invoke",
            "description": "Invoke a backend tool by its tool_id. Automatically retries on transient failures and falls back to alternative tools if available.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "tool_id": {
                        "type": "string",
                        "description": "tool_id from gateway_search_tools result"
                    },
                    "arguments": {
                        "type": "object",
                        "description": "Arguments matching the tool's input schema"
                    },
                    "allow_fallback": {
                        "type": "boolean",
                        "description": "Reserved for compatible fallback policies; disabled for side-effecting tools",
                        "default": true
                    },
                    "confirmed": {
                        "type": "boolean",
                        "description": "Must be true before executing write or destructive tools",
                        "default": false
                    }
                },
                "required": ["tool_id", "arguments"]
            }
        }),
        json!({
            "name": "gateway_health_report",
            "description": "Return a health summary of all connected MCP servers and their tools. Includes zombie detection and degradation warnings.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "scope": {
                        "type": "string",
                        "enum": ["all", "degraded", "zombie"],
                        "description": "Filter by health status",
                        "default": "all"
                    },
                    "time_window_days": {
                        "type": "integer",
                        "description": "Time window for statistics in days",
                        "default": 7
                    }
                }
            }
        }),
        json!({
            "name": "gateway_get_trace",
            "description": "Retrieve a privacy-preserving decision trace by trace_id. Traces contain routing and execution metadata, never tool arguments or results.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "trace_id": {
                        "type": "string",
                        "description": "trace_id returned by gateway_search_tools or gateway_invoke"
                    }
                },
                "required": ["trace_id"]
            }
        }),
        json!({
            "name": "gateway_suggest_cleanup",
            "description": "Analyze tool usage patterns and suggest which MCP servers or tools to remove to reduce context bloat. Returns actionable recommendations.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "aggressive": {
                        "type": "boolean",
                        "description": "If true, flag tools unused for 3+ days instead of 7",
                        "default": false
                    }
                }
            }
        }),
    ];

    Ok(json!({
        "tools": meta_tools
    }))
}

pub async fn handle_tools_call(state: &AppState, params: Option<Value>) -> Result<Value> {
    let params = params.ok_or_else(|| anyhow::anyhow!("Missing params"))?;

    let tool_name = params
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing tool name"))?;

    let arguments = params.get("arguments").cloned().unwrap_or(json!({}));

    info!("Calling meta-tool: {}", tool_name);

    let result = match tool_name {
        "gateway_search_tools" => handle_search_tools(state, arguments).await?,
        "gateway_invoke" => handle_invoke(state, arguments).await?,
        "gateway_health_report" => handle_health_report(state, arguments).await?,
        "gateway_get_trace" => handle_get_trace(state, arguments).await?,
        "gateway_suggest_cleanup" => handle_suggest_cleanup(state, arguments).await?,
        _ => anyhow::bail!("Unknown meta-tool: {}", tool_name),
    };

    Ok(json!({
        "content": [
            {
                "type": "text",
                "text": serde_json::to_string_pretty(&result)?
            }
        ]
    }))
}

async fn handle_search_tools(state: &AppState, arguments: Value) -> Result<Value> {
    let params: SearchToolsParams = serde_json::from_value(arguments)?;

    debug!(query = %params.query, top_k = params.top_k, "Searching tools");

    let health_weight = state.config.routing.health_weight;
    let mut results = state
        .router
        .search(&params.query, params.top_k, health_weight)
        .await;

    // Apply server filter if specified
    if let Some(ref filter) = params.server_filter {
        results.retain(|r| r.server_name.contains(filter));
    }

    let trace_id = state
        .traces
        .record_search(&params.query, &results, "hybrid_rrf")
        .await;
    let tools_json: Vec<Value> = results
        .into_iter()
        .map(|ranked| {
            let health_hint = if ranked.degraded {
                "degraded"
            } else if ranked.health_score < 0.5 {
                "unhealthy"
            } else {
                "healthy"
            };

            json!({
                "tool_id": ranked.tool_id,
                "name": ranked.tool_name,
                "server": ranked.server_name,
                "description": ranked.description,
                "scores": {
                    "semantic": format!("{:.3}", ranked.semantic_score),
                    "health": format!("{:.3}", ranked.health_score),
                    "final": format!("{:.3}", ranked.final_score)
                },
                "health_hint": health_hint,
                "degraded": ranked.degraded
            })
        })
        .collect();

    Ok(json!({
        "query": params.query,
        "results": tools_json,
        "count": tools_json.len(),
        "trace_id": trace_id
    }))
}

async fn handle_invoke(state: &AppState, arguments: Value) -> Result<Value> {
    let params: InvokeParams = serde_json::from_value(arguments)?;

    // Governance: prefer the server's own MCP annotations (authoritative),
    // fall back to the name heuristic only when annotations are absent.
    let annotations = state
        .backend_manager
        .tool_annotations(&params.tool_id)
        .await;
    let policy = match annotations.as_ref() {
        Some(a) => {
            let p = ToolPolicy::from_annotations(a, &params.tool_id)
                .unwrap_or_else(|| ToolPolicy::infer(&params.tool_id));
            debug!(tool_id = %params.tool_id, side_effect = ?p.side_effect, "policy from server annotations");
            p
        }
        None => {
            let p = ToolPolicy::infer(&params.tool_id);
            debug!(tool_id = %params.tool_id, side_effect = ?p.side_effect, "policy from name heuristic");
            p
        }
    };

    policy
        .authorize(params.confirmed)
        .map_err(anyhow::Error::msg)?;

    info!(tool_id = %params.tool_id, side_effect = ?policy.side_effect, "Invoking tool");
    let is_degraded = state.health_manager.is_degraded(&params.tool_id).await;
    if is_degraded {
        info!(tool_id = %params.tool_id, "Invoking degraded tool");
    }

    let mut attempts = 0;
    let mut result;
    loop {
        attempts += 1;
        result = state
            .backend_manager
            .invoke_tool(ToolCall {
                tool_id: params.tool_id.clone(),
                arguments: params.arguments.clone(),
            })
            .await?;

        let should_retry = matches!(classify_error(&result), Some(category) if policy.retry_safe && is_transient(category))
            && attempts < policy.max_attempts;
        if !should_retry {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100 * attempts as u64)).await;
    }

    let (status, error_category, latency_ms) = match &result {
        ToolCallResult::Success { latency_ms, .. } => ("success", None, *latency_ms),
        ToolCallResult::Error { latency_ms, .. } => ("error", classify_error(&result), *latency_ms),
    };
    let trace_id = state
        .traces
        .record_invocation(InvocationTrace {
            tool_id: params.tool_id.clone(),
            side_effect: policy.side_effect,
            confirmation_required: policy.confirmation_required,
            confirmed: params.confirmed,
            attempts,
            outcome: status.to_string(),
            error_category,
            latency_ms,
        })
        .await;

    match result {
        ToolCallResult::Success {
            content,
            latency_ms,
        } => Ok(json!({
            "status": "success",
            "result": content,
            "latency_ms": latency_ms,
            "attempts": attempts,
            "trace_id": trace_id
        })),
        ToolCallResult::Error { error, latency_ms } => Ok(json!({
            "status": "error",
            "error": error,
            "error_category": error_category,
            "latency_ms": latency_ms,
            "attempts": attempts,
            "trace_id": trace_id,
            "fallback_allowed": params.allow_fallback && policy.retry_safe
        })),
    }
}

async fn handle_health_report(state: &AppState, arguments: Value) -> Result<Value> {
    let params: HealthReportParams = serde_json::from_value(arguments)?;

    let report = generate_health_report(
        &state.health_manager,
        state.storage.as_ref(),
        params.time_window_days,
    )
    .await?;

    Ok(json!({
        "report": report,
        "format": "markdown"
    }))
}

async fn handle_get_trace(state: &AppState, arguments: Value) -> Result<Value> {
    let trace_id = arguments
        .get("trace_id")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("Missing trace_id"))?;
    let trace = state
        .traces
        .get(trace_id)
        .await
        .ok_or_else(|| anyhow::anyhow!("Trace not found: {}", trace_id))?;
    Ok(serde_json::to_value(trace)?)
}

async fn handle_suggest_cleanup(state: &AppState, arguments: Value) -> Result<Value> {
    let params: SuggestCleanupParams = serde_json::from_value(arguments)?;

    let suggestions = generate_cleanup_suggestions(
        &state.health_manager,
        state.storage.as_ref(),
        params.aggressive,
    )
    .await?;

    Ok(suggestions)
}
