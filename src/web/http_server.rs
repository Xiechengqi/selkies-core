//! HTTP server for health checks and WebRTC signaling
//!
//! Provides health check endpoints, metrics, and WebRTC signaling WebSocket.

#![allow(dead_code)]

use crate::web::shared::SharedState;
use axum::{
    body::Body,
    extract::State,
    http::{header, StatusCode},
    response::Response,
    routing::get,
    Router,
};

#[cfg(feature = "webrtc-streaming")]
use axum::extract::WebSocketUpgrade;
use log::info;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::net::TcpListener;
use tower_http::services::{ServeDir, ServeFile};

#[cfg(feature = "webrtc-streaming")]
use crate::webrtc::SessionManager;

/// Run the HTTP health check server
pub async fn run_http_server(port: u16, state: Arc<SharedState>) -> Result<(), Box<dyn std::error::Error>> {
    run_http_server_with_webrtc(port, state, None).await
}

/// Run the HTTP server with optional WebRTC signaling support
#[cfg(feature = "webrtc-streaming")]
pub async fn run_http_server_with_webrtc(
    port: u16,
    state: Arc<SharedState>,
    session_manager: Option<Arc<SessionManager>>,
) -> Result<(), Box<dyn std::error::Error>> {
    let addr = format!("0.0.0.0:{}", port);
    let static_root = std::env::var("SELKIES_WEB_ROOT")
        .unwrap_or_else(|_| "web/selkies".to_string());
    let cwd = std::env::current_dir().ok();
    let index_path = PathBuf::from(&static_root).join("index.html");
    info!(
        "Serving web UI from {:?} (cwd: {:?})",
        static_root, cwd
    );
    if !index_path.exists() {
        info!("Web UI index not found at {:?}", index_path);
    }
    let static_service = ServeDir::new(&static_root).fallback(ServeFile::new(index_path));

    // Build router with WebRTC signaling endpoint if available
    let mut app = Router::new()
        .route("/", get(index_handler))
        .route("/index.html", get(index_handler))
        .route("/health", get(health_handler))
        .route("/metrics", get(metrics_handler))
        .route("/clients", get(clients_handler))
        .route("/ui-config", get(ui_config_handler))
        .route("/ws-config", get(ws_config_handler));

    // Add WebRTC signaling endpoint if session manager is provided
    if let Some(manager) = session_manager {
        info!("Adding WebRTC signaling endpoint at /webrtc");
        app = app.route("/webrtc", get({
            let state_clone = state.clone();
            move |ws: WebSocketUpgrade| {
                let state = state_clone.clone();
                let manager = manager.clone();
                async move {
                    ws.on_upgrade(move |socket| async move {
                        crate::transport::handle_signaling_connection(socket, state, manager).await;
                    })
                }
            }
        }));
    }

    let app: Router<()> = app
        .fallback_service(static_service)
        .with_state(state);

    let listener = TcpListener::bind(&addr).await?;
    info!("HTTP server listening on http://{}", addr);

    // axum 0.8: Router<()> can be used directly with serve
    axum::serve(listener, app)
        .await
        .map_err(|e| Box::new(e) as Box<dyn std::error::Error>)?;

    Ok(())
}

/// Fallback for non-WebRTC builds
#[cfg(not(feature = "webrtc-streaming"))]
pub async fn run_http_server_with_webrtc(
    port: u16,
    state: Arc<SharedState>,
    _session_manager: Option<Arc<()>>,
) -> Result<(), Box<dyn std::error::Error>> {
    // Just call the regular HTTP server without WebRTC support
    run_http_server_impl(port, state).await
}

/// Internal implementation of HTTP server
async fn run_http_server_impl(port: u16, state: Arc<SharedState>) -> Result<(), Box<dyn std::error::Error>> {
    let addr = format!("0.0.0.0:{}", port);
    let static_root = std::env::var("SELKIES_WEB_ROOT")
        .unwrap_or_else(|_| "web/selkies".to_string());
    let cwd = std::env::current_dir().ok();
    let index_path = PathBuf::from(&static_root).join("index.html");
    info!(
        "Serving web UI from {:?} (cwd: {:?})",
        static_root, cwd
    );
    if !index_path.exists() {
        info!("Web UI index not found at {:?}", index_path);
    }
    let static_service = ServeDir::new(&static_root).fallback(ServeFile::new(index_path));

    let app: Router<()> = Router::new()
        .route("/", get(index_handler))
        .route("/index.html", get(index_handler))
        .route("/health", get(health_handler))
        .route("/metrics", get(metrics_handler))
        .route("/clients", get(clients_handler))
        .route("/ui-config", get(ui_config_handler))
        .route("/ws-config", get(ws_config_handler))
        .fallback_service(static_service)
        .with_state(state);

    let listener = TcpListener::bind(&addr).await?;
    info!("HTTP server listening on http://{}", addr);

    // axum 0.8: Router<()> can be used directly with serve
    axum::serve(listener, app)
        .await
        .map_err(|e| Box::new(e) as Box<dyn std::error::Error>)?;

    Ok(())
}

/// Health check handler
async fn health_handler(State(state): State<Arc<SharedState>>) -> String {
    let uptime = state.uptime();
    let clients = state.connection_count();

    format!(
        r#"{{
  "status": "healthy",
  "uptime_seconds": {:.2},
  "connections": {},
  "version": "{}"
}}"#,
        uptime.as_secs_f64(),
        clients,
        env!("CARGO_PKG_VERSION")
    )
}

/// Metrics handler (Prometheus format)
async fn metrics_handler(State(state): State<Arc<SharedState>>) -> String {
    let uptime = state.uptime().as_secs_f64();
    let clients = state.connection_count();
    let stats = state.stats.lock().unwrap().clone();

    format!(
        r#"# HELP selkies_core_uptime_seconds Server uptime in seconds
# TYPE selkies_core_uptime_seconds counter
selkies_core_uptime_seconds {}
# HELP selkies_core_connections Current number of connections
# TYPE selkies_core_connections gauge
selkies_core_connections {}
# HELP selkies_core_cpu_percent Process CPU usage percent
# TYPE selkies_core_cpu_percent gauge
selkies_core_cpu_percent {}
# HELP selkies_core_mem_bytes Process RSS in bytes
# TYPE selkies_core_mem_bytes gauge
selkies_core_mem_bytes {}
"#,
        uptime, clients, stats.cpu_percent, stats.mem_used
    )
}

/// Clients handler
async fn clients_handler(State(state): State<Arc<SharedState>>) -> String {
    let clients = state.get_all_clients();
    let client_list: Vec<String> = clients
        .iter()
        .map(|c| {
            format!(
                r#"{{
    "id": "{}",
    "address": "{}",
    "connected_seconds": {:.2}
  }}"#,
                c.id,
                c.address,
                c.connected_at.elapsed().as_secs_f64()
            )
        })
        .collect();

    format!(
        r#"{{
  "count": {},
  "clients": [{}]
}}"#,
        clients.len(),
        client_list.join(",")
    )
}

/// UI configuration handler
async fn ui_config_handler(State(state): State<Arc<SharedState>>) -> String {
    state.ui_config_json()
}

/// WebSocket configuration handler
async fn ws_config_handler(State(state): State<Arc<SharedState>>) -> String {
    format!(r#"{{"ws_port":{}}}"#, state.config.websocket.port)
}

async fn index_handler(State(state): State<Arc<SharedState>>) -> Response {
    let static_root = std::env::var("SELKIES_WEB_ROOT")
        .unwrap_or_else(|_| "web/selkies".to_string());
    let index_path = PathBuf::from(&static_root).join("index.html");
    match tokio::fs::read(&index_path).await {
        Ok(data) => {
            let injected = match String::from_utf8(data) {
                Ok(mut html) => {
                    let port = state.config.websocket.port.to_string();
                    // Only replace the string literal placeholder, not the variable name
                    if html.contains("__SELKIES_INJECTED_PORT__") {
                        html = html.replace("__SELKIES_INJECTED_PORT__", &port);
                    }
                    Body::from(html)
                }
                Err(err) => Body::from(err.into_bytes()),
            };
            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
                .header(header::CACHE_CONTROL, "no-store, max-age=0")
                .body(injected)
                .unwrap()
        }
        Err(_) => Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::from("index.html not found"))
            .unwrap(),
    }
}
