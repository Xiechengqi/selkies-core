use axum::{
    extract::{Path, State},
    http::{header, StatusCode},
    response::Response,
    body::Body,
    routing::{get, post},
    Router,
};
use serde_json::json;
use std::sync::{Arc, Mutex};
use super::app::{PakeApp, AppMode, AppStatus};
use super::store::AppStore;
use super::process::ProcessManager;
use super::WebViewManager;
use super::datadir;
use super::autostart;
use super::native;
use super::state_recovery::AppRunningState;

pub struct PakeState {
    pub store: Arc<AppStore>,
    pub process: ProcessManager,
    pub webview: Arc<Mutex<WebViewManager>>,
}

impl PakeState {
    pub fn new() -> Result<Self, String> {
        let store = Arc::new(AppStore::new()?);
        let mut process = ProcessManager::new();
        process.set_store(store.clone());
        let mut webview_mgr = WebViewManager::new();
        webview_mgr.set_store(store.clone());
        Ok(Self {
            store,
            process,
            webview: Arc::new(Mutex::new(webview_mgr)),
        })
    }

    /// Save current running apps state
    pub fn save_running_state(&self) -> Result<(), String> {
        let apps = self.store.list()?;
        let running_ids: Vec<String> = apps.iter()
            .filter(|app| {
                let status = match app.mode {
                    AppMode::Native => self.process.status(&app.id),
                    AppMode::Webview => self.webview.lock().unwrap().status(&app.id),
                };
                matches!(status, AppStatus::Running)
            })
            .map(|app| app.id.clone())
            .collect();

        if running_ids.is_empty() {
            log::info!("No running apps to save");
            return Ok(());
        }

        let state = AppRunningState::new(running_ids);
        state.save()
    }

    /// Restore previously running apps
    pub async fn restore_running_state(&self) -> Result<(), String> {
        let state = match AppRunningState::load() {
            Ok(s) => s,
            Err(_) => {
                log::info!("No previous running state to restore");
                return Ok(());
            }
        };

        // Only restore if state is recent (within 5 minutes)
        if !state.is_recent() {
            log::info!("Running state is too old, skipping restore");
            AppRunningState::clear()?;
            return Ok(());
        }

        log::info!("Restoring {} running apps", state.app_ids.len());

        for app_id in &state.app_ids {
            match self.store.get(app_id) {
                Ok(app) => {
                    log::info!("Restoring app: {} ({})", app.name, app_id);
                    let result = match app.mode {
                        AppMode::Native => self.process.start(&app).map(|_| ()),
                        AppMode::Webview => {
                            self.webview.lock().unwrap().start(&app)
                        }
                    };

                    if let Err(e) = result {
                        log::warn!("Failed to restore app {}: {}", app_id, e);
                    } else {
                        // Small delay between starts to avoid overwhelming the system
                        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                    }
                }
                Err(e) => {
                    log::warn!("App {} not found in store: {}", app_id, e);
                }
            }
        }

        // Clear the state file after restoration
        AppRunningState::clear()?;
        log::info!("Running state restoration completed");
        Ok(())
    }
}

pub fn router(state: Arc<PakeState>) -> Router {
    Router::new()
        .route("/api/apps", get(list_apps).post(add_app))
        .route("/api/apps/{id}", get(get_app).put(update_app).delete(delete_app))
        .route("/api/apps/{id}/start", post(start_app))
        .route("/api/apps/{id}/stop", post(stop_app))
        .route("/api/apps/{id}/restart", post(restart_app))
        .route("/api/apps/{id}/data-size", get(data_size))
        .route("/api/apps/{id}/clear-data", post(clear_data))
        .route("/api/apps/{id}/logs", get(get_logs))
        .with_state(state)
}

fn json_response(status: StatusCode, body: serde_json::Value) -> Response {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

fn err_response(status: StatusCode, msg: &str) -> Response {
    json_response(status, json!({"error": msg}))
}

fn app_json(app: &PakeApp, status: &str, pid: Option<u32>, data_bytes: u64) -> serde_json::Value {
    json!({
        "id": app.id,
        "name": app.name,
        "url": app.url,
        "mode": app.mode,
        "dark_mode": app.dark_mode,
        "autostart": app.autostart,
        "show_nav": app.show_nav,
        "status": status,
        "pid": pid,
        "data_size_bytes": data_bytes,
        "data_size_human": datadir::size_human(data_bytes),
        "created_at": app.created_at,
    })
}

async fn list_apps(State(state): State<Arc<PakeState>>) -> Response {
    match state.store.list() {
        Ok(apps) => {
            let items: Vec<_> = apps.iter().map(|app| {
                let (st, pid) = match app.mode {
                    AppMode::Native => {
                        let status = state.process.status(&app.id);
                        let pid = state.process.pid(&app.id);
                        (status, pid)
                    }
                    AppMode::Webview => {
                        let status = state.webview.lock().unwrap().status(&app.id);
                        (status, None)
                    }
                };
                let size = datadir::dir_size(&datadir::data_dir(app));
                app_json(app, &format!("{:?}", st).to_lowercase(), pid, size)
            }).collect();
            json_response(StatusCode::OK, json!({"apps": items}))
        }
        Err(e) => err_response(StatusCode::INTERNAL_SERVER_ERROR, &e),
    }
}

async fn add_app(
    State(state): State<Arc<PakeState>>,
    axum::extract::Json(body): axum::extract::Json<serde_json::Value>,
) -> Response {
    let name = match body.get("name").and_then(|v| v.as_str()) {
        Some(n) if !n.trim().is_empty() => n.trim().to_string(),
        _ => return err_response(StatusCode::BAD_REQUEST, "missing name"),
    };
    let url = match body.get("url").and_then(|v| v.as_str()) {
        Some(u) if !u.trim().is_empty() => u.trim().to_string(),
        _ => return err_response(StatusCode::BAD_REQUEST, "missing url"),
    };
    let mode = match body.get("mode").and_then(|v| v.as_str()) {
        Some(m) => AppMode::from_str(m).unwrap_or(AppMode::Native),
        None => AppMode::Native,
    };

    let app = PakeApp {
        id: uuid::Uuid::new_v4().to_string(),
        name,
        url,
        mode,
        dark_mode: body.get("dark_mode").and_then(|v| v.as_bool()).unwrap_or(false),
        autostart: body.get("autostart").and_then(|v| v.as_bool()).unwrap_or(false),
        show_nav: body.get("show_nav").and_then(|v| v.as_bool()).unwrap_or(false),
        created_at: chrono_now(),
    };

    if let Err(e) = state.store.add(&app) {
        return err_response(StatusCode::CONFLICT, &e);
    }
    if app.autostart {
        let _ = autostart::set(&app);
    }
    json_response(StatusCode::CREATED, json!({"ok": true, "app": app}))
}

async fn get_app(State(state): State<Arc<PakeState>>, Path(id): Path<String>) -> Response {
    match state.store.get(&id) {
        Ok(app) => {
            let (st, pid) = match app.mode {
                AppMode::Native => {
                    let status = state.process.status(&app.id);
                    let pid = state.process.pid(&app.id);
                    (status, pid)
                }
                AppMode::Webview => {
                    let status = state.webview.lock().unwrap().status(&app.id);
                    (status, None)
                }
            };
            let size = datadir::dir_size(&datadir::data_dir(&app));
            json_response(StatusCode::OK, app_json(&app, &format!("{:?}", st).to_lowercase(), pid, size))
        }
        Err(e) => err_response(StatusCode::NOT_FOUND, &e),
    }
}

async fn update_app(
    State(state): State<Arc<PakeState>>,
    Path(id): Path<String>,
    axum::extract::Json(body): axum::extract::Json<serde_json::Value>,
) -> Response {
    let existing = match state.store.get(&id) {
        Ok(a) => a,
        Err(e) => return err_response(StatusCode::NOT_FOUND, &e),
    };

    let url = body.get("url").and_then(|v| v.as_str()).unwrap_or(&existing.url).to_string();
    let mode = body.get("mode").and_then(|v| v.as_str())
        .and_then(AppMode::from_str).unwrap_or(existing.mode);
    let dark_mode = body.get("dark_mode").and_then(|v| v.as_bool()).unwrap_or(existing.dark_mode);
    let auto = body.get("autostart").and_then(|v| v.as_bool()).unwrap_or(existing.autostart);
    let show_nav = body.get("show_nav").and_then(|v| v.as_bool()).unwrap_or(existing.show_nav);

    if let Err(e) = state.store.update(&id, &url, mode, dark_mode, auto, show_nav) {
        return err_response(StatusCode::INTERNAL_SERVER_ERROR, &e);
    }
    if auto {
        let updated = PakeApp {
            id: existing.id, name: existing.name, url, mode, dark_mode, autostart: auto, show_nav,
            created_at: existing.created_at,
        };
        let _ = autostart::set(&updated);
    } else {
        let _ = autostart::remove(&id);
    }
    json_response(StatusCode::OK, json!({"ok": true}))
}

async fn delete_app(State(state): State<Arc<PakeState>>, Path(id): Path<String>) -> Response {
    let _ = state.process.stop(&id);
    let _ = state.webview.lock().unwrap().stop(&id);
    let _ = autostart::remove(&id);
    // Clean up data directory before removing from store
    if let Ok(app) = state.store.get(&id) {
        let data_dir = datadir::data_dir(&app).parent().map(|p| p.to_path_buf())
            .unwrap_or_else(|| datadir::data_dir(&app));
        let _ = std::fs::remove_dir_all(&data_dir);
    }
    match state.store.delete(&id) {
        Ok(()) => json_response(StatusCode::OK, json!({"ok": true})),
        Err(e) => err_response(StatusCode::NOT_FOUND, &e),
    }
}

async fn start_app(State(state): State<Arc<PakeState>>, Path(id): Path<String>) -> Response {
    let app = match state.store.get(&id) {
        Ok(a) => a,
        Err(e) => return err_response(StatusCode::NOT_FOUND, &e),
    };

    match app.mode {
        AppMode::Native => {
            match state.process.start(&app) {
                Ok(pid) => json_response(StatusCode::OK, json!({"ok": true, "pid": pid})),
                Err(e) => err_response(StatusCode::INTERNAL_SERVER_ERROR, &e),
            }
        }
        AppMode::Webview => {
            match state.webview.lock().unwrap().start(&app) {
                Ok(()) => json_response(StatusCode::OK, json!({"ok": true, "mode": "webview"})),
                Err(e) => err_response(StatusCode::INTERNAL_SERVER_ERROR, &e),
            }
        }
    }
}

async fn stop_app(State(state): State<Arc<PakeState>>, Path(id): Path<String>) -> Response {
    let app = match state.store.get(&id) {
        Ok(a) => a,
        Err(e) => return err_response(StatusCode::NOT_FOUND, &e),
    };

    let result = match app.mode {
        AppMode::Native => state.process.stop(&id),
        AppMode::Webview => state.webview.lock().unwrap().stop(&id),
    };

    match result {
        Ok(()) => json_response(StatusCode::OK, json!({"ok": true})),
        Err(e) => err_response(StatusCode::BAD_REQUEST, &e),
    }
}

async fn restart_app(State(state): State<Arc<PakeState>>, Path(id): Path<String>) -> Response {
    let app = match state.store.get(&id) {
        Ok(a) => a,
        Err(e) => return err_response(StatusCode::NOT_FOUND, &e),
    };

    match app.mode {
        AppMode::Native => {
            match state.process.restart(&app) {
                Ok(pid) => json_response(StatusCode::OK, json!({"ok": true, "pid": pid})),
                Err(e) => err_response(StatusCode::INTERNAL_SERVER_ERROR, &e),
            }
        }
        AppMode::Webview => {
            match state.webview.lock().unwrap().restart(&app) {
                Ok(()) => json_response(StatusCode::OK, json!({"ok": true, "mode": "webview"})),
                Err(e) => err_response(StatusCode::INTERNAL_SERVER_ERROR, &e),
            }
        }
    }
}

async fn data_size(State(state): State<Arc<PakeState>>, Path(id): Path<String>) -> Response {
    let app = match state.store.get(&id) {
        Ok(a) => a,
        Err(e) => return err_response(StatusCode::NOT_FOUND, &e),
    };
    let dir = datadir::data_dir(&app);
    let bytes = datadir::dir_size(&dir);
    json_response(StatusCode::OK, json!({
        "data_dir": dir.display().to_string(),
        "size_bytes": bytes,
        "size_human": datadir::size_human(bytes),
    }))
}

async fn clear_data(State(state): State<Arc<PakeState>>, Path(id): Path<String>) -> Response {
    let app = match state.store.get(&id) {
        Ok(a) => a,
        Err(e) => return err_response(StatusCode::NOT_FOUND, &e),
    };
    // Stop app before clearing data
    match app.mode {
        AppMode::Native => { let _ = state.process.stop(&id); }
        AppMode::Webview => { let _ = state.webview.lock().unwrap().stop(&id); }
    }
    match datadir::clear(&app) {
        Ok(()) => json_response(StatusCode::OK, json!({"ok": true})),
        Err(e) => err_response(StatusCode::INTERNAL_SERVER_ERROR, &e),
    }
}

async fn get_logs(State(state): State<Arc<PakeState>>, Path(id): Path<String>) -> Response {
    // Verify app exists
    if let Err(e) = state.store.get(&id) {
        return err_response(StatusCode::NOT_FOUND, &e);
    }
    let log_file = native::log_path(&id);
    let content = std::fs::read_to_string(&log_file).unwrap_or_else(|_| "(no logs yet)".into());
    // Return last 200 lines
    let lines: Vec<&str> = content.lines().collect();
    let start = if lines.len() > 200 { lines.len() - 200 } else { 0 };
    let tail = lines[start..].join("\n");
    json_response(StatusCode::OK, json!({"logs": tail, "path": log_file.display().to_string()}))
}

fn chrono_now() -> String {
    // UTC timestamp without chrono dependency
    let d = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = d.as_secs();
    let days = secs / 86400;
    let time_secs = secs % 86400;
    let h = time_secs / 3600;
    let m = (time_secs % 3600) / 60;
    let s = time_secs % 60;
    // Days since 1970-01-01
    let (y, mo, day) = days_to_ymd(days);
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", y, mo, day, h, m, s)
}

fn days_to_ymd(mut days: u64) -> (u64, u64, u64) {
    let mut y = 1970;
    loop {
        let dy = if y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) { 366 } else { 365 };
        if days < dy { break; }
        days -= dy;
        y += 1;
    }
    let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let mdays = [31, if leap {29} else {28}, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut mo = 0;
    for (i, &md) in mdays.iter().enumerate() {
        if days < md { mo = i + 1; break; }
        days -= md;
    }
    (y, mo as u64, days + 1)
}
