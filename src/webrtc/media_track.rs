//! WebRTC Media Track handling
//!
//! Provides RTP packet writing to WebRTC video tracks.

#![allow(dead_code)]

use super::WebRTCError;
use crate::config::VideoCodec;
use log::debug;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use webrtc::track::track_local::track_local_static_rtp::TrackLocalStaticRTP;
use webrtc::track::track_local::{TrackLocal, TrackLocalWriter};
use bytes::Bytes;

/// RTP packet statistics
#[derive(Debug, Default)]
pub struct RtpStats {
    /// Total packets sent
    pub packets_sent: AtomicU64,
    /// Total bytes sent
    pub bytes_sent: AtomicU64,
    /// Packets dropped
    pub packets_dropped: AtomicU64,
    /// Current sequence number
    pub sequence: AtomicU32,
    /// Current timestamp
    pub timestamp: AtomicU32,
}

impl RtpStats {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record_sent(&self, bytes: usize) {
        self.packets_sent.fetch_add(1, Ordering::Relaxed);
        self.bytes_sent.fetch_add(bytes as u64, Ordering::Relaxed);
    }

    pub fn record_dropped(&self) {
        self.packets_dropped.fetch_add(1, Ordering::Relaxed);
    }

    pub fn snapshot(&self) -> (u64, u64, u64) {
        (
            self.packets_sent.load(Ordering::Relaxed),
            self.bytes_sent.load(Ordering::Relaxed),
            self.packets_dropped.load(Ordering::Relaxed),
        )
    }
}

/// Video track writer for sending RTP packets
pub struct VideoTrackWriter {
    track: Arc<TrackLocalStaticRTP>,
    codec: VideoCodec,
    stats: Arc<RtpStats>,
    clock_rate: u32,
}

impl VideoTrackWriter {
    /// Create a new video track writer
    pub fn new(track: Arc<TrackLocalStaticRTP>, codec: VideoCodec) -> Self {
        Self {
            track,
            codec,
            stats: Arc::new(RtpStats::new()),
            clock_rate: 90000,  // Standard video clock rate
        }
    }

    /// Write an RTP packet to the track
    pub async fn write_rtp(&self, packet: &[u8]) -> Result<(), WebRTCError> {
        let bytes = Bytes::copy_from_slice(packet);

        match self.track.write(&bytes).await {
            Ok(n) => {
                self.stats.record_sent(n);
                Ok(())
            }
            Err(e) => {
                self.stats.record_dropped();
                // Don't log every dropped packet, just debug level
                debug!("RTP write error: {}", e);
                Err(WebRTCError::MediaError(format!("RTP write failed: {}", e)))
            }
        }
    }

    /// Write raw sample data (will be packetized internally)
    pub async fn write_sample(&self, data: &[u8], timestamp: u32, marker: bool) -> Result<(), WebRTCError> {
        // For TrackLocalStaticRTP, we need to send pre-formed RTP packets
        // This method is here for future use with different track types
        let _ = (data, timestamp, marker);
        Err(WebRTCError::MediaError("Use write_rtp for raw RTP packets".to_string()))
    }

    /// Get the video codec
    pub fn codec(&self) -> VideoCodec {
        self.codec
    }

    /// Get statistics
    pub fn stats(&self) -> Arc<RtpStats> {
        self.stats.clone()
    }

    /// Get the underlying track
    pub fn track(&self) -> Arc<TrackLocalStaticRTP> {
        self.track.clone()
    }

    /// Get track ID
    pub fn id(&self) -> String {
        self.track.id().to_string()
    }

    /// Get stream ID
    pub fn stream_id(&self) -> String {
        self.track.stream_id().to_string()
    }
}

/// RTP packet parser utilities
pub mod rtp_util {
    /// Extract sequence number from RTP packet
    pub fn get_sequence(packet: &[u8]) -> Option<u16> {
        if packet.len() < 4 {
            return None;
        }
        Some(u16::from_be_bytes([packet[2], packet[3]]))
    }

    /// Extract timestamp from RTP packet
    pub fn get_timestamp(packet: &[u8]) -> Option<u32> {
        if packet.len() < 8 {
            return None;
        }
        Some(u32::from_be_bytes([packet[4], packet[5], packet[6], packet[7]]))
    }

    /// Extract SSRC from RTP packet
    pub fn get_ssrc(packet: &[u8]) -> Option<u32> {
        if packet.len() < 12 {
            return None;
        }
        Some(u32::from_be_bytes([packet[8], packet[9], packet[10], packet[11]]))
    }

    /// Extract payload type from RTP packet
    pub fn get_payload_type(packet: &[u8]) -> Option<u8> {
        if packet.len() < 2 {
            return None;
        }
        Some(packet[1] & 0x7F)
    }

    /// Check if marker bit is set
    pub fn is_marker_set(packet: &[u8]) -> bool {
        if packet.len() < 2 {
            return false;
        }
        (packet[1] & 0x80) != 0
    }

    /// Get RTP header length (including extensions)
    pub fn header_length(packet: &[u8]) -> Option<usize> {
        if packet.len() < 12 {
            return None;
        }

        let cc = (packet[0] & 0x0F) as usize;
        let mut len = 12 + cc * 4;

        // Check extension bit
        if (packet[0] & 0x10) != 0 {
            if packet.len() < len + 4 {
                return None;
            }
            let ext_len = u16::from_be_bytes([packet[len + 2], packet[len + 3]]) as usize;
            len += 4 + ext_len * 4;
        }

        Some(len)
    }

    /// Get payload data
    pub fn get_payload(packet: &[u8]) -> Option<&[u8]> {
        let header_len = header_length(packet)?;
        if packet.len() > header_len {
            Some(&packet[header_len..])
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::rtp_util::*;

    #[test]
    fn test_rtp_parsing() {
        // Minimal RTP packet: V=2, P=0, X=0, CC=0, M=1, PT=96
        let packet = [
            0x80, 0xE0,  // V=2, P=0, X=0, CC=0, M=1, PT=96
            0x00, 0x01,  // Sequence number = 1
            0x00, 0x00, 0x00, 0x00,  // Timestamp = 0
            0x12, 0x34, 0x56, 0x78,  // SSRC
            0x00, 0x01, 0x02,  // Payload
        ];

        assert_eq!(get_sequence(&packet), Some(1));
        assert_eq!(get_timestamp(&packet), Some(0));
        assert_eq!(get_ssrc(&packet), Some(0x12345678));
        assert_eq!(get_payload_type(&packet), Some(96));
        assert!(is_marker_set(&packet));
        assert_eq!(header_length(&packet), Some(12));
        assert_eq!(get_payload(&packet), Some(&[0x00, 0x01, 0x02][..]));
    }
}
