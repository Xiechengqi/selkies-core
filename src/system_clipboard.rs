//! System clipboard integration (X11 via xclip, Wayland via wl-clipboard).

use log::warn;
use std::env;
use std::io::Write;
use std::process::{Command, Stdio};

#[derive(Debug, Clone, Copy)]
enum ClipboardBackend {
    Xclip,
    WlClipboard,
}

fn detect_backend() -> Option<ClipboardBackend> {
    let session = env::var("XDG_SESSION_TYPE").unwrap_or_default().to_ascii_lowercase();
    let wayland_display = env::var("WAYLAND_DISPLAY").unwrap_or_default();
    if (!wayland_display.is_empty() || session == "wayland") && has_command("wl-copy") {
        return Some(ClipboardBackend::WlClipboard);
    }
    if has_command("xclip") {
        return Some(ClipboardBackend::Xclip);
    }
    None
}

pub fn read_text() -> Option<String> {
    match detect_backend() {
        Some(ClipboardBackend::WlClipboard) => {
            read_wl_clipboard(Some("text/plain"))
                .and_then(|bytes| String::from_utf8(bytes).ok())
        }
        Some(ClipboardBackend::Xclip) => {
            read_xclip(Some("text/plain"))
                .and_then(|bytes| String::from_utf8(bytes).ok())
        }
        None => None,
    }
}

pub fn read_binary() -> Option<(String, Vec<u8>)> {
    match detect_backend() {
        Some(ClipboardBackend::WlClipboard) => read_wl_binary(),
        Some(ClipboardBackend::Xclip) => read_xclip_binary(),
        None => None,
    }
}

pub fn write(mime_type: &str, data: &[u8]) -> bool {
    match detect_backend() {
        Some(ClipboardBackend::WlClipboard) => write_wl_clipboard(mime_type, data),
        Some(ClipboardBackend::Xclip) => write_xclip(mime_type, data),
        None => false,
    }
}

fn read_wl_clipboard(mime: Option<&str>) -> Option<Vec<u8>> {
    let mut cmd = Command::new("wl-paste");
    if let Some(mime) = mime {
        cmd.arg("--type").arg(mime);
    }
    let output = cmd.output().ok()?;
    if !output.status.success() {
        return None;
    }
    Some(output.stdout)
}

fn read_xclip(mime: Option<&str>) -> Option<Vec<u8>> {
    let mut cmd = Command::new("xclip");
    cmd.args(["-selection", "clipboard", "-o"]);
    if let Some(mime) = mime {
        cmd.args(["-t", mime]);
    }
    let output = cmd.output().ok()?;
    if !output.status.success() {
        return None;
    }
    Some(output.stdout)
}

fn read_wl_binary() -> Option<(String, Vec<u8>)> {
    let types = Command::new("wl-paste")
        .arg("--list-types")
        .output()
        .ok()?;
    if !types.status.success() {
        return None;
    }
    let list = String::from_utf8_lossy(&types.stdout);
    let preferred = ["image/png", "image/jpeg", "image/webp"];
    let mime = preferred
        .iter()
        .find(|m| list.contains(*m))
        .cloned()
        .or_else(|| list.lines().find(|l| l.starts_with("image/")))?;
    let data = read_wl_clipboard(Some(mime))?;
    Some((mime.to_string(), data))
}

fn read_xclip_binary() -> Option<(String, Vec<u8>)> {
    let output = Command::new("xclip")
        .args(["-selection", "clipboard", "-o", "-t", "TARGETS"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let list = String::from_utf8_lossy(&output.stdout);
    let preferred = ["image/png", "image/jpeg", "image/webp"];
    let mime = preferred
        .iter()
        .find(|m| list.contains(*m))
        .cloned()
        .or_else(|| list.lines().find(|l| l.starts_with("image/")))?;
    let data = read_xclip(Some(mime))?;
    Some((mime.to_string(), data))
}

fn write_wl_clipboard(mime: &str, data: &[u8]) -> bool {
    let mut child = match Command::new("wl-copy")
        .arg("--type")
        .arg(mime)
        .stdin(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(err) => {
            warn!("wl-copy spawn failed: {}", err);
            return false;
        }
    };
    if let Some(stdin) = child.stdin.as_mut() {
        if stdin.write_all(data).is_err() {
            warn!("wl-copy write failed");
        }
    }
    let _ = child.wait();
    true
}

fn write_xclip(mime: &str, data: &[u8]) -> bool {
    let mut child = match Command::new("xclip")
        .args(["-selection", "clipboard", "-i", "-t", mime])
        .stdin(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(err) => {
            warn!("xclip spawn failed: {}", err);
            return false;
        }
    };
    if let Some(stdin) = child.stdin.as_mut() {
        if stdin.write_all(data).is_err() {
            warn!("xclip write failed");
        }
    }
    let _ = child.wait();
    true
}

fn has_command(cmd: &str) -> bool {
    Command::new("sh")
        .arg("-lc")
        .arg(format!("command -v {} >/dev/null 2>&1", cmd))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}
