use wry::{WebViewBuilder, WebViewBuilderExtUnix, WebViewExtUnix};
use tao::window::WindowBuilder;
use tao::platform::unix::WindowExtUnix;
use gtk::prelude::*;
use std::sync::{Arc, Mutex};
use std::path::PathBuf;
use std::fs::{File, OpenOptions};
use std::io::Write;
use log::info;
use super::app::PakeApp;
use super::datadir;

/// A WebView instance for a Pake app
/// Note: Must be created and used on the same thread (GTK/tao thread)
#[allow(dead_code)]
pub struct WebViewInstance {
    pub app_name: String,
    pub window_id: tao::window::WindowId,
    pub is_open: Arc<Mutex<bool>>,
    pub log_file: Arc<Mutex<File>>,
    // Keep window and webview alive
    _window: tao::window::Window,
    _webview: wry::WebView,
}

#[allow(dead_code)]
impl WebViewInstance {
    /// Create a new WebView instance for the given app
    /// This must be called on the tao/GTK event loop thread
    pub fn new(app: &PakeApp, event_loop: &tao::event_loop::EventLoopWindowTarget<()>) -> Result<Self, String> {
        info!("Creating WebView for app '{}' ({})", app.name, app.id);

        // Force tao window to connect to iVnc's Wayland compositor
        // This makes the window appear in iVnc's taskbar automatically
        let ivnc_wayland_display = std::env::var("WAYLAND_DISPLAY")
            .unwrap_or_else(|_| "wayland-1".to_string());
        info!("Connecting WebView to Wayland display: {}", ivnc_wayland_display);

        std::env::set_var("WAYLAND_DISPLAY", &ivnc_wayland_display);
        std::env::set_var("GDK_BACKEND", "wayland"); // Force Wayland backend
        std::env::set_var("QT_QPA_PLATFORM", "wayland"); // For Qt apps

        // Get data directory and set environment variable for WebKitGTK
        let data_dir = datadir::data_dir(app);
        info!("WebView data dir: {}", data_dir.display());

        // Ensure data directory exists
        std::fs::create_dir_all(&data_dir)
            .map_err(|e| format!("Failed to create data dir: {}", e))?;

        // Set WebKitGTK data directory via environment variable
        // This affects the WebView created in this thread
        std::env::set_var("WEBKIT_DISK_CACHE_DIR", data_dir.join("cache"));
        std::env::set_var("WEBKIT_LOCALSTORAGE_DIR", data_dir.join("localstorage"));
        std::env::set_var("WEBKIT_INDEXEDDB_DIR", data_dir.join("indexeddb"));

        // Create tao window
        let window = WindowBuilder::new()
            .with_title(&app.name)
            .with_inner_size(tao::dpi::LogicalSize::new(1280, 720))
            .build(event_loop)
            .map_err(|e| format!("Failed to create window: {}", e))?;

        info!("Tao window created: {}", app.name);

        // Set Wayland app_id for proper taskbar display
        let gtk_window = window.gtk_window();
        gtk_window.set_title(&app.name);

        // Set the WM_CLASS for X11 and app_id for Wayland
        if let Some(gdk_window) = gtk_window.window() {
            #[cfg(target_os = "linux")]
            {
                // Set window role
                gtk_window.set_role(&app.name);

                // For Wayland, try to set app_id via GDK Wayland backend
                // The app_id is typically derived from the program name or WM_CLASS
                // We'll try to influence it by setting various properties

                // Set the icon name which can influence app_id
                gtk_window.set_icon_name(Some(&app.name));

                // Set startup notification ID
                gdk_window.set_startup_id(&app.name);

                info!("Set window identifiers for app_id: {}", app.name);
            }
        }

        info!("Set GTK window title and identifiers to: {}", app.name);

        // Get GTK container from tao window (Linux only)
        let container = {
            let vbox = window.default_vbox()
                .ok_or("Failed to get GTK vbox from tao window")?;

            // Create container for WebView
            let container = gtk::Box::new(gtk::Orientation::Vertical, 0);
            vbox.pack_start(&container, true, true, 0);
            container.show_all();
            container
        };

        info!("GTK container created");

        // Create log file for console output
        let log_file_path = get_log_path(&app.id);
        let log_file = Arc::new(Mutex::new(
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(&log_file_path)
                .map_err(|e| format!("Failed to create log file: {}", e))?
        ));
        info!("WebView log file: {}", log_file_path.display());

        // Build initialization script with console logging
        let init_script = get_init_script_with_logging(app, log_file.clone());

        // Setup IPC handler for console logging
        let log_file_for_ipc = log_file.clone();
        let ipc_handler = move |req: wry::http::Request<String>| {
            let msg = req.body();
            info!("IPC message received: {}", msg);
            if msg.starts_with("console:") {
                let log_msg = msg.strip_prefix("console:").unwrap_or(msg);
                info!("Writing console log: {}", log_msg);
                if let Ok(mut file) = log_file_for_ipc.lock() {
                    let _ = writeln!(file, "{}", log_msg);
                    let _ = file.flush();
                }
            }
        };

        // Build WebView using GTK-specific API
        info!("Building WebView with URL: {}", app.url);
        let webview = WebViewBuilder::new()
            .with_url(&app.url)
            .with_initialization_script(&init_script)
            .with_ipc_handler(ipc_handler)
            .build_gtk(&container)
            .map_err(|e| format!("WebView build failed: {}", e))?;

        info!("WebView built successfully");

        // Get the WebView's GTK widget and ensure it expands to fill the container
        let webview_widget = webview.webview();
        container.pack_start(&webview_widget, true, true, 0);
        webview_widget.show();

        info!("WebView widget added to container");

        let is_open = Arc::new(Mutex::new(true));
        let window_id = window.id();

        // Note: Window close handling will be done in the event loop

        Ok(Self {
            app_name: app.name.clone(),
            window_id,
            is_open,
            log_file,
            _window: window,
            _webview: webview,
        })
    }

    /// Check if the window is still open
    pub fn is_open(&self) -> bool {
        *self.is_open.lock().unwrap()
    }

    /// Mark as closed
    pub fn mark_closed(&self) {
        *self.is_open.lock().unwrap() = false;
    }
}

/// Get log file path for a webview app
#[allow(dead_code)]
fn get_log_path(app_id: &str) -> PathBuf {
    let dir = dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("/root/.config"))
        .join("ivnc")
        .join("pake-apps")
        .join(app_id);
    let _ = std::fs::create_dir_all(&dir);
    dir.join("app.log")
}

/// Generate initialization script for WebView with console logging via IPC
#[allow(dead_code)]
fn get_init_script_with_logging(app: &PakeApp, _log_file: Arc<Mutex<File>>) -> String {
    let dark_mode_script = if app.dark_mode {
        r#"
        const style = document.createElement('style');
        style.textContent = ':root { color-scheme: dark; }';
        if (document.head) {
            document.head.appendChild(style);
        } else {
            document.addEventListener('DOMContentLoaded', () => {
                document.head.appendChild(style);
            });
        }
        "#
    } else {
        ""
    };

    format!(
        r#"
        {}
        (function() {{
            // Test if IPC is available
            if (window.ipc) {{
                window.ipc.postMessage('console:[INIT] iVnc WebView console logging initialized');
            }}

            const originalLog = console.log;
            const originalError = console.error;
            const originalWarn = console.warn;
            const originalInfo = console.info;

            function formatArgs(args) {{
                return Array.from(args).map(arg => {{
                    if (typeof arg === 'object') {{
                        try {{ return JSON.stringify(arg); }}
                        catch(e) {{ return String(arg); }}
                    }}
                    return String(arg);
                }}).join(' ');
            }}

            function sendLog(level, args) {{
                const timestamp = new Date().toISOString();
                const message = formatArgs(args);
                const logEntry = `[${{timestamp}}] [${{level}}] ${{message}}`;
                if (window.ipc) {{
                    window.ipc.postMessage('console:' + logEntry);
                }}
            }}

            console.log = function(...args) {{
                sendLog('LOG', args);
                originalLog.apply(console, args);
            }};

            console.error = function(...args) {{
                sendLog('ERROR', args);
                originalError.apply(console, args);
            }};

            console.warn = function(...args) {{
                sendLog('WARN', args);
                originalWarn.apply(console, args);
            }};

            console.info = function(...args) {{
                sendLog('INFO', args);
                originalInfo.apply(console, args);
            }};

            // Test console logging
            console.log('iVnc WebView console logging active');
        }})();
        "#,
        dark_mode_script
    )
}

/// Generate initialization script for WebView
#[allow(dead_code)]
fn get_init_script(app: &PakeApp) -> String {
    if app.dark_mode {
        r#"
        // Dark mode support
        (function() {
            const style = document.createElement('style');
            style.textContent = `
                :root { color-scheme: dark; }
            `;
            if (document.head) {
                document.head.appendChild(style);
            } else {
                document.addEventListener('DOMContentLoaded', () => {
                    document.head.appendChild(style);
                });
            }
        })();
        "#.to_string()
    } else {
        String::new()
    }
}
