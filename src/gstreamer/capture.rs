//! GStreamer-based X11 screen capture
//!
//! Provides ximagesrc-based screen capture with XShm acceleration.

#![allow(dead_code)]

use super::GstError;
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use gstreamer_video as gst_video;
use log::info;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

/// Configuration for X11 capture
#[derive(Debug, Clone)]
pub struct CaptureConfig {
    /// X11 display (e.g., ":0")
    pub display: String,
    /// Enable cursor capture
    pub show_cursor: bool,
    /// Use XDamage for efficient updates
    pub use_damage: bool,
    /// Target framerate
    pub framerate: u32,
    /// Capture region start X (0 for full screen)
    pub start_x: u32,
    /// Capture region start Y (0 for full screen)
    pub start_y: u32,
    /// Capture width (0 for full screen)
    pub width: u32,
    /// Capture height (0 for full screen)
    pub height: u32,
}

impl Default for CaptureConfig {
    fn default() -> Self {
        Self {
            display: std::env::var("DISPLAY").unwrap_or_else(|_| ":0".to_string()),
            show_cursor: false,
            use_damage: false,  // Disable for lower latency
            framerate: 30,
            start_x: 0,
            start_y: 0,
            width: 0,
            height: 0,
        }
    }
}

/// Raw frame data from capture
pub struct CapturedFrame {
    /// Raw pixel data (BGRA format)
    pub data: Vec<u8>,
    /// Frame width
    pub width: u32,
    /// Frame height
    pub height: u32,
    /// Timestamp in nanoseconds
    pub timestamp: u64,
    /// Frame sequence number
    pub sequence: u64,
}

/// GStreamer-based X11 screen capturer
pub struct GstCapturer {
    pipeline: gst::Pipeline,
    appsink: gst_app::AppSink,
    config: CaptureConfig,
    running: Arc<AtomicBool>,
    width: Arc<AtomicU32>,
    height: Arc<AtomicU32>,
    frame_count: u64,
}

impl GstCapturer {
    /// Create a new GStreamer capturer
    pub fn new(config: CaptureConfig) -> Result<Self, GstError> {
        gst::init().map_err(|e| GstError::InitFailed(e.to_string()))?;

        let pipeline = gst::Pipeline::new();

        // Create ximagesrc
        let mut src_builder = gst::ElementFactory::make("ximagesrc")
            .property_from_str("display-name", &config.display)
            .property("show-pointer", config.show_cursor)
            .property("use-damage", config.use_damage);

        // Set capture region if specified
        if config.width > 0 && config.height > 0 {
            src_builder = src_builder
                .property("startx", config.start_x)
                .property("starty", config.start_y)
                .property("endx", config.start_x + config.width - 1)
                .property("endy", config.start_y + config.height - 1);
        }

        let src = src_builder.build()
            .map_err(|e| GstError::PipelineFailed(format!("Failed to create ximagesrc: {}", e)))?;

        // Create videoconvert to ensure consistent output format
        let convert = gst::ElementFactory::make("videoconvert")
            .build()
            .map_err(|e| GstError::PipelineFailed(format!("Failed to create videoconvert: {}", e)))?;

        // Create videorate for framerate control
        let rate = gst::ElementFactory::make("videorate")
            .property("max-rate", config.framerate as i32)
            .build()
            .map_err(|e| GstError::PipelineFailed(format!("Failed to create videorate: {}", e)))?;

        // Create capsfilter for framerate and format
        let caps = gst::Caps::builder("video/x-raw")
            .field("format", "BGRA")
            .field("framerate", gst::Fraction::new(config.framerate as i32, 1))
            .build();

        let capsfilter = gst::ElementFactory::make("capsfilter")
            .property("caps", &caps)
            .build()
            .map_err(|e| GstError::PipelineFailed(format!("Failed to create capsfilter: {}", e)))?;

        // Create appsink for raw frame output
        let appsink = gst_app::AppSink::builder()
            .name("rawsink")
            .sync(false)
            .max_buffers(2)
            .drop(true)
            .caps(&caps)
            .build();

        // Add elements to pipeline
        pipeline.add_many([&src, &convert, &rate, &capsfilter, appsink.upcast_ref()])
            .map_err(|e| GstError::PipelineFailed(format!("Failed to add elements: {}", e)))?;

        // Link elements
        gst::Element::link_many([&src, &convert, &rate, &capsfilter, appsink.upcast_ref()])
            .map_err(|e| GstError::LinkFailed(format!("Failed to link elements: {}", e)))?;

        Ok(Self {
            pipeline,
            appsink,
            config,
            running: Arc::new(AtomicBool::new(false)),
            width: Arc::new(AtomicU32::new(0)),
            height: Arc::new(AtomicU32::new(0)),
            frame_count: 0,
        })
    }

    /// Start capturing
    pub fn start(&self) -> Result<(), GstError> {
        info!("Starting GStreamer capture on display {}", self.config.display);

        self.pipeline
            .set_state(gst::State::Playing)
            .map_err(|e| GstError::StateChangeFailed(format!("Failed to start capture: {}", e)))?;

        self.running.store(true, Ordering::SeqCst);
        Ok(())
    }

    /// Stop capturing
    pub fn stop(&self) -> Result<(), GstError> {
        info!("Stopping GStreamer capture");

        self.running.store(false, Ordering::SeqCst);

        self.pipeline
            .set_state(gst::State::Null)
            .map_err(|e| GstError::StateChangeFailed(format!("Failed to stop capture: {}", e)))?;

        Ok(())
    }

    /// Check if capturer is running
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    /// Capture a single frame (blocking with timeout)
    pub fn capture_frame(&mut self, _timeout_ms: u64) -> Option<CapturedFrame> {
        let sample = self.appsink
            .pull_sample()
            .ok()?;

        self.process_sample(sample)
    }

    /// Try to capture a frame (non-blocking)
    pub fn try_capture_frame(&mut self) -> Option<CapturedFrame> {
        let sample = self.appsink.try_pull_sample(gst::ClockTime::ZERO)?;
        self.process_sample(sample)
    }

    /// Process a GStreamer sample into a CapturedFrame
    fn process_sample(&mut self, sample: gst::Sample) -> Option<CapturedFrame> {
        let buffer = sample.buffer()?;
        let caps = sample.caps()?;
        let info = gst_video::VideoInfo::from_caps(caps).ok()?;

        let width = info.width();
        let height = info.height();

        // Update cached dimensions
        self.width.store(width, Ordering::Relaxed);
        self.height.store(height, Ordering::Relaxed);

        let map = buffer.map_readable().ok()?;
        let data = map.as_slice().to_vec();

        let timestamp = buffer.pts()
            .map(|pts| pts.nseconds())
            .unwrap_or(0);

        self.frame_count += 1;

        Some(CapturedFrame {
            data,
            width,
            height,
            timestamp,
            sequence: self.frame_count,
        })
    }

    /// Get current capture dimensions
    pub fn dimensions(&self) -> (u32, u32) {
        (
            self.width.load(Ordering::Relaxed),
            self.height.load(Ordering::Relaxed),
        )
    }

    /// Get frame count
    pub fn frame_count(&self) -> u64 {
        self.frame_count
    }

    /// Get the configuration
    pub fn config(&self) -> &CaptureConfig {
        &self.config
    }
}

impl Drop for GstCapturer {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

// Need to import gstreamer_video for VideoInfo
use gstreamer_video;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_capture_config_default() {
        let config = CaptureConfig::default();
        assert_eq!(config.framerate, 30);
        assert!(!config.show_cursor);
    }
}
