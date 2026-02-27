//! Clipboard message handling (text + binary, single or multipart).

use crate::web::SharedState;
use crate::system_clipboard;
use base64::Engine;
use log::{info, warn};
use std::sync::Arc;

const MAX_CLIPBOARD_BYTES: usize = 16 * 1024 * 1024;

pub struct ClipboardReceiver {
    state: Arc<SharedState>,
    buffer: Option<Vec<u8>>,
    total_size: usize,
    mime_type: String,
    in_progress: bool,
    is_binary: bool,
}

impl ClipboardReceiver {
    pub fn new(state: Arc<SharedState>) -> Self {
        Self {
            state,
            buffer: None,
            total_size: 0,
            mime_type: "text/plain".to_string(),
            in_progress: false,
            is_binary: false,
        }
    }

    pub fn handle_message(&mut self, message: &str) -> bool {
        if !self.state.config.input.enable_clipboard {
            return false;
        }

        if message.starts_with("cw,") {
            let payload = message.trim_start_matches("cw,");
            self.handle_single_text(payload);
            return true;
        }
        // Legacy format: "c,<base64>"
        if message.starts_with("c,") {
            let payload = message.trim_start_matches("c,");
            self.handle_single_text(payload);
            return true;
        }
        if message.starts_with("cb,") {
            let payload = message.trim_start_matches("cb,");
            self.handle_single_binary(payload);
            return true;
        }
        if message.starts_with("cws,") {
            let payload = message.trim_start_matches("cws,");
            self.start_multipart("text/plain", payload, false);
            return true;
        }
        if message.starts_with("cbs,") {
            let payload = message.trim_start_matches("cbs,");
            self.start_multipart_binary(payload);
            return true;
        }
        if message.starts_with("cwd,") {
            let payload = message.trim_start_matches("cwd,");
            self.handle_chunk(payload);
            return true;
        }
        if message.starts_with("cbd,") {
            let payload = message.trim_start_matches("cbd,");
            self.handle_chunk(payload);
            return true;
        }
        if message == "cwe" || message == "cbe" {
            self.finish_multipart();
            return true;
        }

        false
    }

    fn handle_single_text(&self, base64_payload: &str) {
        match decode_base64(base64_payload) {
            Some(bytes) => {
                if bytes.len() > MAX_CLIPBOARD_BYTES {
                    warn!("Clipboard payload exceeds limit ({} bytes)", bytes.len());
                    return;
                }
                let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
                let _ = self.state.clipboard_incoming_tx.send(encoded.clone());
                self.state.set_clipboard(encoded);
            }
            None => warn!("Failed to decode clipboard text payload"),
        }
    }

    fn handle_single_binary(&self, payload: &str) {
        if !self.state.runtime_settings.binary_clipboard_enabled() {
            warn!("Binary clipboard is disabled; ignoring payload");
            return;
        }
        let mut parts = payload.splitn(2, ',');
        let mime = parts.next().unwrap_or("application/octet-stream");
        let b64 = parts.next().unwrap_or_default();
        match decode_base64(b64) {
            Some(bytes) => {
                if bytes.len() > MAX_CLIPBOARD_BYTES {
                    warn!("Clipboard payload exceeds limit ({} bytes)", bytes.len());
                    return;
                }
                if system_clipboard::write(mime, &bytes) {
                    self.state.mark_clipboard_written(mime, &bytes);
                }
                self.state.set_clipboard_binary(mime.to_string(), bytes);
            }
            None => warn!("Failed to decode binary clipboard payload"),
        }
    }

    fn start_multipart(&mut self, mime: &str, total_str: &str, is_binary: bool) {
        let total_size = match total_str.trim().parse::<usize>() {
            Ok(value) => value,
            Err(_) => {
                warn!("Invalid multipart clipboard size: {}", total_str);
                return;
            }
        };
        if total_size == 0 || total_size > MAX_CLIPBOARD_BYTES {
            warn!("Multipart clipboard size {} exceeds limit", total_size);
            return;
        }
        self.buffer = Some(Vec::with_capacity(total_size));
        self.total_size = total_size;
        self.mime_type = mime.to_string();
        self.in_progress = true;
        self.is_binary = is_binary;
        info!("Started multipart clipboard receive: {} bytes ({})", total_size, mime);
    }

    fn start_multipart_binary(&mut self, payload: &str) {
        if !self.state.runtime_settings.binary_clipboard_enabled() {
            warn!("Binary clipboard is disabled; ignoring multipart start");
            return;
        }
        let mut parts = payload.splitn(2, ',');
        let mime = parts.next().unwrap_or("application/octet-stream");
        let total_str = parts.next().unwrap_or("0");
        self.start_multipart(mime, total_str, true);
    }

    fn handle_chunk(&mut self, base64_payload: &str) {
        if !self.in_progress {
            warn!("Clipboard chunk received without active multipart transfer");
            return;
        }
        if let Some(chunk) = decode_base64(base64_payload) {
            if let Some(buffer) = self.buffer.as_mut() {
                if buffer.len().saturating_add(chunk.len()) > self.total_size {
                    warn!("Clipboard chunk exceeds declared size; aborting transfer");
                    self.reset();
                    return;
                }
                buffer.extend_from_slice(&chunk);
            }
        } else {
            warn!("Failed to decode clipboard chunk");
            self.reset();
        }
    }

    fn finish_multipart(&mut self) {
        if !self.in_progress {
            return;
        }
        let buffer = match self.buffer.take() {
            Some(value) => value,
            None => {
                self.reset();
                return;
            }
        };
        if buffer.len() != self.total_size {
            warn!(
                "Clipboard multipart size mismatch: expected {}, got {}",
                self.total_size,
                buffer.len()
            );
            self.reset();
            return;
        }

        if self.is_binary {
            if system_clipboard::write(&self.mime_type, &buffer) {
                self.state.mark_clipboard_written(&self.mime_type, &buffer);
            }
            self.state
                .set_clipboard_binary(self.mime_type.clone(), buffer);
        } else {
            if system_clipboard::write("text/plain", &buffer) {
                self.state.mark_clipboard_written("text/plain", &buffer);
            }
            let encoded = base64::engine::general_purpose::STANDARD.encode(buffer);
            let _ = self.state.clipboard_incoming_tx.send(encoded.clone());
            self.state.set_clipboard(encoded);
        }
        self.reset();
    }

    fn reset(&mut self) {
        self.buffer = None;
        self.total_size = 0;
        self.mime_type = "text/plain".to_string();
        self.in_progress = false;
        self.is_binary = false;
    }
}

fn decode_base64(payload: &str) -> Option<Vec<u8>> {
    base64::engine::general_purpose::STANDARD
        .decode(payload)
        .ok()
}
