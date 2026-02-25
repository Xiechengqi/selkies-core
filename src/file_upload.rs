//! File upload handling for WebRTC and WebSocket data channels.

use crate::config::Config;
use log::{error, info, warn};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Component, PathBuf};

#[derive(Debug, Clone)]
pub struct FileUploadSettings {
    pub upload_dir: Option<PathBuf>,
    pub allow_upload: bool,
}

impl FileUploadSettings {
    pub fn from_config(config: &Config) -> Self {
        let allow_upload = config
            .input
            .file_transfers
            .iter()
            .any(|entry| entry.trim().eq_ignore_ascii_case("upload"));
        let upload_dir = if allow_upload {
            resolve_upload_dir(&config.input.upload_dir)
        } else {
            None
        };

        Self {
            upload_dir,
            allow_upload,
        }
    }
}

pub struct FileUploadHandler {
    settings: FileUploadSettings,
    active_path: Option<PathBuf>,
    active_file: Option<File>,
    expected_size: Option<u64>,
    written_size: u64,
}

impl FileUploadHandler {
    pub fn new(settings: FileUploadSettings) -> Self {
        Self {
            settings,
            active_path: None,
            active_file: None,
            expected_size: None,
            written_size: 0,
        }
    }

    #[allow(dead_code)]
    pub fn from_config(config: &Config) -> Self {
        Self::new(FileUploadSettings::from_config(config))
    }

    pub fn handle_control_message(&mut self, message: &str) -> bool {
        if message.starts_with("FILE_UPLOAD_START:") {
            if !self.is_upload_allowed() {
                warn!("File upload requested but uploads are disabled");
                return true;
            }
            let payload = message.trim_start_matches("FILE_UPLOAD_START:");
            let mut parts = payload.splitn(2, ':');
            let rel_path = parts.next().unwrap_or_default();
            let size = parts.next().unwrap_or_default();
            if let Err(err) = self.start_upload(rel_path, size) {
                error!("File upload start failed: {}", err);
                self.abort_active();
            }
            return true;
        }

        if message.starts_with("FILE_UPLOAD_END:") {
            let payload = message.trim_start_matches("FILE_UPLOAD_END:");
            info!("Received FILE_UPLOAD_END for {}", payload);
            self.finish_upload();
            return true;
        }

        if message.starts_with("FILE_UPLOAD_ERROR:") {
            let payload = message.trim_start_matches("FILE_UPLOAD_ERROR:");
            error!("Client reported upload error: {}", payload);
            self.abort_active();
            return true;
        }

        false
    }

    pub fn handle_binary(&mut self, data: &[u8]) {
        if data.is_empty() {
            return;
        }
        if data[0] != 0x01 {
            return;
        }
        let payload = &data[1..];
        if let Some(file) = self.active_file.as_mut() {
            if let Some(expected) = self.expected_size {
                let next = self.written_size.saturating_add(payload.len() as u64);
                if next > expected {
                    error!(
                        "Upload exceeded declared size (expected {}, got {})",
                        expected,
                        next
                    );
                    self.abort_active();
                    return;
                }
            }
            if let Err(err) = file.write_all(payload) {
                error!(
                    "File write error for {:?}: {}",
                    self.active_path.as_ref().map(|p| p.as_path()),
                    err
                );
                self.abort_active();
                return;
            }
            self.written_size = self.written_size.saturating_add(payload.len() as u64);
        } else {
            warn!("Received file data after upload path is closed");
        }
    }

    pub fn abort_active(&mut self) {
        if let Some(mut file) = self.active_file.take() {
            let _ = file.flush();
        }
        if let Some(path) = self.active_path.take() {
            if let Err(err) = fs::remove_file(&path) {
                warn!("Failed to remove incomplete upload {:?}: {}", path, err);
            } else {
                info!("Purged incomplete upload {:?}", path);
            }
        }
        self.expected_size = None;
        self.written_size = 0;
    }

    pub fn finish_upload(&mut self) {
        if let Some(mut file) = self.active_file.take() {
            if let Err(err) = file.flush() {
                warn!("Failed to flush upload file: {}", err);
            }
        }
        if let Some(path) = self.active_path.take() {
            if let Some(expected) = self.expected_size {
                if self.written_size != expected {
                    warn!(
                        "Upload size mismatch for {:?}: expected {}, got {}",
                        path,
                        expected,
                        self.written_size
                    );
                    let _ = fs::remove_file(&path);
                } else {
                    info!("Upload finished: {:?}", path);
                }
            } else {
                info!("Upload finished: {:?}", path);
            }
        }
        self.expected_size = None;
        self.written_size = 0;
    }

    fn is_upload_allowed(&self) -> bool {
        self.settings.allow_upload && self.settings.upload_dir.is_some()
    }

    fn start_upload(&mut self, rel_path: &str, size_str: &str) -> Result<(), String> {
        let upload_dir = self
            .settings
            .upload_dir
            .as_ref()
            .ok_or_else(|| "Upload directory is not configured".to_string())?;

        let size = size_str
            .trim()
            .parse::<u64>()
            .map_err(|_| "Invalid file size")?;
        if size == 0 {
            return Err("Invalid file size".to_string());
        }
        const MAX_UPLOAD_BYTES: u64 = 512 * 1024 * 1024;
        if size > MAX_UPLOAD_BYTES {
            return Err(format!("Upload exceeds size limit ({} bytes)", MAX_UPLOAD_BYTES));
        }

        let safe_rel = sanitize_relative_path(rel_path)
            .ok_or_else(|| format!("Invalid relative path: {}", rel_path))?;

        let upload_root = upload_dir.to_path_buf();
        let target_path = upload_root.join(&safe_rel);
        let target_dir = target_path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| upload_root.clone());

        if !target_dir.starts_with(&upload_root) {
            return Err(format!(
                "Path escape attempt detected: {:?} is outside {:?}",
                target_path, upload_root
            ));
        }

        if target_dir != upload_root {
            if let Err(err) = fs::create_dir_all(&target_dir) {
                return Err(format!("Failed to create upload directory {:?}: {}", target_dir, err));
            }
        }

        let root_canon = fs::canonicalize(&upload_root)
            .map_err(|err| format!("Failed to canonicalize upload root {:?}: {}", upload_root, err))?;
        let target_dir_canon = fs::canonicalize(&target_dir)
            .map_err(|err| format!("Failed to canonicalize upload target {:?}: {}", target_dir, err))?;
        if !target_dir_canon.starts_with(&root_canon) {
            return Err(format!(
                "Path escape attempt detected via symlink: {:?} is outside {:?}",
                target_dir_canon, root_canon
            ));
        }
        if let Ok(meta) = fs::symlink_metadata(&target_path) {
            if meta.file_type().is_symlink() {
                return Err(format!("Refusing to follow symlink target {:?}", target_path));
            }
        }

        if self.active_file.is_some() {
            warn!("Closing previous upload before starting new one");
            self.finish_upload();
        }

        let file = File::create(&target_path)
            .map_err(|err| format!("Failed to create upload file {:?}: {}", target_path, err))?;
        self.active_file = Some(file);
        self.active_path = Some(target_path.clone());
        self.expected_size = Some(size);
        self.written_size = 0;
        info!("Upload started: {:?}", target_path);
        Ok(())
    }
}

fn resolve_upload_dir(raw: &str) -> Option<PathBuf> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed == "/sys" || trimmed == "/proc" || trimmed == "/dev" {
        warn!("Refusing to use upload directory {}", trimmed);
        return None;
    }
    let expanded = if trimmed == "~/Desktop" || trimmed.starts_with("~/") {
        if let Ok(home) = std::env::var("HOME") {
            let suffix = trimmed.trim_start_matches("~/");
            PathBuf::from(home).join(suffix)
        } else {
            PathBuf::from(trimmed)
        }
    } else {
        PathBuf::from(trimmed)
    };

    if let Err(err) = fs::create_dir_all(&expanded) {
        warn!("Could not create upload directory {:?}: {}", expanded, err);
        return None;
    }
    Some(expanded)
}

fn sanitize_relative_path(rel_path: &str) -> Option<PathBuf> {
    let trimmed = rel_path
        .trim()
        .trim_start_matches(&['/', '\\'][..]);
    if trimmed.is_empty() {
        return None;
    }

    let normalized = trimmed.replace('\\', "/");
    let mut safe = PathBuf::new();
    for component in PathBuf::from(normalized).components() {
        match component {
            Component::Normal(part) => safe.push(part),
            Component::CurDir => {}
            _ => return None,
        }
    }

    if safe.as_os_str().is_empty() {
        None
    } else {
        Some(safe)
    }
}
