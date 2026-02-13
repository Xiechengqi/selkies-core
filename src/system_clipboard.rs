//! System clipboard integration (Wayland via wl-clipboard).

use log::warn;
use std::io::Write;
use std::process::{Command, Stdio};

#[allow(dead_code)]
pub fn read_text() -> Option<String> {
    let output = Command::new("wl-paste")
        .arg("--type")
        .arg("text/plain")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout).ok()
}

#[allow(dead_code)]
pub fn read_binary() -> Option<(String, Vec<u8>)> {
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
    let data = Command::new("wl-paste")
        .arg("--type")
        .arg(mime)
        .output()
        .ok()?;
    if !data.status.success() {
        return None;
    }
    Some((mime.to_string(), data.stdout))
}

pub fn write(mime_type: &str, data: &[u8]) -> bool {
    let mut child = match Command::new("wl-copy")
        .arg("--type")
        .arg(mime_type)
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
