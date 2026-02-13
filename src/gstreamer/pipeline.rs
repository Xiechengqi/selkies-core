//! GStreamer video encoding pipeline
//!
//! Provides a complete pipeline for:
//! - Receiving raw frames from compositor via appsrc
//! - Video encoding (H.264, VP8, VP9)

#![allow(dead_code)]
//! - RTP packetization for WebRTC

use super::{GstError, encoder::EncoderSelection};
use crate::config::{VideoCodec, HardwareEncoder, WebRTCConfig};
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use gstreamer_video as gst_video;
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
    /// Frame width
    pub width: u32,
    /// Frame height
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
}

impl From<&WebRTCConfig> for PipelineConfig {
    fn from(config: &WebRTCConfig) -> Self {
        Self {
            width: 1920,
            height: 1080,
            framerate: 30,
            codec: config.video_codec,
            bitrate: config.video_bitrate,
            hardware_encoder: config.hardware_encoder,
            keyframe_interval: config.keyframe_interval,
            latency_ms: config.pipeline_latency_ms,
        }
    }
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            width: 1920,
            height: 1080,
            framerate: 30,
            codec: VideoCodec::H264,
            bitrate: 4000,
            hardware_encoder: HardwareEncoder::Auto,
            keyframe_interval: 60,
            latency_ms: 50,
        }
    }
}

/// RTP packet callback type
pub type RtpCallback = Box<dyn Fn(&[u8], u32, u64) + Send + Sync>;

/// Video pipeline for GStreamer-based encoding
pub struct VideoPipeline {
    pipeline: gst::Pipeline,
    appsrc: gst_app::AppSrc,
    appsink: gst_app::AppSink,
    config: PipelineConfig,
    state: Arc<AtomicBool>,
    frame_count: Arc<AtomicU64>,
    encoder_element: String,
}

impl VideoPipeline {
    /// Create a new video pipeline with appsrc for compositor frame input
    pub fn new(config: PipelineConfig) -> Result<Self, GstError> {
        gst::init().map_err(|e| GstError::InitFailed(e.to_string()))?;

        let pipeline = gst::Pipeline::new();

        // Create appsrc for receiving raw frames from compositor
        let caps_str = format!(
            "video/x-raw,format=BGRx,width={},height={},framerate={}/1",
            config.width, config.height, config.framerate
        );
        let caps = caps_str.parse::<gst::Caps>()
            .map_err(|e| GstError::PipelineFailed(format!("Invalid caps: {}", e)))?;

        let appsrc = gst_app::AppSrc::builder()
            .name("framesrc")
            .caps(&caps)
            .format(gst::Format::Time)
            .is_live(true)
            .do_timestamp(true)
            .build();

        // videoconvert: BGRx -> I420 for encoder
        let convert = gst::ElementFactory::make("videoconvert")
            .build()
            .map_err(|e| GstError::PipelineFailed(format!("Failed to create videoconvert: {}", e)))?;

        let encoder_selection = EncoderSelection::select(config.codec, config.hardware_encoder);
        let (encoder, encoder_name) = encoder_selection.create_encoder(
            config.bitrate, config.keyframe_interval,
        )?;
        info!("Using encoder: {} for codec {:?}", encoder_name, config.codec);

        let payloader = Self::create_payloader(config.codec)?;

        let appsink = gst_app::AppSink::builder()
            .name("rtpsink")
            .sync(false)
            .max_buffers(0)
            .drop(false)
            .build();

        pipeline.add_many([
            appsrc.upcast_ref(),
            &convert,
            &encoder,
            &payloader,
            appsink.upcast_ref(),
        ]).map_err(|e| GstError::PipelineFailed(format!("Failed to add elements: {}", e)))?;

        // Link: appsrc -> convert -> encoder -> payloader -> appsink
        appsrc.upcast_ref::<gst::Element>().link(&convert)
            .map_err(|e| GstError::LinkFailed(format!("appsrc->convert: {}", e)))?;
        convert.link(&encoder)
            .map_err(|e| GstError::LinkFailed(format!("convert->encoder: {}", e)))?;
        encoder.link(&payloader)
            .map_err(|e| GstError::LinkFailed(format!("encoder->payloader: {}", e)))?;
        payloader.link(appsink.upcast_ref::<gst::Element>())
            .map_err(|e| GstError::LinkFailed(format!("payloader->appsink: {}", e)))?;

        pipeline.set_latency(gst::ClockTime::from_mseconds(config.latency_ms as u64));

        Ok(Self {
            pipeline,
            appsrc,
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

        let mut builder = gst::ElementFactory::make(element_name)
            .property("pt", pt as u32);

        // For H264, ensure SPS/PPS are sent regularly for browser decoders.
        if matches!(codec, VideoCodec::H264) {
            builder = builder.property("config-interval", 1i32);
        }

        // Note: aggregate-mode requires enum type, skip for now
        builder.build()
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

    /// Push a raw frame (XRGB8888 / BGRx) into the pipeline via appsrc
    pub fn push_frame(&self, data: &[u8]) -> Result<(), GstError> {
        let mut buffer = gst::Buffer::with_size(data.len())
            .map_err(|e| GstError::PipelineFailed(format!("Buffer alloc failed: {}", e)))?;
        {
            let buffer_ref = buffer.get_mut().unwrap();
            let mut map = buffer_ref.map_writable()
                .map_err(|e| GstError::PipelineFailed(format!("Buffer map failed: {}", e)))?;
            map.copy_from_slice(data);
        }
        self.appsrc.push_buffer(buffer)
            .map_err(|e| GstError::PipelineFailed(format!("appsrc push failed: {:?}", e)))?;
        self.frame_count.fetch_add(1, Ordering::Relaxed);
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

    /// Pull a sample with timeout (blocks up to timeout_ms)
    pub fn try_pull_sample_timeout(&self, timeout_ms: u64) -> Option<gst::Sample> {
        self.appsink.try_pull_sample(gst::ClockTime::from_mseconds(timeout_ms))
    }

    /// Request a keyframe (IDR)
    pub fn request_keyframe(&self) {
        if let Some(encoder) = self.pipeline.by_name("encoder") {
            // send_event() on an element sends upstream events upstream through sink pads
            let event = gst_video::UpstreamForceKeyUnitEvent::builder()
                .all_headers(true)
                .build();
            if !encoder.send_event(event) {
                warn!("Failed to send force-keyunit event to encoder");
            } else {
                info!("Sent force-keyunit event to encoder for IDR frame");
            }
        } else {
            warn!("No encoder element found for keyframe request");
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

    /// Update keyframe interval dynamically (best-effort)
    pub fn set_keyframe_interval(&self, interval: u32) {
        if let Some(encoder) = self.pipeline.by_name("encoder") {
            if encoder.has_property("key-int-max", None) {
                let _ = encoder.set_property("key-int-max", interval as i32);
            } else if encoder.has_property("gop-size", None) {
                let _ = encoder.set_property("gop-size", interval);
            } else if encoder.has_property("keyframe-max-dist", None) {
                let _ = encoder.set_property("keyframe-max-dist", interval as i32);
            } else if encoder.has_property("keyframe-period", None) {
                let _ = encoder.set_property("keyframe-period", interval as i32);
            } else if encoder.has_property("iframeinterval", None) {
                let _ = encoder.set_property("iframeinterval", interval as i32);
            }
            debug!("Updated encoder keyframe interval to {} frames", interval);
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
