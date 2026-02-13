//! Embedded web assets using rust-embed
//!
//! This module embeds the web UI assets directly into the binary,
//! eliminating the need for external file dependencies.

use axum::{
    body::Body,
    http::{header, StatusCode},
    response::Response,
};
use rust_embed::RustEmbed;

/// Embedded web UI assets from the Vite build output
#[derive(RustEmbed)]
#[folder = "web/ivnc/dist"]
pub struct WebAssets;

/// Get an embedded file and return it as an Axum response
pub fn get_embedded_file(path: &str) -> Response {
    // Normalize path: remove leading slash, default to index.html
    let path = path.trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };

    match WebAssets::get(path) {
        Some(content) => {
            let mime = mime_guess::from_path(path).first_or_octet_stream();
            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, mime.as_ref())
                .header(header::CACHE_CONTROL, cache_control_for_path(path))
                .body(Body::from(content.data.into_owned()))
                .unwrap()
        }
        None => Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::from("Not found"))
            .unwrap(),
    }
}

/// Check if embedded assets are available
pub fn has_embedded_assets() -> bool {
    WebAssets::get("index.html").is_some()
}

/// List all embedded files (for debugging)
#[allow(dead_code)]
pub fn list_embedded_files() -> Vec<String> {
    WebAssets::iter().map(|s| s.to_string()).collect()
}

/// Determine cache control header based on file type
fn cache_control_for_path(path: &str) -> &'static str {
    if path == "index.html" {
        "no-store, max-age=0"
    } else if path.ends_with(".js") || path.ends_with(".css") {
        "no-cache, max-age=0"
    } else if path.ends_with(".woff2") || path.ends_with(".woff") || path.ends_with(".ttf") {
        "public, max-age=31536000, immutable"
    } else {
        "public, max-age=3600"
    }
}
