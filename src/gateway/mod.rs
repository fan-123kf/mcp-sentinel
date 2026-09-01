use crate::backend::BackendManager;
use crate::config::Config;
use crate::decision_trace::DecisionTraceStore;
use crate::health::HealthManager;
use crate::router::SemanticRouter;
use crate::storage::StorageManager;
use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Json},
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;
use tracing::{error, info};

mod meta_tools;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub backend_manager: Arc<BackendManager>,
    pub router: Arc<SemanticRouter>,
    pub health_manager: HealthManager,
    pub storage: Option<Arc<StorageManager>>,
    pub traces: DecisionTraceStore,
}

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    jsonrpc: String,
    id: Option<Value>,
    method: String,
    params: Option<Value>,
}

#[derive(Debug, Serialize)]
struct JsonRpcResponse {
    jsonrpc: String,
    id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize)]
struct JsonRpcError {
    code: i32,
    message: String,
}

pub async fn start_gateway(
    config: Config,
    backend_manager: BackendManager,
    router: SemanticRouter,
    health_manager: HealthManager,
    storage: Option<Arc<StorageManager>>,
) -> anyhow::Result<()> {
    let state = AppState {
        config: Arc::new(config.clone()),
        backend_manager: Arc::new(backend_manager),
        router: Arc::new(router),
        health_manager,
        storage,
        traces: DecisionTraceStore::default(),
    };

    let app = Router::new()
        .route("/", get(root_handler))
        .route("/health", get(health_handler))
        .route("/mcp", post(mcp_handler))
        .layer(CorsLayer::new().allow_origin(Any).allow_methods(Any))
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let addr = format!("0.0.0.0:{}", config.gateway.port);
    info!("🚀 mcp-sentinel gateway listening on http://{}", addr);
    info!("   MCP endpoint: http://{}/mcp", addr);
    info!("   Health check: http://{}/health", addr);

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

async fn root_handler() -> impl IntoResponse {
    Json(json!({
        "service": "mcp-sentinel",
        "version": env!("CARGO_PKG_VERSION"),
        "description": "Intelligent MCP gateway with health-driven adaptive routing",
        "endpoints": {
            "mcp": "/mcp",
            "health": "/health"
        }
    }))
}

async fn health_handler(State(state): State<AppState>) -> impl IntoResponse {
    let health_scores = state.health_manager.get_all_scores().await;
    let healthy_count = health_scores
        .iter()
        .filter(|h| !h.degraded && !h.zombie)
        .count();
    let degraded_count = health_scores.iter().filter(|h| h.degraded).count();
    let zombie_count = health_scores.iter().filter(|h| h.zombie).count();

    Json(json!({
        "status": "healthy",
        "tools": {
            "total": health_scores.len(),
            "healthy": healthy_count,
            "degraded": degraded_count,
            "zombie": zombie_count
        }
    }))
}

async fn mcp_handler(
    State(state): State<AppState>,
    Json(request): Json<JsonRpcRequest>,
) -> impl IntoResponse {
    info!("Received MCP request: {}", request.method);

    let result = match request.method.as_str() {
        "initialize" => meta_tools::handle_initialize(&state).await,
        "tools/list" => meta_tools::handle_tools_list(&state).await,
        "tools/call" => meta_tools::handle_tools_call(&state, request.params).await,
        _ => Err(anyhow::anyhow!("Unknown method: {}", request.method)),
    };

    let response = match result {
        Ok(result) => JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id: request.id,
            result: Some(result),
            error: None,
        },
        Err(e) => {
            error!("Error handling request: {}", e);
            JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                id: request.id,
                result: None,
                error: Some(JsonRpcError {
                    code: -32603,
                    message: e.to_string(),
                }),
            }
        }
    };

    (StatusCode::OK, Json(response))
}
