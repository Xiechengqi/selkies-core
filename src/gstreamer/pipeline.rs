//! GStreamer video capture and encoding pipeline
//!
//! Provides a complete pipeline for:
//! - X11 screen capture via ximagesrc
//! - Video encoding (H.264, VP8, VP9)

#![allow(dead_code)]
//! - RTP packetization for WebRTC

use super::{GstError, encoder::EncoderSelection};
use crate::config::{VideoCodec, HardwareEncoder, WebRTCConfig};
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use log::{info, warn, debug};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

/// Pipeline state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PipelineState {
    /// Pipeline is stopped
    Stopped,
    /// Pipeline is starting
    Starting,
    /// Pipeline is running
    Running,
    /// Pipeline is paused
    Paused,
    /// Pipeline encountered an error
    Error,
}

/// Pipeline configuration
#[derive(Debug, Clone)]
pub struct PipelineConfig {
    /// X11 display string (e.g., ":0")
    pub display: String,
    /// Capture width (0 for auto-detect)
    pub width: u32,
    /// Capture height (0 for auto-detect)
    pub height: u32,
    /// Target framerate
    pub framerate: u32,
    /// Video codec
    pub codec: VideoCodec,
    /// Target bitrate in kbps
    pub bitrate: u32,
    /// Hardware encoder preference
    pub hardware_encoder: HardwareEncoder,
    /// Keyframe interval in frames
    pub keyframe_interval: u32,
    /// Pipeline latency in ms
    pub latency_ms: u32,
    /// Show cursor in capture
    pub show_cursor: bool,
}

impl From<&WebRTCConfig> for PipelineConfig {
    fn from(config: &WebRTCConfig) -> Self {
        Self {
            display: std::env::var("DISPLAY").unwrap_or_else(|_| ":0".to_string()),
            width: 0,
            height: 0,
            framerate: 30,
            codec: config.video_codec,
            bitrate: config.video_bitrate,
            hardware_encoder: config.hardware_encoder,
            keyframe_interval: config.keyframe_interval,
            latency_ms: config.pipeline_latency_ms,
            show_cursor: false,
        }
    }
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            display: std::env::var("DISPLAY").unwrap_or_else(|_| ":0".to_string()),
            width: 0,
            height: 0,
            framerate: 30,
            codec: VideoCodec::H264,
            bitrate: 4000,
            hardware_encoder: HardwareEncoder::Auto,
            keyframe_interval: 60,
            latency_ms: 50,
            show_cursor: false,
        }
    }
}

/// RTP packet callback type
pub type RtpCallback = Box<dyn Fn(&[u8], u32, u64) + Send + Sync>;

/// Video pipeline for GStreamer-based capture and encoding
pub struct VideoPipeline {
    pipeline: gst::Pipeline,
    appsink: gst_app::AppSink,
    config: PipelineConfig,
    state: Arc<AtomicBool>,  // true = running
    frame_count: Arc<AtomicU64>,
    encoder_element: String,
}

impl VideoPipeline {
    /// Create a new video pipeline
    pub fn new(config: PipelineConfig) -> Result<Self, GstError> {
        // Ensure GStreamer is initialized
        gst::init().map_err(|e| GstError::InitFailed(e.to_string()))?;

        let pipeline = gst::Pipeline::new();

        // Create source element (ximagesrc for X11)
        let src = gst::ElementFactory::make("ximagesrc")
            .property_from_str("display-name", &config.display)
            .property("show-pointer", config.show_cursor)
            .property("use-damage", false)  // Disable for lower latency
            .build()
            .map_err(|e| GstError::PipelineFailed(format!("Failed to create ximagesrc: {}", e)))?;

        // Create videoconvert for format conversion
        let convert = gst::ElementFactory::make("videoconvert")
            .build()
            .map_err(|e| GstError::PipelineFailed(format!("Failed to create videoconvert: {}", e)))?;

        // Create videoscale for optional scaling
        let scale = gst::ElementFactory::make("videoscale")
            .build()
            .map_err(|e| GstError::PipelineFailed(format!("Failed to create videoscale: {}", e)))?;

        // Create videorate for framerate control
        let rate = gst::ElementFactory::make("videorate")
            .property("max-rate", config.framerate as i32)
            .build()
            .map_err(|e| GstError::PipelineFailed(format!("Failed to create videorate: {}", e)))?;

        // Select and create encoder
        let encoder_selection = EncoderSelection::select(
            config.codec,
            config.hardware_encoder,
        );

        let (encoder, encoder_name) = encoder_selection.create_encoder(
            config.bitrate,
            config.keyframe_interval,
        )?;

        info!("Using encoder: {} for codec {:?}", encoder_name, config.codec);

        // Create RTP payloader based on codec
        let payloader = Self::create_payloader(config.codec)?;

        // Create appsink for RTP packet output
        let appsink = gst_app::AppSink::builder()
            .name("rtpsink")
            .sync(false)
            .max_buffers(2)
            .drop(true)
            .build();

        // Add elements to pipeline
        pipeline.add_many([
            &src,
            &convert,
            &scale,
            &rate,
            &encoder,
            &payloader,
            appsink.upcast_ref(),
        ]).map_err(|e| GstError::PipelineFailed(format!("Failed to add elements: {}", e)))?;

        // Create caps filter for framerate
        let caps_str = if config.width > 0 && config.height > 0 {
            format!(
                "video/x-raw,framerate={}/1,width={},height={}",
                config.framerate, config.width, config.height
            )
        } else {
            format!("video/x-raw,framerate={}/1", config.framerate)
        };

        let caps = caps_str.parse::<gst::Caps>()
            .map_err(|e| GstError::PipelineFailed(format!("Invalid caps: {}", e)))?;

        // Link elements
        src.link(&convert)
            .map_err(|e| GstError::LinkFailed(format!("src->convert: {}", e)))?;
        convert.link(&scale)
            .map_err(|e| GstError::LinkFailed(format!("convert->scale: {}", e)))?;
        scale.link_filtered(&rate, &caps)
            .map_err(|e| GstError::LinkFailed(format!("scale->rate: {}", e)))?;
        rate.link(&encoder)
            .map_err(|e| GstError::LinkFailed(format!("rate->encoder: {}", e)))?;
        encoder.link(&payloader)
            .map_err(|e| GstError::LinkFailed(format!("encoder->payloader: {}", e)))?;
        payloader.link(appsink.upcast_ref::<gst::Element>())
            .map_err(|e| GstError::LinkFailed(format!("payloader->appsink: {}", e)))?;

        // Set pipeline latency
        pipeline.set_latency(gst::ClockTime::from_mseconds(config.latency_ms as u64));

        Ok(Self {
            pipeline,
            appsink,
            config,
            state: Arc::new(AtomicBool::new(false)),
            frame_count: Arc::new(AtomicU64::new(0)),
            encoder_element: encoder_name,
        })
    }

    /// Create RTP payloader for the specified codec
    fn create_payloader(codec: VideoCodec) -> Result<gst::Element, GstError> {
        let (element_name, pt) = match codec {
            VideoCodec::H264 => ("rtph264pay", 96),
            VideoCodec::VP8 => ("rtpvp8pay", 97),
            VideoCodec::VP9 => ("rtpvp9pay", 98),
            VideoCodec::AV1 => ("rtpav1pay", 99),
        };

        gst::ElementFactory::make(element_name)
            .property("pt", pt as u32)
            .property("config-interval", -1i32)  // Send config with every IDR
            // Note: aggregate-mode requires enum type, skip for now
            .build()
            .map_err(|e| GstError::PipelineFailed(format!("Failed to create {}: {}", element_name, e)))
    }

    /// Start the pipeline
    pub fn start(&self) -> Result<(), GstError> {
        info!("Starting GStreamer pipeline with encoder: {}", self.encoder_element);

        self.pipeline
            .set_state(gst::State::Playing)
            .map_err(|e| GstError::StateChangeFailed(format!("Failed to start pipeline: {}", e)))?;

        self.state.store(true, Ordering::SeqCst);
        Ok(())
    }

    /// Stop the pipeline
    pub fn stop(&self) -> Result<(), GstError> {
        info!("Stopping GStreamer pipeline");

        self.state.store(false, Ordering::SeqCst);

        self.pipeline
            .set_state(gst::State::Null)
            .map_err(|e| GstError::StateChangeFailed(format!("Failed to stop pipeline: {}", e)))?;

        Ok(())
    }

    /// Pause the pipeline
    pub fn pause(&self) -> Result<(), GstError> {
        self.pipeline
            .set_state(gst::State::Paused)
            .map_err(|e| GstError::StateChangeFailed(format!("Failed to pause pipeline: {}", e)))?;
        Ok(())
    }

    /// Resume the pipeline
    pub fn resume(&self) -> Result<(), GstError> {
        self.pipeline
            .set_state(gst::State::Playing)
            .map_err(|e| GstError::StateChangeFailed(format!("Failed to resume pipeline: {}", e)))?;
        Ok(())
    }

    /// Check if pipeline is running
    pub fn is_running(&self) -> bool {
        self.state.load(Ordering::SeqCst)
    }

    /// Get the current state
    pub fn state(&self) -> PipelineState {
        let (_, current, _) = self.pipeline.state(gst::ClockTime::from_mseconds(0));
        match current {
            gst::State::Null => PipelineState::Stopped,
            gst::State::Ready => PipelineState::Starting,
            gst::State::Paused => PipelineState::Paused,
            gst::State::Playing => PipelineState::Running,
            _ => PipelineState::Error,
        }
    }

    /// Get the appsink for pulling RTP packets
    pub fn appsink(&self) -> &gst_app::AppSink {
        &self.appsink
    }

    /// Pull a sample from the pipeline (blocking with timeout)
    pub fn pull_sample(&self, _timeout_ms: u64) -> Option<gst::Sample> {
        self.appsink.pull_sample().ok()
    }

    /// Pull a sample (non-blocking)
    pub fn try_pull_sample(&self) -> Option<gst::Sample> {
        self.appsink.try_pull_sample(gst::ClockTime::ZERO)
    }

    /// Request a keyframe (IDR)
    pub fn request_keyframe(&self) {
        // Send force-keyunit event
        let event = gst::event::CustomUpstream::builder(
            gst::Structure::builder("GstForceKeyUnit")
                .field("all-headers", true)
                .build()
        ).build();

        if !self.appsink.send_event(event) {
            warn!("Failed to send force-keyunit event");
        } else {
            debug!("Keyframe requested");
        }
    }

    /// Update bitrate dynamically
    pub fn set_bitrate(&self, bitrate_kbps: u32) {
        if let Some(encoder) = self.pipeline.by_name("encoder") {
            // Try setting bitrate property (different encoders use different properties)
            if encoder.has_property("bitrate", None) {
                // x264enc uses kbps
                let _ = encoder.set_property("bitrate", bitrate_kbps);
                debug!("Updated encoder bitrate to {} kbps", bitrate_kbps);
            } else if encoder.has_property("target-bitrate", None) {
                // vp8enc/vp9enc use bps
                let _ = encoder.set_property("target-bitrate", bitrate_kbps * 1000);
                debug!("Updated encoder target-bitrate to {} bps", bitrate_kbps * 1000);
            }
        }
    }

    /// Get frame count
    pub fn frame_count(&self) -> u64 {
        self.frame_count.load(Ordering::Relaxed)
    }

    /// Get pipeline configuration
    pub fn config(&self) -> &PipelineConfig {
        &self.config
    }

    /// Get the name of the encoder being used
    pub fn encoder_name(&self) -> &str {
        &self.encoder_element
    }
}

impl Drop for VideoPipeline {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pipeline_config_default() {
        let config = PipelineConfig::default();
        assert_eq!(config.framerate, 30);
        assert_eq!(config.bitrate, 4000);
        assert_eq!(config.codec, VideoCodec::H264);
    }
}
