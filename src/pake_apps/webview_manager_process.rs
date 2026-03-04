use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::process::{Child, Command};
use log::{info, warn};
use super::app::{PakeApp, AppStatus};
use super::datadir;

/// Process information for a WebView instance
struct WebViewProcess {
    app_name: String,
    child: Child,
    pid: u32,
}

/// Manager for WebView instances running as separate processes
pub struct WebViewManager {
    processes: Arc<Mutex<HashMap<String, WebViewProcess>>>,
    webview_binary: String,
    /// App IDs explicitly stopped by user (should not auto-restart)
    stopped_by_user: Arc<Mutex<HashSet<String>>>,
}

impl WebViewManager {
    pub fn new() -> Self {
        info!("Initializing WebViewManager (process-based)");

        let webview_binary = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.join("ivnc-webview")))
            .and_then(|p| {
                if p.exists() {
                    p.canonicalize().ok().and_then(|cp| cp.to_str().map(String::from))
                } else {
                    None
                }
            })
            .unwrap_or_else(|| "ivnc-webview".to_string());

        info!("WebView binary path: {}", webview_binary);
        if !std::path::Path::new(&webview_binary).exists() {
            warn!("WebView binary not found at: {}", webview_binary);
        }

        Self {
            processes: Arc::new(Mutex::new(HashMap::new())),
            webview_binary,
            stopped_by_user: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    pub fn set_store(&mut self, store: Arc<super::store::AppStore>) {
        self.start_watchdog(store);
    }

    fn start_watchdog(&self, store: Arc<super::store::AppStore>) {
        let processes = self.processes.clone();
        let stopped_by_user = self.stopped_by_user.clone();
        let webview_binary = self.webview_binary.clone();

        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;

                let crashed: Vec<(String, String)> = {
                    let mut procs = processes.lock().unwrap();
                    let user_stopped = stopped_by_user.lock().unwrap();
                    let mut crashed = Vec::new();
                    procs.retain(|app_id, proc| {
                        match nix::sys::signal::kill(
                            nix::unistd::Pid::from_raw(proc.pid as i32),
                            None,
                        ) {
                            Ok(_) => true,
                            Err(_) => {
                                if !user_stopped.contains(app_id) {
                                    crashed.push((app_id.clone(), proc.app_name.clone()));
                                }
                                false
                            }
                        }
                    });
                    crashed
                };

                for (app_id, _app_name) in crashed {
                    info!("Watchdog: webview app {} exited unexpectedly, restarting", app_id);
                    if let Ok(app) = store.get(&app_id) {
                        let mgr_ref = processes.clone();
                        let binary = webview_binary.clone();
                        match spawn_webview_process(&app, &binary) {
                            Ok((child, pid)) => {
                                info!("Watchdog: restarted webview '{}' (pid={})", app.name, pid);
                                mgr_ref.lock().unwrap().insert(app_id, WebViewProcess {
                                    app_name: app.name.clone(),
                                    child,
                                    pid,
                                });
                            }
                            Err(e) => warn!("Watchdog: failed to restart webview {}: {}", app_id, e),
                        }
                    }
                }
            }
        });
    }

    /// Start a WebView for the given app as a separate process
    pub fn start(&mut self, app: &PakeApp) -> Result<(), String> {
        info!("Starting WebView process for app: {} ({})", app.name, app.id);

        {
            let processes = self.processes.lock().unwrap();
            if processes.contains_key(&app.id) {
                return Err(format!("WebView already running for app: {}", app.id));
            }
        }

        // Remove from user-stopped so watchdog will restart if it crashes
        self.stopped_by_user.lock().unwrap().remove(&app.id);

        let (child, pid) = spawn_webview_process(app, &self.webview_binary)?;

        self.processes.lock().unwrap().insert(app.id.clone(), WebViewProcess {
            app_name: app.name.clone(),
            child,
            pid,
        });

        Ok(())
    }

    /// Stop a WebView process
    pub fn stop(&self, app_id: &str) -> Result<(), String> {
        info!("Stopping WebView process for app: {}", app_id);

        // Mark as user-stopped so watchdog won't restart it
        self.stopped_by_user.lock().unwrap().insert(app_id.to_string());

        let mut processes = self.processes.lock().unwrap();
        if let Some(mut process) = processes.remove(app_id) {
            let _ = process.child.kill();
            let _ = process.child.wait();
            info!("WebView process stopped: {}", app_id);
            Ok(())
        } else {
            Err(format!("WebView not found: {}", app_id))
        }
    }

    /// Restart a WebView
    pub fn restart(&mut self, app: &PakeApp) -> Result<(), String> {
        info!("Restarting WebView for app: {}", app.id);
        let _ = self.stop(&app.id);
        // Remove from stopped_by_user so watchdog keeps it alive after restart
        self.stopped_by_user.lock().unwrap().remove(&app.id);
        self.start(app)
    }

    /// Get status of a WebView
    pub fn status(&self, app_id: &str) -> AppStatus {
        let processes = self.processes.lock().unwrap();
        if let Some(process) = processes.get(app_id) {
            // Check if process is still alive
            match nix::sys::signal::kill(
                nix::unistd::Pid::from_raw(process.pid as i32),
                None
            ) {
                Ok(_) => AppStatus::Running,
                Err(_) => AppStatus::Crashed,
            }
        } else {
            AppStatus::Stopped
        }
    }

    /// Get PID of a WebView process
    #[allow(dead_code)]
    pub fn pid(&self, app_id: &str) -> Option<u32> {
        let processes = self.processes.lock().unwrap();
        processes.get(app_id).map(|p| p.pid)
    }

    /// Stop all WebViews
    pub fn stop_all(&self) {
        info!("Stopping all WebView processes");
        let app_ids: Vec<String> = self.processes.lock().unwrap().keys().cloned().collect();
        for app_id in app_ids {
            let _ = self.stop(&app_id);
        }
    }
}

impl Drop for WebViewManager {
    fn drop(&mut self) {
        info!("Dropping WebViewManager");
        self.stop_all();
    }
}

/// Get log file path for a webview app
fn get_log_path(app_id: &str) -> std::path::PathBuf {
    let dir = dirs::config_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("/root/.config"))
        .join("ivnc")
        .join("pake-apps")
        .join(app_id);
    dir.join("app.log")
}

/// Spawn a webview process for the given app, returns (Child, pid)
fn spawn_webview_process(app: &PakeApp, webview_binary: &str) -> Result<(std::process::Child, u32), String> {
    // Check if webview binary exists
    if !std::path::Path::new(webview_binary).exists() {
        return Err(format!("WebView binary not found: {}", webview_binary));
    }

    let data_dir = datadir::data_dir(app);
    std::fs::create_dir_all(&data_dir)
        .map_err(|e| format!("Failed to create data dir: {}", e))?;

    let log_file_path = get_log_path(&app.id);
    let log_dir = log_file_path.parent().ok_or("Invalid log path")?;
    std::fs::create_dir_all(log_dir)
        .map_err(|e| format!("Failed to create log dir: {}", e))?;

    let wayland_display = std::env::var("WAYLAND_DISPLAY")
        .unwrap_or_else(|_| "wayland-1".to_string());

    let webview_path = std::path::Path::new(webview_binary);
    let webview_dir = webview_path.parent().ok_or("Invalid webview binary path")?;

    let safe_app_name = app.name
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect::<String>();

    let app_symlink = webview_dir.join(&safe_app_name);
    let _ = std::fs::remove_file(&app_symlink);

    // Use absolute path for symlink target
    let abs_webview_binary = webview_path.canonicalize()
        .unwrap_or_else(|_| webview_path.to_path_buf());

    std::os::unix::fs::symlink(&abs_webview_binary, &app_symlink)
        .map_err(|e| format!("Failed to create symlink {} -> {}: {}", app_symlink.display(), abs_webview_binary.display(), e))?;

    info!("Spawning WebView: {} -> {} (WAYLAND_DISPLAY={})", app_symlink.display(), abs_webview_binary.display(), wayland_display);

    let child = Command::new(&app_symlink)
        .env("IVNC_APP_ID", &app.id)
        .env("IVNC_APP_NAME", &app.name)
        .env("IVNC_APP_URL", &app.url)
        .env("IVNC_DARK_MODE", if app.dark_mode { "true" } else { "false" })
        .env("IVNC_LOG_FILE", log_file_path.to_str().unwrap())
        .env("IVNC_DATA_DIR", data_dir.to_str().unwrap())
        .env("WAYLAND_DISPLAY", wayland_display)
        .env("GDK_BACKEND", "wayland")
        .env("RUST_LOG", std::env::var("RUST_LOG").unwrap_or_else(|_| "info".to_string()))
        .spawn()
        .map_err(|e| format!("Failed to spawn WebView process {}: {} (binary: {}, symlink exists: {})",
            app_symlink.display(), e, abs_webview_binary.display(), app_symlink.exists()))?;

    let pid = child.id();
    info!("WebView process started with PID: {}", pid);
    Ok((child, pid))
}
