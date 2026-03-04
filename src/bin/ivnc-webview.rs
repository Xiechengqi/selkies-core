/// Standalone WebView process for Pake applications
/// This binary is launched by iVnc's WebViewManager to run each WebView app in isolation
use std::env;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::sync::{Arc, Mutex};
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::platform::unix::EventLoopBuilderExtUnix;
use tao::window::WindowBuilder;
use tao::platform::unix::WindowExtUnix;
use wry::{WebViewBuilder, WebViewBuilderExtUnix, WebViewExtUnix};
use gtk::prelude::*;

fn main() {
    env_logger::init();

    // Read configuration from environment variables
    let app_id = env::var("IVNC_APP_ID").expect("IVNC_APP_ID not set");
    let app_name = env::var("IVNC_APP_NAME").expect("IVNC_APP_NAME not set");
    let app_url = env::var("IVNC_APP_URL").expect("IVNC_APP_URL not set");
    let dark_mode = env::var("IVNC_DARK_MODE").unwrap_or_else(|_| "false".to_string()) == "true";
    let log_file_path = env::var("IVNC_LOG_FILE").expect("IVNC_LOG_FILE not set");
    let data_dir = env::var("IVNC_DATA_DIR").expect("IVNC_DATA_DIR not set");

    log::info!("Starting WebView process for app '{}' ({})", app_name, app_id);
    log::info!("URL: {}", app_url);
    log::info!("Data dir: {}", data_dir);

    // Set WebKitGTK data directory
    std::env::set_var("WEBKIT_DISK_CACHE_DIR", format!("{}/cache", data_dir));
    std::env::set_var("WEBKIT_LOCALSTORAGE_DIR", format!("{}/localstorage", data_dir));
    std::env::set_var("WEBKIT_INDEXEDDB_DIR", format!("{}/indexeddb", data_dir));

    // Create log file
    let log_file = Arc::new(Mutex::new(
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_file_path)
            .expect("Failed to create log file")
    ));

    // Create event loop
    let event_loop = EventLoopBuilder::new()
        .with_any_thread(true)
        .build();

    // Create window with app_id set
    let window = WindowBuilder::new()
        .with_title(&app_name)
        .with_inner_size(tao::dpi::LogicalSize::new(1280, 720))
        .build(&event_loop)
        .expect("Failed to create window");

    log::info!("Window created: {}", app_name);

    // Set window properties for Wayland app_id
    let gtk_window = window.gtk_window();
    gtk_window.set_title(&app_name);
    gtk_window.set_icon_name(Some(&app_name));
    gtk_window.set_role(&app_name);

    // CRITICAL: Set the program name which becomes the Wayland app_id
    glib::set_program_name(Some(&app_name));
    glib::set_application_name(&app_name);

    log::info!("Set program name and app identifiers to: {}", app_name);

    // Create GTK container (WebView fills entire window; nav buttons injected via JS)
    let container = {
        let vbox = window.default_vbox()
            .expect("Failed to get GTK vbox");
        let container = gtk::Box::new(gtk::Orientation::Vertical, 0);
        vbox.pack_start(&container, true, true, 0);
        container.show_all();
        container
    };

    // Build initialization script
    let init_script = build_init_script(dark_mode, log_file.clone());

    // Setup IPC handler
    let log_file_for_ipc = log_file.clone();
    let ipc_handler = move |req: wry::http::Request<String>| {
        let msg = req.body();
        if msg.starts_with("console:") {
            let log_msg = msg.strip_prefix("console:").unwrap_or(msg);
            if let Ok(mut file) = log_file_for_ipc.lock() {
                let _ = writeln!(file, "{}", log_msg);
                let _ = file.flush();
            }
        }
    };

    // Build WebView
    log::info!("Building WebView with URL: {}", app_url);
    let webview = WebViewBuilder::new()
        .with_url(&app_url)
        .with_initialization_script(&init_script)
        .with_ipc_handler(ipc_handler)
        .build_gtk(&container)
        .expect("Failed to build WebView");

    log::info!("WebView built successfully");

    // Add WebView widget to container
    let webview_widget = webview.webview();
    container.pack_start(&webview_widget, true, true, 0);
    webview_widget.show();

    log::info!("WebView process ready");

    // Run event loop
    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;

        match event {
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                log::info!("Window close requested, exiting");
                *control_flow = ControlFlow::Exit;
            }
            Event::WindowEvent {
                event: WindowEvent::Destroyed,
                ..
            } => {
                log::info!("Window destroyed, exiting");
                *control_flow = ControlFlow::Exit;
            }
            _ => {}
        }
    });
}

fn build_init_script(dark_mode: bool, _log_file: Arc<Mutex<File>>) -> String {
    let dark_mode_script = if dark_mode {
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
        // Console logging via IPC
        (function() {{
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

            console.log('iVnc WebView console logging active');
        }})();

        // Navigation buttons (Pake-style, injected after DOM ready)
        (function() {{
            const NAV_CSS = `
                #pake-nav {{
                    position: fixed;
                    top: 8px;
                    left: 8px;
                    z-index: 2147483647;
                    display: flex;
                    gap: 2px;
                    padding: 4px;
                    border-radius: 16px;
                    background: rgba(255, 255, 255, 0.72);
                    backdrop-filter: blur(12px);
                    -webkit-backdrop-filter: blur(12px);
                    box-shadow: 0 1px 4px rgba(0, 0, 0, 0.12);
                    user-select: none;
                }}
                #pake-nav button {{
                    width: 28px;
                    height: 28px;
                    border: none;
                    border-radius: 50%;
                    background: transparent;
                    cursor: pointer;
                    font-size: 14px;
                    padding: 0;
                    color: #333;
                    display: flex;
                    align-items: center;
                    justify-content: center;
                }}
                #pake-nav button:hover {{ background: rgba(0,0,0,0.08); }}
                #pake-nav button:active {{ background: rgba(0,0,0,0.14); }}
                @media (prefers-color-scheme: dark) {{
                    #pake-nav {{ background: rgba(40,40,40,0.78); box-shadow: 0 1px 4px rgba(0,0,0,0.32); }}
                    #pake-nav button {{ color: #ddd; }}
                    #pake-nav button:hover {{ background: rgba(255,255,255,0.1); }}
                    #pake-nav button:active {{ background: rgba(255,255,255,0.18); }}
                }}
            `;

            function injectNav() {{
                if (document.getElementById('pake-nav')) return;

                const style = document.createElement('style');
                style.textContent = NAV_CSS;
                document.head.appendChild(style);

                const nav = document.createElement('div');
                nav.id = 'pake-nav';

                const backBtn = document.createElement('button');
                backBtn.textContent = '\u25C0';
                backBtn.title = 'Back (Alt+Left)';
                backBtn.addEventListener('click', () => history.back());

                const fwdBtn = document.createElement('button');
                fwdBtn.textContent = '\u25B6';
                fwdBtn.title = 'Forward (Alt+Right)';
                fwdBtn.addEventListener('click', () => history.forward());

                const reloadBtn = document.createElement('button');
                reloadBtn.textContent = '\u21BB';
                reloadBtn.title = 'Refresh (F5)';
                reloadBtn.addEventListener('click', () => location.reload());

                nav.appendChild(backBtn);
                nav.appendChild(fwdBtn);
                nav.appendChild(reloadBtn);
                document.body.appendChild(nav);
            }}

            if (document.readyState === 'loading') {{
                document.addEventListener('DOMContentLoaded', injectNav);
            }} else {{
                injectNav();
            }}

            // Re-inject on navigation (SPA support)
            const observer = new MutationObserver(() => {{
                if (!document.getElementById('pake-nav') && document.body) {{
                    injectNav();
                }}
            }});
            document.addEventListener('DOMContentLoaded', () => {{
                observer.observe(document.body, {{ childList: true, subtree: false }});
            }});

            document.addEventListener('keydown', (e) => {{
                if (e.altKey && e.key === 'ArrowLeft') {{ e.preventDefault(); history.back(); }}
                else if (e.altKey && e.key === 'ArrowRight') {{ e.preventDefault(); history.forward(); }}
                else if (e.key === 'F5' || (e.ctrlKey && e.key === 'r')) {{ e.preventDefault(); location.reload(); }}
            }});
        }})();
        "#,
        dark_mode_script
    )
}
