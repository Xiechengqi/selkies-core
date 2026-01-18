//! WebSocket message codec
//!
//! Handles encoding/decoding of WebSocket messages.
//! Note: This module is currently unused but retained for future use.

#![allow(dead_code)]

use crate::encode::Stripe;
use base64::Engine;
use log::debug;
use std::fmt;

/// WebSocket frame wrapper
#[derive(Debug, Clone)]
pub struct WebSocketFrame {
    /// Frame type
    pub frame_type: FrameType,
    /// Payload
    pub payload: Vec<u8>,
    /// Is binary
    pub is_binary: bool,
}

/// Frame type enumeration
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameType {
    /// Video stripe
    Stripe,
    /// Cursor update
    Cursor,
    /// Clipboard data
    Clipboard,
    /// Control message
    Control,
    /// Ping/Pong
    Ping,
}

impl fmt::Display for FrameType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FrameType::Stripe => write!(f, "Stripe"),
            FrameType::Cursor => write!(f, "Cursor"),
            FrameType::Clipboard => write!(f, "Clipboard"),
            FrameType::Control => write!(f, "Control"),
            FrameType::Ping => write!(f, "Ping"),
        }
    }
}

/// Message codec for WebSocket communication
pub struct MessageCodec;

impl MessageCodec {
    /// Encode a stripe as WebSocket message
    pub fn encode_stripe(stripe: &Stripe) -> String {
        let encoded = base64::engine::general_purpose::STANDARD.encode(&stripe.data);
        format!("s,{},{},{}", stripe.y, stripe.height, encoded)
    }

    /// Decode a message from the browser
    pub fn decode_message(text: &str) -> Result<DecodedMessage, String> {
        let parts: Vec<&str> = text.split(',').collect();

        if parts.is_empty() {
            return Err("Empty message".to_string());
        }

        match parts[0] {
            "m" => {
                if parts.len() >= 3 {
                    let x: i32 = parts[1].parse().map_err(|_| "Invalid x")?;
                    let y: i32 = parts[2].parse().map_err(|_| "Invalid y")?;
                    Ok(DecodedMessage::MouseMove { x, y })
                } else {
                    Err("Invalid mouse move format".to_string())
                }
            }
            "b" | "M" => {
                if parts.len() >= 3 {
                    let button: u8 = parts[1].parse().map_err(|_| "Invalid button")?;
                    let pressed = parse_bool(parts[2]).map_err(|_| "Invalid pressed")?;
                    Ok(DecodedMessage::MouseButton { button, pressed })
                } else {
                    Err("Invalid mouse button format".to_string())
                }
            }
            "w" | "W" => {
                if parts.len() >= 3 {
                    let dx: i32 = parts[1].parse().map_err(|_| "Invalid delta_x")?;
                    let dy: i32 = parts[2].parse().map_err(|_| "Invalid delta_y")?;
                    let dx = dx.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
                    let dy = dy.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
                    Ok(DecodedMessage::MouseWheel { dx, dy })
                } else {
                    Err("Invalid mouse wheel format".to_string())
                }
            }
            "k" => {
                if parts.len() >= 3 {
                    let keysym: u32 = parts[1].parse().map_err(|_| "Invalid keysym")?;
                    let pressed = parse_bool(parts[2]).map_err(|_| "Invalid pressed")?;
                    Ok(DecodedMessage::Keyboard { keysym, pressed })
                } else {
                    Err("Invalid keyboard format".to_string())
                }
            }
            "c" => {
                if parts.len() >= 2 {
                    Ok(DecodedMessage::Clipboard { data: parts[1].to_string() })
                } else {
                    Err("Invalid clipboard format".to_string())
                }
            }
            _ => {
                debug!("Unknown message: {}", text);
                Err(format!("Unknown message type: {}", parts[0]))
            }
        }
    }
}

/// Decoded message types
#[derive(Debug, Clone)]
pub enum DecodedMessage {
    MouseMove { x: i32, y: i32 },
    MouseButton { button: u8, pressed: bool },
    MouseWheel { dx: i16, dy: i16 },
    Keyboard { keysym: u32, pressed: bool },
    Clipboard { data: String },
}

fn parse_bool(value: &str) -> Result<bool, ()> {
    match value {
        "1" | "true" | "True" => Ok(true),
        "0" | "false" | "False" => Ok(false),
        _ => Err(()),
    }
}

#[cfg(test)]
mod tests {
    use super::{DecodedMessage, MessageCodec};

    #[test]
    fn decode_mouse_move() {
        let msg = MessageCodec::decode_message("m,10,20").expect("decode");
        match msg {
            DecodedMessage::MouseMove { x, y } => {
                assert_eq!(x, 10);
                assert_eq!(y, 20);
            }
            _ => panic!("unexpected message"),
        }
    }

    #[test]
    fn decode_keyboard() {
        let msg = MessageCodec::decode_message("k,65,1").expect("decode");
        match msg {
            DecodedMessage::Keyboard { keysym, pressed } => {
                assert_eq!(keysym, 65);
                assert!(pressed);
            }
            _ => panic!("unexpected message"),
        }
    }

    #[test]
    fn decode_clipboard() {
        let msg = MessageCodec::decode_message("c,SGVsbG8=").expect("decode");
        match msg {
            DecodedMessage::Clipboard { data } => {
                assert_eq!(data, "SGVsbG8=");
            }
            _ => panic!("unexpected message"),
        }
    }
}
