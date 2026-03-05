//! HTTP server with same-port ICE-TCP protocol splitting
//!
//! Provides health check endpoints, metrics, WebRTC signaling WebSocket,
//! and same-port HTTP + ICE-TCP multiplexing. Incoming TCP connections are
//! classified by peeking the first byte:
//! - ASCII letters (HTTP methods) → axum HTTP handler
//! - 0x00-0x03 (ICE) or 0x14-0x17 (DTLS) → WebRTC ICE-TCP session

#![allow(dead_code)]

use crate::web::embedded_assets::{get_embedded_file, has_embedded_assets};
use crate::web::shared::SharedState;
use axum::{
    body::Body,
    extract::{Query, State, WebSocketUpgrade},
    http::{header, Request, StatusCode, Uri},
    middleware,
    response::Response,
    routing::{get, post},
    Router,
};

use hyper_util::rt::TokioIo;
use log::{info, warn, debug};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::AsyncReadExt;
#[cfg(feature = "tls")]
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tower::Service;
use tower_http::services::{ServeDir, ServeFile};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::webrtc::SessionManager;
use crate::pake_apps::api::PakeState;

/// Classify a TCP connection by its first bytes.
fn classify_first_bytes(buf: &[u8]) -> ConnectionType {
    if buf.is_empty() {
        return ConnectionType::Unknown;
    }
    if looks_like_http(buf) {
        return ConnectionType::Http;
    }
    let b0 = buf[0];
    // DTLS/TLS handshake record type (0x16) needs version disambiguation.
    if b0 == 0x16 {
        if buf.len() >= 2 {
            let b1 = buf[1];
            if b1 == 0x03 {
                return ConnectionType::Tls; // TLS record
            }
            if b1 == 0xFE {
                return ConnectionType::IceTcp; // DTLS record
            }
        }
        // Ambiguous: default to ICE/DTLS to avoid misrouting DTLS to HTTPS.
        return ConnectionType::IceTcp;
    }
    // ICE/DTLS record types (ChangeCipherSpec, Alert, Handshake, ApplicationData)
    if (0x00..=0x03).contains(&b0) || (0x14..=0x17).contains(&b0) {
        return ConnectionType::IceTcp;
    }
    ConnectionType::Unknown
}

fn looks_like_http(buf: &[u8]) -> bool {
    const METHODS: [&[u8]; 9] = [
        b"GET", b"POST", b"HEAD", b"PUT", b"PATCH", b"DELETE", b"OPTIONS", b"CONNECT", b"TRACE",
    ];
    if METHODS.iter().any(|m| buf.starts_with(m)) {
        return true;
    }
    // If we only have a few bytes, treat an initial ASCII letter as a partial HTTP method.
    buf.len() < 3 && buf.first().is_some_and(|b| b.is_ascii_alphabetic())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConnectionType {
    Http,
    IceTcp,
    Tls,
    Unknown,
}

/// Run the HTTP server with WebRTC signaling support and same-port ICE-TCP
pub async fn run_http_server_with_webrtc(
    port: u16,
    state: Arc<SharedState>,
    session_manager: Option<Arc<SessionManager>>,
    enable_tls: bool,
    pake_state: Option<Arc<PakeState>>,
) -> Result<(), Box<dyn std::error::Error>> {
    let addr = format!("0.0.0.0:{}", port);

    // Check for embedded assets first, then fall back to filesystem
    let use_embedded = has_embedded_assets() && std::env::var("IVNC_WEB_ROOT").is_err();

    if use_embedded {
        info!("Serving web UI from embedded assets");
    } else {
        let static_root = std::env::var("IVNC_WEB_ROOT")
            .unwrap_or_else(|_| "web/ivnc".to_string());
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
        .route("/api/change-password", post(change_password_handler))
        .route("/api/version", get(get_version_handler))
        .route("/api/upgrade/ws", get(upgrade_ws_handler))
        ;

    // Add WebRTC signaling endpoint if session manager is provided
    if let Some(ref manager) = session_manager {
        info!("Adding WebRTC signaling endpoint at /webrtc");
        let state_clone = state.clone();
        let manager_clone = manager.clone();
        let signaling_handler = move |
            headers: axum::http::HeaderMap,
            ws: WebSocketUpgrade,
        | {
            let state = state_clone.clone();
            let manager = manager_clone.clone();
            let host_str = headers.get(axum::http::header::HOST)
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string());
            async move {
                ws.on_upgrade(move |socket| async move {
                    crate::transport::handle_signaling_connection(socket, state, manager, host_str).await;
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

    // MCP Streamable HTTP endpoint
    #[cfg(feature = "mcp")]
    {
        let mcp_state = state.clone();
        let mcp_session_mgr = Arc::new(
            rmcp::transport::streamable_http_server::session::local::LocalSessionManager::default(),
        );
        let mcp_config = rmcp::transport::streamable_http_server::StreamableHttpServerConfig {
            stateful_mode: true,
            ..Default::default()
        };
        let mcp_service = rmcp::transport::streamable_http_server::StreamableHttpService::new(
            move || Ok(crate::mcp::McpServer::new(mcp_state.clone())),
            mcp_session_mgr,
            mcp_config,
        );
        app = app.route_service("/mcp", mcp_service);
        info!("MCP Streamable HTTP endpoint enabled at /mcp");
    }

    // Pake apps management routes
    if let Some(_pake) = &pake_state {
        app = app.route("/console", get(console_handler));
        info!("Pake apps console enabled at /console");
    }

    // Set up fallback for static files
    let auth_state = state.clone();
    let metrics_state = state.clone(); // keep a copy for the accept loop (metrics)
    let mut app: Router<()> = if use_embedded {
        app.fallback(embedded_fallback_handler)
            .with_state(state)
    } else {
        let static_root = std::env::var("IVNC_WEB_ROOT")
            .unwrap_or_else(|_| "web/ivnc".to_string());
        let index_path = PathBuf::from(&static_root).join("index.html");
        let static_service = ServeDir::new(&static_root).fallback(ServeFile::new(index_path));
        app.fallback_service(static_service)
            .with_state(state)
    };

    // Merge pake routes after with_state (both are Router<()> now)
    if let Some(ref pake) = pake_state {
        app = app.merge(crate::pake_apps::api::router(pake.clone()));
    }

    let app = app.layer(middleware::from_fn_with_state(auth_state, basic_auth_middleware));

    let listener = TcpListener::bind(&addr).await?;
    let local_addr = listener.local_addr()?;

    // TLS setup
    #[cfg(feature = "tls")]
    let tls_acceptor = if enable_tls {
        let acceptor = create_tls_acceptor()?;
        info!("HTTPS+ICE-TCP server listening on https://{}", local_addr);
        Some(acceptor)
    } else {
        info!("HTTP+ICE-TCP server listening on http://{}", local_addr);
        None
    };
    #[cfg(not(feature = "tls"))]
    {
        let _ = enable_tls;
        info!("HTTP+ICE-TCP server listening on http://{}", local_addr);
    }

    if session_manager.is_some() {
        info!("Same-port ICE-TCP multiplexing enabled on :{}", port);
    }

    // Accept loop with first-byte protocol splitting
    loop {
        let (tcp_stream, peer_addr) = match listener.accept().await {
            Ok(conn) => conn,
            Err(e) => {
                warn!("TCP accept error: {}", e);
                continue;
            }
        };

        let app = app.clone();
        let sm = session_manager.clone();
        let conn_state = metrics_state.clone();
        #[cfg(feature = "tls")]
        let tls_acceptor = tls_acceptor.clone();

        tokio::spawn(async move {
            let mut first_bytes = vec![0u8; 8];
            let peek_result = tokio::time::timeout(
                std::time::Duration::from_secs(10),
                tcp_stream.peek(&mut first_bytes),
            ).await;
            let n = match peek_result {
                Ok(Ok(0)) | Err(_) => return,
                Ok(Ok(n)) => n,
                Ok(Err(e)) => {
                    debug!("Peek error from {}: {}", peer_addr, e);
                    return;
                }
            };
            first_bytes.truncate(n);
            // If we only got the first byte and it's a TLS/DTLS handshake marker,
            // try a quick re-peek to disambiguate version (0x03 vs 0xFE).
            if first_bytes.len() < 2 && first_bytes.first() == Some(&0x16) {
                let mut retry_buf = vec![0u8; 8];
                if let Ok(Ok(n2)) = tokio::time::timeout(
                    std::time::Duration::from_millis(50),
                    tcp_stream.peek(&mut retry_buf),
                ).await {
                    if n2 > 0 {
                        retry_buf.truncate(n2);
                        first_bytes = retry_buf;
                    }
                }
            }
            let kind = classify_first_bytes(&first_bytes);
            debug!("Connection from {} classified as {:?} (first_bytes={:02x?})", peer_addr, kind, &first_bytes);

            // Record protocol classification metric
            conn_state.record_protocol_classification(match kind {
                ConnectionType::Http => "http",
                ConnectionType::IceTcp => "ice_tcp",
                ConnectionType::Tls => "tls",
                ConnectionType::Unknown => "unknown",
            });

            // In TLS mode: disambiguate TLS vs DTLS by version bytes
            #[cfg(feature = "tls")]
            if let Some(ref acceptor) = tls_acceptor {
                if kind == ConnectionType::Tls {
                    // TLS handshake
                    match acceptor.accept(tcp_stream).await {
                        Ok(tls_stream) => {
                            serve_http(TokioIo::new(tls_stream), app).await;
                        }
                        Err(e) => {
                            debug!("TLS handshake error from {}: {}", peer_addr, e);
                        }
                    }
                    return;
                }
                // Non-TLS: route HTTP if detected, otherwise ICE-TCP
                match kind {
                    ConnectionType::Http => {
                        debug!("Rejecting plaintext HTTP on TLS port from {}", peer_addr);
                        let body = "HTTPS required";
                        let response = format!(
                            "HTTP/1.1 426 Upgrade Required\r\nConnection: close\r\nContent-Length: {}\r\nContent-Type: text/plain\r\n\r\n{}",
                            body.len(),
                            body
                        );
                        let _ = tcp_stream.write_all(response.as_bytes()).await;
                        let _ = tcp_stream.shutdown().await;
                    }
                    ConnectionType::IceTcp | ConnectionType::Unknown | ConnectionType::Tls => {
                        handle_ice_connection(tcp_stream, peer_addr, sm).await;
                    }
                }
                return;
            }

            // Non-TLS mode: first-byte classification
            match kind {
                ConnectionType::IceTcp => handle_ice_connection(tcp_stream, peer_addr, sm).await,
                ConnectionType::Http | ConnectionType::Tls => {
                    serve_http(TokioIo::new(tcp_stream), app).await;
                }
                ConnectionType::Unknown => {
                    warn!("Unrecognized protocol from {} (first_bytes={:02x?}), closing", peer_addr, &first_bytes);
                }
            }
        });
    }
}

/// Serve HTTP over a generic IO stream
async fn serve_http<I>(io: TokioIo<I>, app: Router<()>)
where
    I: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let service = hyper::service::service_fn(move |req| {
        let mut app = app.clone();
        async move { app.call(req).await }
    });
    let _ = hyper_util::server::conn::auto::Builder::new(
        hyper_util::rt::TokioExecutor::new(),
    )
    .serve_connection_with_upgrades(io, service)
    .await;
}

/// Handle an ICE-TCP connection
async fn handle_ice_connection(
    mut stream: tokio::net::TcpStream,
    peer_addr: std::net::SocketAddr,
    sm: Option<Arc<SessionManager>>,
) {
    if let Some(sm) = sm {
        let ice_local_addr = sm.listen_addr();
        let mut buf = vec![0u8; 8192];
        match stream.read(&mut buf).await {
            Ok(0) => return,
            Ok(n) => {
                buf.truncate(n);
                info!("ICE-TCP connection from {} ({} bytes, first16={:02x?})",
                    peer_addr, n, &buf[..n.min(16)]);
                if let Err(e) = sm.handle_ice_tcp_connection(
                    stream, peer_addr, ice_local_addr, &buf,
                ).await {
                    warn!("ICE-TCP session match failed from {}: {}", peer_addr, e);
                }
            }
            Err(e) => {
                debug!("ICE-TCP read error from {}: {}", peer_addr, e);
            }
        }
    } else {
        debug!("ICE-TCP connection from {} but no session manager", peer_addr);
    }
}

#[cfg(feature = "tls")]
fn create_tls_acceptor() -> Result<tokio_rustls::TlsAcceptor, Box<dyn std::error::Error>> {
    use rustls::ServerConfig;
    use std::sync::Arc as StdArc;

    let cert = rcgen::generate_simple_self_signed(vec![
        "localhost".to_string(),
        "ivnc.local".to_string(),
    ])?;
    let cert_der = rustls::pki_types::CertificateDer::from(cert.cert);
    let key_der = rustls::pki_types::PrivateKeyDer::try_from(cert.key_pair.serialize_der())
        .map_err(|e| format!("TLS key error: {}", e))?;

    let config = ServerConfig::builder_with_provider(StdArc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()?
        .with_no_client_auth()
        .with_single_cert(vec![cert_der], key_der)?;

    info!("TLS enabled with self-signed certificate");
    Ok(tokio_rustls::TlsAcceptor::from(StdArc::new(config)))
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
        r#"# HELP ivnc_uptime_seconds Server uptime in seconds
# TYPE ivnc_uptime_seconds counter
ivnc_uptime_seconds {}
# HELP ivnc_connections Current number of connections
# TYPE ivnc_connections gauge
ivnc_connections {}
# HELP ivnc_cpu_percent Process CPU usage percent
# TYPE ivnc_cpu_percent gauge
ivnc_cpu_percent {}
# HELP ivnc_mem_bytes Process RSS in bytes
# TYPE ivnc_mem_bytes gauge
ivnc_mem_bytes {}
# HELP ivnc_client_latency_ms Client-reported latency in ms
# TYPE ivnc_client_latency_ms gauge
ivnc_client_latency_ms {}
# HELP ivnc_client_fps Client-reported FPS
# TYPE ivnc_client_fps gauge
ivnc_client_fps {}
# HELP ivnc_proto_connections_total Protocol classification counters
# TYPE ivnc_proto_connections_total counter
ivnc_proto_connections_total{{protocol="http"}} {}
ivnc_proto_connections_total{{protocol="ice_tcp"}} {}
ivnc_proto_connections_total{{protocol="tls"}} {}
ivnc_proto_connections_total{{protocol="unknown"}} {}
"#,
        uptime,
        clients,
        stats.cpu_percent,
        stats.mem_used,
        stats.client_latency_ms,
        stats.client_fps,
        stats.proto_http,
        stats.proto_ice_tcp,
        stats.proto_tls,
        stats.proto_unknown
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

    let path = req.uri().path();
    if path == "/health"
        || path == "/manifest.json"
        || path == "/sw.js"
        || path.starts_with("/icons/")
    {
        return next.run(req).await;
    }

    // Read password override; clone to release the RwLock guard immediately
    let expected_password = {
        let guard = state.password_override.read().await;
        match guard.as_deref() {
            Some(overridden) => overridden.to_string(),
            None => state.config.http.basic_auth_password.clone(),
        }
    };

    match req.headers().get(header::AUTHORIZATION) {
        Some(value) => {
            if let Ok(value_str) = value.to_str() {
                if let Some(encoded) = value_str.strip_prefix("Basic ") {
                    if let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(encoded) {
                        if let Ok(decoded_str) = String::from_utf8(decoded) {
                            if let Some((user, pass)) = decoded_str.split_once(':') {
                                if user == state.config.http.basic_auth_user
                                    && pass == expected_password
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
        header::HeaderValue::from_static("Basic realm=\"ivnc\""),
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
        "ws_port": state.config.http.port,
        "tcp_only": state.config.webrtc.tcp_only
    });
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(payload.to_string()))
    .unwrap()
}

async fn index_handler(State(_state): State<Arc<SharedState>>) -> Response {
    // Check for embedded assets first, then fall back to filesystem
    let use_embedded = has_embedded_assets() && std::env::var("IVNC_WEB_ROOT").is_err();

    if use_embedded {
        return get_embedded_file("index.html");
    }

    // Fallback to filesystem
    let static_root = std::env::var("IVNC_WEB_ROOT")
        .unwrap_or_else(|_| "web/ivnc".to_string());
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

/// Change password handler
async fn change_password_handler(
    State(state): State<Arc<SharedState>>,
    axum::extract::Json(body): axum::extract::Json<serde_json::Value>,
) -> Response {
    let new_password = match body.get("new_password").and_then(|v| v.as_str()) {
        Some(p) => p,
        None => {
            return Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"error":"missing new_password field"}"#))
                .unwrap();
        }
    };

    if new_password.len() < 4 {
        return Response::builder()
            .status(StatusCode::BAD_REQUEST)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(r#"{"error":"password must be at least 4 characters"}"#))
            .unwrap();
    }

    let mut pw = state.password_override.write().await;
    *pw = Some(new_password.to_string());
    info!("Password changed via /api/change-password");

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"ok":true}"#))
        .unwrap()
}

/// Console page handler - serves the Pake apps management UI
async fn console_handler() -> Response {
    let html = include_str!("../../web/console/index.html");
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
        .header(header::CACHE_CONTROL, "no-store, max-age=0")
        .body(Body::from(html))
        .unwrap()
}

// ============================================================================
// Force Update Feature
// ============================================================================

/// Version information
#[derive(Serialize, Clone)]
struct VersionInfo {
    current: String,
    latest: String,
    has_update: bool,
    download_url: String,
}

/// Upgrade log entry for WebSocket streaming
#[derive(Clone, Serialize)]
struct UpgradeLogEntry {
    step: u8,
    total_steps: u8,
    message: String,
    level: String,  // "info" | "error" | "success" | "progress"
    #[serde(skip_serializing_if = "Option::is_none")]
    progress: Option<u8>,  // 0-100
}

/// GitHub Release response
#[derive(Deserialize)]
struct GitHubRelease {
    tag_name: String,
}

/// WebSocket auth query
#[derive(Deserialize)]
struct WsAuthQuery {
    token: Option<String>,
}

/// GET /api/version - Check for updates
async fn get_version_handler() -> axum::Json<VersionInfo> {
    let current = env!("CARGO_PKG_VERSION").to_string();

    // Detect architecture
    let arch = if cfg!(target_arch = "x86_64") {
        "amd64"
    } else if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        return axum::Json(VersionInfo {
            current: current.clone(),
            latest: current,
            has_update: false,
            download_url: String::new(),
        });
    };

    // Fetch latest version from GitHub
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .user_agent("ivnc-updater")
        .build() {
        Ok(c) => c,
        Err(_) => {
            return axum::Json(VersionInfo {
                current: current.clone(),
                latest: current,
                has_update: false,
                download_url: String::new(),
            });
        }
    };

    let latest_version = match client
        .get("https://api.github.com/repos/Xiechengqi/iVnc/releases/latest")
        .send()
        .await {
        Ok(resp) => match resp.json::<GitHubRelease>().await {
            Ok(release) => release.tag_name.trim_start_matches('v').to_string(),
            Err(_) => current.clone(),
        },
        Err(_) => current.clone(),
    };

    let has_update = latest_version != current;
    let download_url = format!(
        "https://github.com/Xiechengqi/iVnc/releases/download/latest/ivnc-linux-{}",
        arch
    );

    axum::Json(VersionInfo {
        current,
        latest: latest_version,
        has_update,
        download_url,
    })
}

/// GET /api/upgrade/ws - WebSocket upgrade endpoint
async fn upgrade_ws_handler(
    State(state): State<Arc<SharedState>>,
    Query(_query): Query<WsAuthQuery>,
    headers: axum::http::HeaderMap,
    ws: WebSocketUpgrade,
) -> Result<Response, StatusCode> {
    // Verify authentication if basic auth is enabled
    if state.config.http.basic_auth_enabled {
        // Check Authorization header
        if let Some(auth_header) = headers.get(header::AUTHORIZATION) {
            if let Ok(auth_str) = auth_header.to_str() {
                if let Some(encoded) = auth_str.strip_prefix("Basic ") {
                    if let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(encoded) {
                        if let Ok(decoded_str) = String::from_utf8(decoded) {
                            if let Some((user, pass)) = decoded_str.split_once(':') {
                                // Read password override
                                let expected_password = {
                                    let guard = state.password_override.read().await;
                                    match guard.as_deref() {
                                        Some(overridden) => overridden.to_string(),
                                        None => state.config.http.basic_auth_password.clone(),
                                    }
                                };

                                if user == state.config.http.basic_auth_user && pass == expected_password {
                                    return Ok(ws.on_upgrade(handle_upgrade_websocket));
                                }
                            }
                        }
                    }
                }
            }
        }

        // Authentication failed
        return Err(StatusCode::UNAUTHORIZED);
    }

    // No auth required, proceed
    Ok(ws.on_upgrade(handle_upgrade_websocket))
}

/// Handle WebSocket connection for upgrade
async fn handle_upgrade_websocket(mut socket: axum::extract::ws::WebSocket) {
    use tokio::sync::mpsc;
    use futures::SinkExt;

    let (log_tx, mut log_rx) = mpsc::channel::<UpgradeLogEntry>(32);

    // Spawn upgrade task
    let mut upgrade_task = tokio::spawn(async move {
        perform_upgrade_with_logs(log_tx).await
    });

    // Forward logs to WebSocket
    loop {
        tokio::select! {
            Some(entry) = log_rx.recv() => {
                let json = serde_json::to_string(&entry).unwrap_or_default();
                if socket.send(axum::extract::ws::Message::Text(json.into())).await.is_err() {
                    break;
                }
            }
            _ = &mut upgrade_task => {
                break;
            }
        }
    }

    // Drain remaining logs
    while let Ok(entry) = log_rx.try_recv() {
        let json = serde_json::to_string(&entry).unwrap_or_default();
        let _ = socket.send(axum::extract::ws::Message::Text(json.into())).await;
    }

    let _ = socket.close().await;
}

/// Perform upgrade with real-time logging
async fn perform_upgrade_with_logs(log_tx: tokio::sync::mpsc::Sender<UpgradeLogEntry>) {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use futures::StreamExt;

    let send_log = |step: u8, message: &str, level: &str, progress: Option<u8>| {
        let entry = UpgradeLogEntry {
            step,
            total_steps: 10,
            message: message.to_string(),
            level: level.to_string(),
            progress,
        };
        let tx = log_tx.clone();
        async move {
            let _ = tx.send(entry).await;
        }
    };

    // Step 1: Detect architecture
    send_log(1, "检测系统架构...", "info", None).await;
    let arch = if cfg!(target_arch = "x86_64") {
        "amd64"
    } else if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        send_log(1, "不支持的系统架构", "error", None).await;
        return;
    };
    send_log(1, &format!("系统架构: {}", arch), "success", None).await;

    // Step 2: Build download URL
    send_log(2, "准备下载最新版本...", "info", None).await;
    let download_url = format!(
        "https://github.com/Xiechengqi/iVnc/releases/download/latest/ivnc-linux-{}",
        arch
    );
    send_log(2, &format!("下载地址: {}", download_url), "success", None).await;

    // Step 3: Download new version
    send_log(3, "开始下载...", "info", None).await;
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(300))
        .user_agent("ivnc-updater")
        .build() {
        Ok(c) => c,
        Err(e) => {
            send_log(3, &format!("创建 HTTP 客户端失败: {}", e), "error", None).await;
            return;
        }
    };

    let mut response = match client.get(&download_url).send().await {
        Ok(r) => {
            if !r.status().is_success() {
                send_log(3, &format!("下载失败: HTTP {}", r.status()), "error", None).await;
                return;
            }
            r
        },
        Err(e) => {
            send_log(3, &format!("下载失败: {}", e), "error", None).await;
            return;
        }
    };

    let total_size = response.content_length().unwrap_or(0);
    let temp_path = format!("/tmp/ivnc-new-{}", std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs());

    let mut file = match tokio::fs::File::create(&temp_path).await {
        Ok(f) => f,
        Err(e) => {
            send_log(3, &format!("创建临时文件失败: {}", e), "error", None).await;
            return;
        }
    };

    let mut downloaded = 0u64;
    let mut last_progress = 0u8;

    while let Some(chunk_result) = response.chunk().await.transpose() {
        let chunk = match chunk_result {
            Ok(c) => c,
            Err(e) => {
                send_log(3, &format!("下载数据失败: {}", e), "error", None).await;
                let _ = tokio::fs::remove_file(&temp_path).await;
                return;
            }
        };

        if tokio::io::AsyncWriteExt::write_all(&mut file, &chunk).await.is_err() {
            send_log(3, "写入文件失败", "error", None).await;
            let _ = tokio::fs::remove_file(&temp_path).await;
            return;
        }

        downloaded += chunk.len() as u64;
        if total_size > 0 {
            let progress = ((downloaded as f64 / total_size as f64) * 100.0) as u8;
            if progress != last_progress && (progress % 5 == 0 || progress == 100) {
                send_log(3, &format!("下载中... {}/{} MB",
                    downloaded / 1024 / 1024,
                    total_size / 1024 / 1024),
                    "progress", Some(progress)).await;
                last_progress = progress;
            }
        } else {
            // No content-length, show downloaded bytes only
            let current_mb = downloaded / 1024 / 1024;
            if current_mb > 0 && current_mb % 5 == 0 && current_mb as u8 != last_progress {
                send_log(3, &format!("下载中... {} MB", current_mb), "info", None).await;
                last_progress = current_mb as u8;
            }
        }
    }
    drop(file);
    send_log(3, "下载完成", "success", None).await;

    // Step 4: Verify download
    send_log(4, "验证下载文件...", "info", None).await;
    let metadata = match tokio::fs::metadata(&temp_path).await {
        Ok(m) => m,
        Err(e) => {
            send_log(4, &format!("读取文件信息失败: {}", e), "error", None).await;
            let _ = tokio::fs::remove_file(&temp_path).await;
            return;
        }
    };

    if metadata.len() < 1024 * 1024 {
        send_log(4, "下载文件过小，可能损坏", "error", None).await;
        let _ = tokio::fs::remove_file(&temp_path).await;
        return;
    }
    send_log(4, &format!("文件大小: {} MB", metadata.len() / 1024 / 1024), "success", None).await;

    // Step 5: Backup current version
    send_log(5, "备份当前版本...", "info", None).await;
    let current_exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            send_log(5, &format!("获取当前程序路径失败: {}", e), "error", None).await;
            let _ = tokio::fs::remove_file(&temp_path).await;
            return;
        }
    };

    let backup_path = format!("{}.backup-{}",
        current_exe.display(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    );

    if let Err(e) = tokio::fs::copy(&current_exe, &backup_path).await {
        send_log(5, &format!("备份失败: {}", e), "error", None).await;
        let _ = tokio::fs::remove_file(&temp_path).await;
        return;
    }
    send_log(5, "备份完成", "success", None).await;

    // Cleanup old backups (keep only the 3 most recent)
    cleanup_old_backups(&current_exe).await;

    // Step 6: Set permissions
    send_log(6, "设置执行权限...", "info", None).await;
    if let Err(e) = fs::set_permissions(&temp_path, fs::Permissions::from_mode(0o755)) {
        send_log(6, &format!("设置权限失败: {}", e), "error", None).await;
        let _ = tokio::fs::remove_file(&temp_path).await;
        return;
    }
    send_log(6, "权限设置完成", "success", None).await;

    // Step 7: Replace binary
    send_log(7, "替换程序文件...", "info", None).await;
    if let Err(e) = tokio::fs::remove_file(&current_exe).await {
        send_log(7, &format!("删除旧文件失败: {}", e), "error", None).await;
        return;
    }

    if let Err(e) = tokio::fs::rename(&temp_path, &current_exe).await {
        send_log(7, &format!("移动新文件失败: {}", e), "error", None).await;
        // Try to restore backup
        let _ = tokio::fs::copy(&backup_path, &current_exe).await;
        return;
    }
    send_log(7, "文件替换完成", "success", None).await;

    // Step 7.5: Verify new binary
    send_log(7, "验证新版本...", "info", None).await;
    match tokio::process::Command::new(&current_exe)
        .arg("--version")
        .output()
        .await {
        Ok(output) if output.status.success() => {
            send_log(7, "新版本验证通过", "success", None).await;
        }
        _ => {
            send_log(7, "新版本验证失败，恢复备份", "error", None).await;
            // Restore backup
            let _ = tokio::fs::copy(&backup_path, &current_exe).await;
            return;
        }
    }

    // Step 8: Cleanup
    send_log(8, "清理临时文件...", "info", None).await;
    let _ = tokio::fs::remove_file(&temp_path).await;
    send_log(8, "清理完成", "success", None).await;

    // Step 9: Prepare restart
    send_log(9, "准备重启服务...", "info", None).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    send_log(9, "即将重启", "success", None).await;

    // Step 10: Restart
    send_log(10, "重启服务...", "info", None).await;
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Try systemd restart first
    if try_restart_systemd("ivnc").await.is_ok() {
        return;
    }

    // Use exec to restart
    use std::os::unix::process::CommandExt;
    let args: Vec<String> = std::env::args().collect();
    let err = std::process::Command::new(&current_exe)
        .args(&args[1..])
        .exec();

    // If we reach here, exec failed
    send_log(10, &format!("重启失败: {}", err), "error", None).await;

    // Try to restore backup
    let _ = fs::remove_file(&current_exe);
    let _ = fs::copy(&backup_path, &current_exe);
    let _ = fs::set_permissions(&current_exe, fs::Permissions::from_mode(0o755));
}

/// Try to restart via systemd
async fn try_restart_systemd(service_name: &str) -> Result<(), Box<dyn std::error::Error>> {
    let output = tokio::process::Command::new("systemctl")
        .args(&["restart", service_name])
        .output()
        .await?;

    if output.status.success() {
        Ok(())
    } else {
        Err("systemctl restart failed".into())
    }
}

/// Cleanup old backup files, keeping only the most recent N backups
async fn cleanup_old_backups(exe_path: &std::path::Path) {
    use std::path::PathBuf;

    let parent_dir = match exe_path.parent() {
        Some(dir) => dir,
        None => return,
    };

    let exe_name = match exe_path.file_name() {
        Some(name) => name.to_string_lossy(),
        None => return,
    };

    // Find all backup files
    let mut backups: Vec<(PathBuf, u64)> = Vec::new();

    if let Ok(mut entries) = tokio::fs::read_dir(parent_dir).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            if let Ok(file_name) = entry.file_name().into_string() {
                // Match pattern: {exe_name}.backup-{timestamp}
                if file_name.starts_with(&format!("{}.backup-", exe_name)) {
                    if let Some(timestamp_str) = file_name.strip_prefix(&format!("{}.backup-", exe_name)) {
                        if let Ok(timestamp) = timestamp_str.parse::<u64>() {
                            backups.push((entry.path(), timestamp));
                        }
                    }
                }
            }
        }
    }

    // Keep only the 3 most recent backups
    const KEEP_COUNT: usize = 3;
    if backups.len() > KEEP_COUNT {
        // Sort by timestamp (newest first)
        backups.sort_by(|a, b| b.1.cmp(&a.1));

        // Delete old backups
        for (path, _) in backups.iter().skip(KEEP_COUNT) {
            let _ = tokio::fs::remove_file(path).await;
            info!("Cleaned up old backup: {:?}", path);
        }
    }
}
