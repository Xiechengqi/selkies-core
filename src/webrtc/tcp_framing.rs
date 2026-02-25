//! RFC 4571 TCP framing for WebRTC media over TCP
//!
//! Each packet is prefixed with a 2-byte big-endian length header.

/// Encode a packet with RFC 4571 framing (2-byte length prefix).
///
/// Panics if `data` exceeds 65535 bytes (u16 max). WebRTC packets
/// are well under this limit in practice (MTU ~1200-1400 bytes).
pub fn frame_packet(data: &[u8]) -> Vec<u8> {
    assert!(data.len() <= u16::MAX as usize, "RFC 4571 frame too large: {} bytes", data.len());
    let len = data.len() as u16;
    let mut framed = Vec::with_capacity(2 + data.len());
    framed.extend_from_slice(&len.to_be_bytes());
    framed.extend_from_slice(data);
    framed
}

/// Maximum allowed RFC 4571 frame size (bytes).
/// DTLS records can be up to ~16KB, and SCTP messages carrying large
/// DataChannel payloads may exceed 4KB.  Use the full u16 range to
/// avoid spurious FrameTooLarge disconnects.
pub const MAX_RFC4571_FRAME: usize = 65535;

#[derive(Debug)]
pub enum TcpFrameError {
    FrameTooLarge(#[allow(dead_code)] usize),
    ZeroLength,
}

/// Stateful decoder for RFC 4571 framed TCP streams.
///
/// Handles partial reads across TCP segment boundaries.
pub struct TcpFrameDecoder {
    buf: Vec<u8>,
}

impl TcpFrameDecoder {
    pub fn new() -> Self {
        Self { buf: Vec::with_capacity(4096) }
    }

    /// Append received bytes to the internal buffer
    pub fn extend(&mut self, data: &[u8]) {
        self.buf.extend_from_slice(data);
    }

    /// Extract the next complete packet, if available
    pub fn next_packet(&mut self) -> Result<Option<Vec<u8>>, TcpFrameError> {
        if self.buf.len() < 2 {
            return Ok(None);
        }
        let length = u16::from_be_bytes([self.buf[0], self.buf[1]]) as usize;
        if length == 0 {
            return Err(TcpFrameError::ZeroLength);
        }
        if length > MAX_RFC4571_FRAME {
            return Err(TcpFrameError::FrameTooLarge(length));
        }
        let total = 2 + length;
        if self.buf.len() < total {
            return Ok(None);
        }
        let pkt = self.buf[2..total].to_vec();
        self.buf.drain(..total);
        Ok(Some(pkt))
    }

    pub fn take_remaining(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.buf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_frame_roundtrip() {
        let data = b"hello world";
        let framed = frame_packet(data);
        assert_eq!(framed.len(), 2 + data.len());
        assert_eq!(&framed[0..2], &(data.len() as u16).to_be_bytes());

        let mut decoder = TcpFrameDecoder::new();
        decoder.extend(&framed);
        let decoded = decoder.next_packet().unwrap().unwrap();
        assert_eq!(decoded, data);
        assert!(decoder.next_packet().unwrap().is_none());
    }

    #[test]
    fn test_partial_reads() {
        let data = b"test packet";
        let framed = frame_packet(data);

        let mut decoder = TcpFrameDecoder::new();
        // Feed one byte at a time
        for &byte in &framed {
            decoder.extend(&[byte]);
        }
        let decoded = decoder.next_packet().unwrap().unwrap();
        assert_eq!(decoded, data);
    }

    #[test]
    fn test_multiple_packets() {
        let mut decoder = TcpFrameDecoder::new();
        let p1 = frame_packet(b"first");
        let p2 = frame_packet(b"second");
        let mut combined = p1;
        combined.extend_from_slice(&p2);
        decoder.extend(&combined);

        assert_eq!(decoder.next_packet().unwrap().unwrap(), b"first");
        assert_eq!(decoder.next_packet().unwrap().unwrap(), b"second");
        assert!(decoder.next_packet().unwrap().is_none());
    }

    #[test]
    fn test_take_remaining_clears_buffer() {
        let mut decoder = TcpFrameDecoder::new();
        decoder.extend(&[0x00, 0x05, b'h', b'e']);
        let remaining = decoder.take_remaining();
        assert_eq!(remaining, vec![0x00, 0x05, b'h', b'e']);
        assert!(decoder.next_packet().unwrap().is_none());
    }
}
