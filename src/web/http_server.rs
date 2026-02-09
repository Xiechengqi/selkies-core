//! HTTP server for health checks and WebRTC signaling
//!
//! Provides health check endpoints, metrics, and WebRTC signaling WebSocket.

#![allow(dead_code)]

use crate::web::embedded_assets::{get_embedded_file, has_embedded_assets};
use crate::web::shared::SharedState;
use axum::{
    body::Body,
    extract::State,
    extract::WebSocketUpgrade,
    http::{header, Request, StatusCode, Uri},
    middleware,
    response::Response,
    routing::get,
    Router,
};

use log::info;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::net::TcpListener;
use tower_http::services::{ServeDir, ServeFile};
use base64::Engine;
use hmac::{Hmac, Mac};
use serde_json::json;
use sha1::Sha1;

use crate::webrtc::SessionManager;

/// Run the HTTP server with WebRTC signaling support
pub async fn run_http_server_with_webrtc(
    port: u16,
    state: Arc<SharedState>,
    session_manager: Option<Arc<SessionManager>>,
) -> Result<(), Box<dyn std::error::Error>> {
    let addr = format!("0.0.0.0:{}", port);

    // Check for embedded assets first, then fall back to filesystem
    let use_embedded = has_embedded_assets() && std::env::var("SELKIES_WEB_ROOT").is_err();

    if use_embedded {
        info!("Serving web UI from embedded assets");
    } else {
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
    }

    // Build router
    let mut app = Router::new()
        .route("/", get(index_handler))
        .route("/index.html", get(index_handler))
        .route("/health", get(health_handler))
        .route("/metrics", get(metrics_handler))
        .route("/clients", get(clients_handler))
        .route("/ui-config", get(ui_config_handler))
        .route("/ws-config", get(ws_config_handler))
        .route("/turn", get(turn_config_handler));

    // Add WebRTC signaling endpoint if session manager is provided
    if let Some(manager) = session_manager {
        info!("Adding WebRTC signaling endpoint at /webrtc");
        let state_clone = state.clone();
        let manager_clone = manager.clone();
        let signaling_handler = move |ws: WebSocketUpgrade| {
            let state = state_clone.clone();
            let manager = manager_clone.clone();
            async move {
                ws.on_upgrade(move |socket| async move {
                    crate::transport::handle_signaling_connection(socket, state, manager).await;
                })
            }
        };
        app = app
            .route("/webrtc", get(signaling_handler.clone()))
            .route("/webrtc/signaling", get(signaling_handler.clone()))
            .route("/webrtc/signaling/", get(signaling_handler.clone()))
            .route("/{app}/signaling", get(signaling_handler.clone()))
            .route("/{app}/signaling/", get(signaling_handler));
    }

    // Set up fallback for static files
    let auth_state = state.clone();
    let app: Router<()> = if use_embedded {
        app.fallback(embedded_fallback_handler)
            .with_state(state)
    } else {
        let static_root = std::env::var("SELKIES_WEB_ROOT")
            .unwrap_or_else(|_| "web/selkies".to_string());
        let index_path = PathBuf::from(&static_root).join("index.html");
        let static_service = ServeDir::new(&static_root).fallback(ServeFile::new(index_path));
        app.fallback_service(static_service)
            .with_state(state)
    };

    let app = app.layer(middleware::from_fn_with_state(auth_state, basic_auth_middleware));

    let listener = TcpListener::bind(&addr).await?;
    info!("HTTP server listening on http://{}", addr);

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
# HELP selkies_core_client_latency_ms Client-reported latency in ms
# TYPE selkies_core_client_latency_ms gauge
selkies_core_client_latency_ms {}
# HELP selkies_core_client_fps Client-reported FPS
# TYPE selkies_core_client_fps gauge
selkies_core_client_fps {}
# HELP selkies_core_ice_candidates_total Total ICE candidates observed
# TYPE selkies_core_ice_candidates_total counter
selkies_core_ice_candidates_total {}
# HELP selkies_core_ice_candidates_udp Total ICE candidates over UDP
# TYPE selkies_core_ice_candidates_udp counter
selkies_core_ice_candidates_udp {}
# HELP selkies_core_ice_candidates_tcp Total ICE candidates over TCP
# TYPE selkies_core_ice_candidates_tcp counter
selkies_core_ice_candidates_tcp {}
# HELP selkies_core_ice_candidates_host Total ICE candidates of type host
# TYPE selkies_core_ice_candidates_host counter
selkies_core_ice_candidates_host {}
# HELP selkies_core_ice_candidates_srflx Total ICE candidates of type srflx
# TYPE selkies_core_ice_candidates_srflx counter
selkies_core_ice_candidates_srflx {}
# HELP selkies_core_ice_candidates_relay Total ICE candidates of type relay
# TYPE selkies_core_ice_candidates_relay counter
selkies_core_ice_candidates_relay {}
# HELP selkies_core_ice_candidates_prflx Total ICE candidates of type prflx
# TYPE selkies_core_ice_candidates_prflx counter
selkies_core_ice_candidates_prflx {}
"#,
        uptime,
        clients,
        stats.cpu_percent,
        stats.mem_used,
        stats.client_latency_ms,
        stats.client_fps,
        stats.ice_candidates_total,
        stats.ice_candidates_udp,
        stats.ice_candidates_tcp,
        stats.ice_candidates_host,
        stats.ice_candidates_srflx,
        stats.ice_candidates_relay,
        stats.ice_candidates_prflx
    )
}

async fn basic_auth_middleware(
    State(state): State<Arc<SharedState>>,
    req: Request<Body>,
    next: middleware::Next,
) -> Response {
    if !state.config.http.basic_auth_enabled {
        return next.run(req).await;
    }

    if req.uri().path() == "/health" {
        return next.run(req).await;
    }

    match req.headers().get(header::AUTHORIZATION) {
        Some(value) => {
            if let Ok(value_str) = value.to_str() {
                if let Some(encoded) = value_str.strip_prefix("Basic ") {
                    if let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(encoded) {
                        if let Ok(decoded_str) = String::from_utf8(decoded) {
                            if let Some((user, pass)) = decoded_str.split_once(':') {
                                if user == state.config.http.basic_auth_user
                                    && pass == state.config.http.basic_auth_password
                                {
                                    return next.run(req).await;
                                }
                            }
                        }
                    }
                }
            }
        }
        None => {}
    }

    let mut response = Response::builder()
        .status(StatusCode::UNAUTHORIZED)
        .body(Body::from("Unauthorized"))
        .unwrap_or_else(|_| Response::new(Body::empty()));
    response.headers_mut().insert(
        header::WWW_AUTHENTICATE,
        header::HeaderValue::from_static("Basic realm=\"selkies-core\""),
    );
    response
}

/// Clients handler - returns WebRTC session count
async fn clients_handler(State(state): State<Arc<SharedState>>) -> String {
    let webrtc_sessions = state.webrtc_sessions();

    format!(
        r#"{{
  "webrtc_sessions": {}
}}"#,
        webrtc_sessions
    )
}

/// UI configuration handler
async fn ui_config_handler(State(state): State<Arc<SharedState>>) -> String {
    state.ui_config_json()
}

/// WebSocket configuration handler
async fn ws_config_handler(State(state): State<Arc<SharedState>>) -> Response {
    let payload = json!({
        "ws_port": state.config.http.port
    });
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(payload.to_string()))
        .unwrap()
}

/// TURN/STUN configuration handler for WebRTC
async fn turn_config_handler(State(state): State<Arc<SharedState>>) -> Response {
    let ice_servers = build_ice_servers(&state.config.webrtc);
    let payload = json!({
        "iceServers": ice_servers
    });
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(payload.to_string()))
        .unwrap()
}

async fn index_handler(State(_state): State<Arc<SharedState>>) -> Response {
    // Check for embedded assets first, then fall back to filesystem
    let use_embedded = has_embedded_assets() && std::env::var("SELKIES_WEB_ROOT").is_err();

    if use_embedded {
        return get_embedded_file("index.html");
    }

    // Fallback to filesystem
    let static_root = std::env::var("SELKIES_WEB_ROOT")
        .unwrap_or_else(|_| "web/selkies".to_string());
    let index_path = PathBuf::from(&static_root).join("index.html");
    match tokio::fs::read(&index_path).await {
        Ok(data) => {
            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
                .header(header::CACHE_CONTROL, "no-store, max-age=0")
                .body(Body::from(data))
                .unwrap()
        }
        Err(_) => Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::from("index.html not found"))
            .unwrap(),
    }
}

/// Handler for serving embedded static files
async fn embedded_fallback_handler(uri: Uri) -> Response {
    get_embedded_file(uri.path())
}

fn build_ice_servers(config: &crate::config::WebRTCConfig) -> Vec<crate::config::IceServerConfig> {
    let mut servers = Vec::new();

    let has_turn = !config.turn_host.is_empty()
        || !config.turn_shared_secret.is_empty()
        || !config.turn_username.is_empty()
        || !config.turn_password.is_empty();
    let has_stun = !config.stun_host.is_empty() && config.stun_port != 0;

    if has_stun {
        servers.push(crate::config::IceServerConfig {
            urls: vec![format!("stun:{}:{}", config.stun_host, config.stun_port)],
            username: None,
            credential: None,
        });
    }

    if has_turn && !config.turn_host.is_empty() {
        let scheme = if config.turn_tls { "turns" } else { "turn" };
        let transport = if config.turn_protocol.is_empty() {
            "udp"
        } else {
            config.turn_protocol.as_str()
        };
        let url = format!(
            "{}:{}:{}?transport={}",
            scheme,
            config.turn_host,
            config.turn_port,
            transport
        );

        let (username, credential) = if !config.turn_shared_secret.is_empty() {
            let ttl_secs: u64 = 24 * 60 * 60;
            let expiry = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() + ttl_secs)
                .unwrap_or(ttl_secs);
            let user = format!("{}:selkies", expiry);
            let password = hmac_sha1_base64(&config.turn_shared_secret, &user);
            (Some(user), Some(password))
        } else if !config.turn_username.is_empty() && !config.turn_password.is_empty() {
            (Some(config.turn_username.clone()), Some(config.turn_password.clone()))
        } else {
            (None, None)
        };

        servers.push(crate::config::IceServerConfig {
            urls: vec![url],
            username,
            credential,
        });
    }

    if servers.is_empty() {
        return config.ice_servers.clone();
    }

    servers
}

fn hmac_sha1_base64(secret: &str, message: &str) -> String {
    let mut mac = Hmac::<Sha1>::new_from_slice(secret.as_bytes())
        .unwrap_or_else(|_| Hmac::<Sha1>::new_from_slice(&[]).unwrap());
    mac.update(message.as_bytes());
    let result = mac.finalize().into_bytes();
    base64::engine::general_purpose::STANDARD.encode(result)
}
