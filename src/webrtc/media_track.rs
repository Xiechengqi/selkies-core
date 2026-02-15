//! RTP packet utilities for WebRTC media
//!
//! Provides RTP header parsing helpers used by the str0m RTP mode
//! integration to extract fields from GStreamer-produced RTP packets.

#![allow(dead_code)]

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
