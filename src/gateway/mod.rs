use crate::backend::BackendManager;
use crate::config::Config;
use crate::decision_trace::DecisionTraceStore;
use crate::health::HealthManager;
use crate::router::SemanticRouter;
use crate::storage::StorageManager;
use axum::{
    body::Body,
    extract::{Request, State},
    http::{header, HeaderMap, HeaderName, HeaderValue, Method, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Json, Response},
    routing::get,
    Router,
};
use rmcp::transport::{
    streamable_http_server::session::local::LocalSessionManager, StreamableHttpServerConfig,
    StreamableHttpService,
};
use serde_json::{json, Value};
use std::sync::Arc;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::trace::TraceLayer;
use tracing::info;

mod mcp_server;
pub(crate) mod meta_tools;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub backend_manager: Arc<BackendManager>,
    pub router: Arc<SemanticRouter>,
    pub health_manager: HealthManager,
    pub storage: Option<Arc<StorageManager>>,
    pub traces: DecisionTraceStore,
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
    let app = build_router(config.gateway.clone(), state)?;

    let bind_ip: std::net::IpAddr = config.gateway.bind_address.parse()?;
    let addr = std::net::SocketAddr::new(bind_ip, config.gateway.port);
    info!("🚀 mcp-sentinel gateway listening on http://{}", addr);
    info!("   MCP endpoint: http://{}/mcp", addr);
    info!("   Health check: http://{}/health", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;

    Ok(())
}

fn build_router(config: crate::config::GatewayConfig, state: AppState) -> anyhow::Result<Router> {
    let allowed_origins = config
        .allowed_origins
        .iter()
        .map(|origin| origin.parse::<HeaderValue>())
        .collect::<Result<Vec<_>, _>>()?;
    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::list(allowed_origins))
        .allow_methods([Method::POST, Method::GET, Method::DELETE])
        .allow_headers([
            header::ACCEPT,
            header::AUTHORIZATION,
            header::CONTENT_TYPE,
            HeaderName::from_static("mcp-protocol-version"),
            HeaderName::from_static("mcp-session-id"),
            HeaderName::from_static("last-event-id"),
        ])
        .expose_headers([
            HeaderName::from_static("mcp-protocol-version"),
            HeaderName::from_static("mcp-session-id"),
        ]);

    let mcp_service = StreamableHttpService::new(
        {
            let state = state.clone();
            move || Ok(mcp_server::GatewayMcpServer::new(state.clone()))
        },
        Arc::new(LocalSessionManager::default()),
        StreamableHttpServerConfig::default()
            .with_allowed_hosts(config.allowed_hosts.clone())
            .with_allowed_origins(config.allowed_origins.clone()),
    );
    let mcp_router: Router<AppState> = Router::new()
        .route_service("/mcp", mcp_service)
        .route_layer(middleware::from_fn_with_state(
            config.clone(),
            authorize_mcp_middleware,
        ))
        .with_state(state.clone());

    let app = Router::new()
        .route("/", get(root_handler))
        .route("/health", get(health_handler))
        .merge(mcp_router)
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    Ok(app)
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

    let status = if degraded_count > 0 {
        "degraded"
    } else {
        "healthy"
    };
    Json(json!({
        "status": status,
        "tools": {
            "total": health_scores.len(),
            "healthy": healthy_count,
            "degraded": degraded_count,
            "zombie": zombie_count
        }
    }))
}

async fn authorize_mcp_middleware(
    State(config): State<crate::config::GatewayConfig>,
    request: Request<Body>,
    next: Next,
) -> Response {
    if let Err(status) = authorize_request(request.headers(), &config) {
        return (status, "MCP gateway authorization failed").into_response();
    }
    next.run(request).await
}

fn authorize_request(
    headers: &HeaderMap,
    config: &crate::config::GatewayConfig,
) -> Result<(), StatusCode> {
    if let Some(expected) = config
        .auth_token
        .as_deref()
        .filter(|token| !token.trim().is_empty())
    {
        let supplied = headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
            .ok_or(StatusCode::UNAUTHORIZED)?;
        if !constant_time_eq(supplied.as_bytes(), expected.as_bytes()) {
            return Err(StatusCode::UNAUTHORIZED);
        }
    }

    if let Some(origin) = headers.get(axum::http::header::ORIGIN) {
        let origin = origin.to_str().map_err(|_| StatusCode::FORBIDDEN)?;
        if !config
            .allowed_origins
            .iter()
            .any(|allowed| allowed == origin)
        {
            return Err(StatusCode::FORBIDDEN);
        }
    }
    Ok(())
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    let mut difference = left.len() ^ right.len();
    for index in 0..left.len().max(right.len()) {
        difference |= usize::from(
            left.get(index).copied().unwrap_or(0) ^ right.get(index).copied().unwrap_or(0),
        );
    }
    difference == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::GatewayConfig;
    use axum::http::HeaderValue;

    #[test]
    fn enforces_bearer_token_when_configured() {
        let mut config = GatewayConfig::default();
        config.auth_token = Some("test-secret".into());
        let mut headers = HeaderMap::new();

        assert_eq!(
            authorize_request(&headers, &config),
            Err(StatusCode::UNAUTHORIZED)
        );
        headers.insert(
            axum::http::header::AUTHORIZATION,
            HeaderValue::from_static("Bearer wrong"),
        );
        assert_eq!(
            authorize_request(&headers, &config),
            Err(StatusCode::UNAUTHORIZED)
        );
        headers.insert(
            axum::http::header::AUTHORIZATION,
            HeaderValue::from_static("Bearer test-secret"),
        );
        assert_eq!(authorize_request(&headers, &config), Ok(()));
    }

    #[test]
    fn rejects_unlisted_browser_origins() {
        let mut config = GatewayConfig::default();
        config.allowed_origins = vec!["https://agent.example".into()];
        let mut headers = HeaderMap::new();

        headers.insert(
            axum::http::header::ORIGIN,
            HeaderValue::from_static("https://attacker.example"),
        );
        assert_eq!(
            authorize_request(&headers, &config),
            Err(StatusCode::FORBIDDEN)
        );

        headers.insert(
            axum::http::header::ORIGIN,
            HeaderValue::from_static("https://agent.example"),
        );
        assert_eq!(authorize_request(&headers, &config), Ok(()));
    }

    #[test]
    fn compares_tokens_without_content_dependent_early_exit() {
        assert!(constant_time_eq(b"token", b"token"));
        assert!(!constant_time_eq(b"token", b"tokens"));
        assert!(!constant_time_eq(b"token", b"other"));
    }

    #[test]
    fn loopback_without_auth_accepts_native_requests() {
        let config = GatewayConfig::default();
        assert_eq!(authorize_request(&HeaderMap::new(), &config), Ok(()));
    }
}

#[cfg(test)]
mod end_to_end_tests {
    use super::*;
    use crate::config::{BackendConfig, Config};
    use axum::Router;
    use rmcp::{
        model::{
            CallToolRequestParams, CallToolResponse, CallToolResult, Implementation,
            ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerConfig,
            Tool as McpTool,
        },
        serve_client,
        service::RequestContext,
        transport::{
            streamable_http_client::StreamableHttpClientTransportConfig,
            streamable_http_server::session::local::LocalSessionManager,
            StreamableHttpClientTransport, StreamableHttpServerConfig, StreamableHttpService,
        },
        ErrorData, RoleServer, ServerHandler,
    };
    use serde_json::json;
    use std::{future::Future, sync::Arc};

    #[derive(Clone)]
    struct EchoBackend;

    impl ServerHandler for EchoBackend {
        fn get_info(&self) -> ServerConfig {
            ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
                .with_server_info(Implementation::new("echo-backend", "1.0.0"))
        }

        fn list_tools(
            &self,
            _request: Option<PaginatedRequestParams>,
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
                Ok(ListToolsResult::with_all_items(vec![McpTool::new(
                    "echo_value",
                    "Echo the provided value",
                    Arc::new(schema),
                )]))
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
                    .unwrap_or(serde_json::Value::Null);
                Ok(CallToolResult::structured(json!({"echo": value})).into())
            }
        }
    }

    #[tokio::test]
    async fn client_searches_and_invokes_a_remote_backend_through_gateway_http() {
        let backend_service = StreamableHttpService::new(
            || Ok(EchoBackend),
            Arc::new(LocalSessionManager::default()),
            StreamableHttpServerConfig::default(),
        );
        let backend_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let backend_address = backend_listener.local_addr().unwrap();
        let (backend_shutdown_tx, backend_shutdown_rx) = tokio::sync::oneshot::channel::<()>();
        let backend_server = tokio::spawn(async move {
            axum::serve(
                backend_listener,
                Router::new().route_service("/mcp", backend_service),
            )
            .with_graceful_shutdown(async {
                let _ = backend_shutdown_rx.await;
            })
            .await
            .unwrap();
        });

        let mut config: Config =
            toml::from_str(include_str!("../../examples/minimal.toml")).unwrap();
        config.backends.clear();
        config.backends.insert(
            "test".into(),
            BackendConfig::Http {
                url: format!("http://{backend_address}/mcp"),
                auth: None,
            },
        );
        let health = HealthManager::new();
        let backends = BackendManager::new(&config, health.clone()).await.unwrap();
        let router = SemanticRouter::new(health.clone());
        router.index_tools(backends.list_all_tools().await).await;
        let state = AppState {
            config: Arc::new(config.clone()),
            backend_manager: Arc::new(backends),
            router: Arc::new(router),
            health_manager: health,
            storage: None,
            traces: DecisionTraceStore::default(),
        };
        let gateway_app = build_router(config.gateway, state).unwrap();
        let gateway_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let gateway_address = gateway_listener.local_addr().unwrap();
        let (gateway_shutdown_tx, gateway_shutdown_rx) = tokio::sync::oneshot::channel::<()>();
        let gateway_server = tokio::spawn(async move {
            axum::serve(gateway_listener, gateway_app)
                .with_graceful_shutdown(async {
                    let _ = gateway_shutdown_rx.await;
                })
                .await
                .unwrap();
        });

        let transport = StreamableHttpClientTransport::from_config(
            StreamableHttpClientTransportConfig::with_uri(format!("http://{gateway_address}/mcp")),
        );
        let client = serve_client((), transport).await.unwrap();
        let tools = client.peer().list_all_tools().await.unwrap();
        assert_eq!(tools.len(), 6);

        let search = client
            .peer()
            .call_tool_once(
                CallToolRequestParams::new("gateway_search_tools").with_arguments(
                    json!({"query": "echo a value", "top_k": 3})
                        .as_object()
                        .unwrap()
                        .clone(),
                ),
            )
            .await
            .unwrap();
        let CallToolResponse::Complete(search) = search else {
            panic!("search unexpectedly returned an incomplete response");
        };
        let search_json = serde_json::to_value(search).unwrap();
        assert!(search_json.to_string().contains("test::echo_value"));

        let invocation = client
            .peer()
            .call_tool_once(
                CallToolRequestParams::new("gateway_invoke").with_arguments(
                    json!({
                        "tool_id": "test::echo_value",
                        "arguments": {"value": "round-trip"}
                    })
                    .as_object()
                    .unwrap()
                    .clone(),
                ),
            )
            .await
            .unwrap();
        let CallToolResponse::Complete(invocation) = invocation else {
            panic!("invocation unexpectedly returned an incomplete response");
        };
        assert!(serde_json::to_value(invocation)
            .unwrap()
            .to_string()
            .contains("round-trip"));

        let _ = gateway_shutdown_tx.send(());
        let _ = backend_shutdown_tx.send(());
        gateway_server.await.unwrap();
        backend_server.await.unwrap();
    }
}
