//! JPEG encoding using turbojpeg
//!
//! Provides stripe-based JPEG encoding with change detection.

use crate::capture::Frame;
use std::sync::{Arc, Mutex};
use turbojpeg::{Compressor, PixelFormat, Subsamp};
use xxhash_rust::xxh64::xxh64;

/// Represents an encoded stripe
#[derive(Debug, Clone)]
pub struct Stripe {
    /// Frame id (mod 65536)
    pub frame_id: u16,
    /// Y position of this stripe
    pub y: u32,
    /// Height of this stripe in pixels
    #[allow(dead_code)]
    pub height: u32,

    /// Compressed JPEG data
    pub data: Vec<u8>,
}

/// JPEG Encoder configuration
#[derive(Debug, Clone)]
pub struct EncoderConfig {
    /// JPEG quality (1-100)
    pub quality: u8,

    /// Stripe height in pixels
    pub stripe_height: u32,

    /// Subsampling mode (0 = 444, 1 = 422, 2 = 420)
    pub subsample: u8,
}

impl Default for EncoderConfig {
    fn default() -> Self {
        Self {
            quality: 75,
            stripe_height: 64,
            subsample: 1, // 422 for better compression
        }
    }
}

/// JPEG encoder using turbojpeg
pub struct Encoder {
    /// JPEG compressor
    compressor: Compressor,

    /// Configuration
    config: EncoderConfig,

    /// Previous frame hash for change detection
    prev_frame_hash: Arc<Mutex<Vec<u64>>>,

    /// Stripe count
    stripe_count: usize,
}

impl Encoder {
    /// Create a new encoder
    pub fn new(config: EncoderConfig) -> Result<Self, Box<dyn std::error::Error>> {
        let compressor = Compressor::new()?;
        let stripe_count = (4096 / config.stripe_height as usize) + 1; // Max 4096 height

        Ok(Self {
            compressor,
            config,
            prev_frame_hash: Arc::new(Mutex::new(vec![0; stripe_count])),
            stripe_count,
        })
    }

    /// Encode a frame into stripes
    pub fn encode_frame(&mut self, frame: &Frame) -> Result<Vec<Stripe>, Box<dyn std::error::Error>> {
        let mut stripes = Vec::new();
        let stripe_height = self.config.stripe_height;
        let frame_hash = self.hash_frame(frame);

        // Calculate stripe hashes from previous frame
        let prev_hashes = self.prev_frame_hash.lock().unwrap().clone();

        let all_prev_zero = prev_hashes.iter().all(|&h| h == 0);
        if frame.sequence == 0 || all_prev_zero {
            log::info!("encode_frame: frame {}x{}, seq={}, frame_hash len={}, prev_hashes len={}, all_prev_zero={}",
                   frame.width, frame.height, frame.sequence, frame_hash.len(), prev_hashes.len(), all_prev_zero);
        }

        // Iterate over actual frame stripes, not prev_hashes
        for (i, &current_hash) in frame_hash.iter().enumerate() {
            let y = (i * stripe_height as usize) as u32;

            // Skip if past frame height
            if y >= frame.height {
                break;
            }

            let prev_hash = prev_hashes.get(i).copied().unwrap_or(0);
            let is_changed = current_hash != prev_hash;

            // Determine if this should be a keyframe (every Nth stripe or first)
            let should_send = i == 0 || i % 30 == 0 || is_changed;

            if all_prev_zero && i < 3 {
                log::info!("  stripe {}: current_hash={}, prev_hash={}, is_changed={}, should_send={}",
                    i, current_hash, prev_hash, is_changed, should_send);
            }

            if !should_send {
                continue; // Skip unchanged stripe
            }

            // Get stripe data
            let actual_height = stripe_height.min(frame.height - y);
            let stripe_data = self.extract_stripe(frame, y, actual_height);

            // Encode the stripe
            let compressed = self.encode_stripe(
                &stripe_data,
                frame.width as usize,
                actual_height as usize,
            )?;

            stripes.push(Stripe {
                frame_id: 0,
                y,
                height: actual_height,
                data: compressed,
            });
        }

        // Update previous frame hash
        *self.prev_frame_hash.lock().unwrap() = frame_hash;

        if frame.sequence == 0 || all_prev_zero {
            log::info!("encode_frame DONE: generated {} stripes (seq={}, all_prev_zero={})",
                stripes.len(), frame.sequence, all_prev_zero);
        }

        Ok(stripes)
    }

    /// Force next frame to refresh all stripes
    pub fn force_refresh(&mut self) {
        let mut hashes = self.prev_frame_hash.lock().unwrap();
        *hashes = vec![0; self.stripe_count];
        log::info!("Encoder: force_refresh called, reset {} stripe hashes to 0", self.stripe_count);
    }

    /// Extract a stripe from the frame
    fn extract_stripe(&self, frame: &Frame, y: u32, height: u32) -> Vec<u8> {
        let actual_height = height.min(frame.height - y);
        let row_size = frame.width * 3;
        let offset = (y * row_size) as usize;

        let mut stripe = Vec::with_capacity((row_size * actual_height) as usize);

        for row in 0..actual_height {
            let start = offset + (row * row_size) as usize;
            let end = start + row_size as usize;
            stripe.extend_from_slice(&frame.data[start..end]);
        }

        stripe
    }

    /// Encode stripe data as JPEG
    fn encode_stripe(
        &mut self,
        data: &[u8],
        width: usize,
        height: usize,
    ) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        // Prepare image buffer
        let image = turbojpeg::Image {
            pixels: data,
            width,
            pitch: width * 3,
            height,
            format: PixelFormat::RGB,
        };

        // Set quality
        self.compressor.set_quality(self.config.quality as i32);
        self.compressor.set_subsamp(match self.config.subsample {
            0 => Subsamp::None,
            1 => Subsamp::Sub2x1,
            2 => Subsamp::Sub2x2,
            _ => Subsamp::Sub2x2,
        });

        // Encode
        let compressed = self.compressor.compress_to_vec(image)?;
        Ok(compressed)
    }

    /// Hash frame by stripes using xxhash
    fn hash_frame(&self, frame: &Frame) -> Vec<u64> {
        let stripe_height = self.config.stripe_height;
        let row_size = frame.width * 3;
        let mut hashes = Vec::new();

        let mut y = 0u32;
        while y < frame.height {
            let height = stripe_height.min(frame.height - y);
            let offset = (y * row_size) as usize;
            let size = (row_size * height) as usize;

            if offset + size <= frame.data.len() {
                let data = &frame.data[offset..offset + size];
                let hash = xxh64(data, 0);
                hashes.push(hash);
            } else {
                hashes.push(0);
            }

            y += stripe_height;
        }

        hashes
    }

}
