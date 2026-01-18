//! GStreamer encoder selection and configuration
//!
//! Handles automatic detection and selection of hardware/software encoders
//! based on system capabilities.

#![allow(dead_code, unused_imports)]

use super::GstError;
use crate::config::{VideoCodec, HardwareEncoder};
use gstreamer as gst;
use gstreamer::prelude::*;
use log::{info, warn, debug};

/// Encoder availability information
#[derive(Debug, Clone)]
pub struct EncoderInfo {
    pub name: &'static str,
    pub encoder_type: HardwareEncoder,
    pub codec: VideoCodec,
    pub priority: u8,  // Higher = preferred
}

/// Software encoders
const SOFTWARE_ENCODERS: &[EncoderInfo] = &[
    EncoderInfo { name: "x264enc", encoder_type: HardwareEncoder::Software, codec: VideoCodec::H264, priority: 50 },
    EncoderInfo { name: "openh264enc", encoder_type: HardwareEncoder::Software, codec: VideoCodec::H264, priority: 40 },
    EncoderInfo { name: "vp8enc", encoder_type: HardwareEncoder::Software, codec: VideoCodec::VP8, priority: 50 },
    EncoderInfo { name: "vp9enc", encoder_type: HardwareEncoder::Software, codec: VideoCodec::VP9, priority: 50 },
    EncoderInfo { name: "av1enc", encoder_type: HardwareEncoder::Software, codec: VideoCodec::AV1, priority: 50 },
    EncoderInfo { name: "rav1enc", encoder_type: HardwareEncoder::Software, codec: VideoCodec::AV1, priority: 45 },
];

/// VA-API hardware encoders (Intel, AMD)
const VAAPI_ENCODERS: &[EncoderInfo] = &[
    EncoderInfo { name: "vaapih264enc", encoder_type: HardwareEncoder::Vaapi, codec: VideoCodec::H264, priority: 90 },
    EncoderInfo { name: "vaapivp8enc", encoder_type: HardwareEncoder::Vaapi, codec: VideoCodec::VP8, priority: 90 },
    EncoderInfo { name: "vaapivp9enc", encoder_type: HardwareEncoder::Vaapi, codec: VideoCodec::VP9, priority: 90 },
    EncoderInfo { name: "vaapiav1enc", encoder_type: HardwareEncoder::Vaapi, codec: VideoCodec::AV1, priority: 90 },
];

/// NVIDIA NVENC encoders
const NVENC_ENCODERS: &[EncoderInfo] = &[
    EncoderInfo { name: "nvh264enc", encoder_type: HardwareEncoder::Nvenc, codec: VideoCodec::H264, priority: 95 },
    EncoderInfo { name: "nvv4l2h264enc", encoder_type: HardwareEncoder::Nvenc, codec: VideoCodec::H264, priority: 85 },
];

/// Intel Quick Sync encoders
const QSV_ENCODERS: &[EncoderInfo] = &[
    EncoderInfo { name: "qsvh264enc", encoder_type: HardwareEncoder::Qsv, codec: VideoCodec::H264, priority: 92 },
    EncoderInfo { name: "qsvvp9enc", encoder_type: HardwareEncoder::Qsv, codec: VideoCodec::VP9, priority: 92 },
    EncoderInfo { name: "qsvav1enc", encoder_type: HardwareEncoder::Qsv, codec: VideoCodec::AV1, priority: 92 },
];

/// Check if a GStreamer element is available
fn element_available(name: &str) -> bool {
    gst::ElementFactory::find(name).is_some()
}

/// Detect available hardware encoders
pub fn detect_hardware_encoder(codec: VideoCodec) -> Vec<EncoderInfo> {
    let mut available = Vec::new();

    // Check NVENC
    for encoder in NVENC_ENCODERS {
        if encoder.codec == codec && element_available(encoder.name) {
            debug!("Found NVENC encoder: {}", encoder.name);
            available.push(encoder.clone());
        }
    }

    // Check QSV
    for encoder in QSV_ENCODERS {
        if encoder.codec == codec && element_available(encoder.name) {
            debug!("Found QSV encoder: {}", encoder.name);
            available.push(encoder.clone());
        }
    }

    // Check VA-API
    for encoder in VAAPI_ENCODERS {
        if encoder.codec == codec && element_available(encoder.name) {
            debug!("Found VA-API encoder: {}", encoder.name);
            available.push(encoder.clone());
        }
    }

    // Check software encoders
    for encoder in SOFTWARE_ENCODERS {
        if encoder.codec == codec && element_available(encoder.name) {
            debug!("Found software encoder: {}", encoder.name);
            available.push(encoder.clone());
        }
    }

    // Sort by priority (highest first)
    available.sort_by(|a, b| b.priority.cmp(&a.priority));

    available
}

/// Encoder selection result
pub struct EncoderSelection {
    pub info: EncoderInfo,
}

impl EncoderSelection {
    /// Select the best encoder for the given codec and hardware preference
    pub fn select(codec: VideoCodec, hw_pref: HardwareEncoder) -> Self {
        let available = detect_hardware_encoder(codec);

        if available.is_empty() {
            // Fallback to x264enc for H264, vp8enc for VP8
            let fallback_name = match codec {
                VideoCodec::H264 => "x264enc",
                VideoCodec::VP8 => "vp8enc",
                VideoCodec::VP9 => "vp9enc",
                VideoCodec::AV1 => "av1enc",
            };

            warn!("No encoder found for {:?}, will try {}", codec, fallback_name);

            return Self {
                info: EncoderInfo {
                    name: fallback_name,
                    encoder_type: HardwareEncoder::Software,
                    codec,
                    priority: 0,
                },
            };
        }

        // If specific hardware preference is requested, try to find it
        if hw_pref != HardwareEncoder::Auto {
            for encoder in &available {
                if encoder.encoder_type == hw_pref {
                    info!("Selected requested encoder: {} ({:?})", encoder.name, hw_pref);
                    return Self { info: encoder.clone() };
                }
            }
            warn!("Requested encoder type {:?} not available, using best alternative", hw_pref);
        }

        // Use the best available (highest priority)
        let best = available.into_iter().next().unwrap();
        info!("Selected encoder: {} (type: {:?}, priority: {})", best.name, best.encoder_type, best.priority);

        Self { info: best }
    }

    /// Create the GStreamer encoder element with appropriate settings
    pub fn create_encoder(&self, bitrate_kbps: u32, keyframe_interval: u32) -> Result<(gst::Element, String), GstError> {
        let encoder = match self.info.name {
            // Software H.264 (x264)
            "x264enc" => {
                gst::ElementFactory::make("x264enc")
                    .name("encoder")
                    .property_from_str("tune", "zerolatency")
                    .property_from_str("speed-preset", "superfast")
                    .property("bitrate", bitrate_kbps)
                    .property("key-int-max", keyframe_interval)
                    .property("threads", 4u32)
                    .property("b-adapt", false)
                    .property("bframes", 0u32)
                    .property("sliced-threads", true)
                    .build()
            }

            // OpenH264
            "openh264enc" => {
                gst::ElementFactory::make("openh264enc")
                    .name("encoder")
                    .property("bitrate", bitrate_kbps * 1000)  // bps
                    .property("gop-size", keyframe_interval)
                    .build()
            }

            // VP8 software
            "vp8enc" => {
                gst::ElementFactory::make("vp8enc")
                    .name("encoder")
                    .property("target-bitrate", bitrate_kbps * 1000)
                    .property("keyframe-max-dist", keyframe_interval as i32)
                    .property("deadline", 1i64)  // Realtime
                    .property("cpu-used", 8i32)  // Faster encoding
                    .property("threads", 4i32)
                    .build()
            }

            // VP9 software
            "vp9enc" => {
                gst::ElementFactory::make("vp9enc")
                    .name("encoder")
                    .property("target-bitrate", bitrate_kbps * 1000)
                    .property("keyframe-max-dist", keyframe_interval as i32)
                    .property("deadline", 1i64)
                    .property("cpu-used", 8i32)
                    .property("threads", 4i32)
                    .build()
            }

            // AV1 software
            "av1enc" | "rav1enc" => {
                gst::ElementFactory::make(self.info.name)
                    .name("encoder")
                    .property("target-bitrate", bitrate_kbps)
                    .build()
            }

            // VA-API H.264
            "vaapih264enc" => {
                gst::ElementFactory::make("vaapih264enc")
                    .name("encoder")
                    .property("bitrate", bitrate_kbps)
                    .property("keyframe-period", keyframe_interval)
                    .property("rate-control", 2u32)  // VBR
                    .property("tune", 3u32)  // Low-latency
                    .build()
            }

            // VA-API VP8/VP9
            "vaapivp8enc" | "vaapivp9enc" => {
                gst::ElementFactory::make(self.info.name)
                    .name("encoder")
                    .property("bitrate", bitrate_kbps)
                    .property("keyframe-period", keyframe_interval)
                    .build()
            }

            // VA-API AV1
            "vaapiav1enc" => {
                gst::ElementFactory::make("vaapiav1enc")
                    .name("encoder")
                    .property("bitrate", bitrate_kbps)
                    .build()
            }

            // NVIDIA NVENC H.264
            "nvh264enc" => {
                gst::ElementFactory::make("nvh264enc")
                    .name("encoder")
                    .property("bitrate", bitrate_kbps)
                    .property("gop-size", keyframe_interval as i32)
                    .property_from_str("preset", "low-latency-hq")
                    .property("zerolatency", true)
                    .property("rc-mode", 2i32)  // VBR
                    .build()
            }

            // NVIDIA V4L2 H.264
            "nvv4l2h264enc" => {
                gst::ElementFactory::make("nvv4l2h264enc")
                    .name("encoder")
                    .property("bitrate", bitrate_kbps * 1000)
                    .property("iframeinterval", keyframe_interval)
                    .build()
            }

            // Intel QSV H.264
            "qsvh264enc" => {
                gst::ElementFactory::make("qsvh264enc")
                    .name("encoder")
                    .property("bitrate", bitrate_kbps)
                    .property("gop-size", keyframe_interval)
                    .property("low-latency", true)
                    .build()
            }

            // Intel QSV VP9/AV1
            "qsvvp9enc" | "qsvav1enc" => {
                gst::ElementFactory::make(self.info.name)
                    .name("encoder")
                    .property("bitrate", bitrate_kbps)
                    .property("gop-size", keyframe_interval)
                    .build()
            }

            _ => {
                return Err(GstError::EncoderNotFound(format!("Unknown encoder: {}", self.info.name)));
            }
        };

        let encoder = encoder.map_err(|e| {
            GstError::EncoderNotFound(format!("Failed to create encoder '{}': {}", self.info.name, e))
        })?;

        Ok((encoder, self.info.name.to_string()))
    }
}

/// Get a list of all available encoders for diagnostics
pub fn list_available_encoders() -> Vec<(String, VideoCodec, HardwareEncoder)> {
    let mut result = Vec::new();

    for codec in [VideoCodec::H264, VideoCodec::VP8, VideoCodec::VP9, VideoCodec::AV1] {
        for encoder in detect_hardware_encoder(codec) {
            result.push((encoder.name.to_string(), encoder.codec, encoder.encoder_type));
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encoder_selection_fallback() {
        // This test may fail if GStreamer is not initialized
        if gst::init().is_err() {
            return;
        }

        let selection = EncoderSelection::select(VideoCodec::H264, HardwareEncoder::Auto);
        // Should at least fall back to x264enc or similar
        assert!(!selection.info.name.is_empty());
    }
}
