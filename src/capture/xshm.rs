//! X11 Screen capture
//!
//! Provides screen capture using X11 XImage.

use crate::capture::frame::{Frame, FrameStats};
use log::debug;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use x11rb::connection::Connection;
use x11rb::protocol::shm;
use x11rb::protocol::xproto::*;
use x11rb::xcb_ffi::XCBConnection;

/// Screen capturer using X11
pub struct X11Capturer {
    /// XCB connection
    conn: Arc<XCBConnection>,

    /// Root window
    root: Window,

    /// Screen width
    width: u32,

    /// Screen height
    height: u32,

    /// Depth
    depth: u8,

    /// Byte order (0 = LSB, 1 = MSB)
    byte_order: u8,

    /// Bytes per pixel
    bytes_per_pixel: usize,

    /// Bytes per line (stride)
    bytes_per_line: usize,

    /// Use XShm capture path
    use_shm: bool,

    /// Shared memory segment id
    shmseg: u32,

    /// Shared memory segment handle
    shmid: i32,

    /// Shared memory address
    shmaddr: *mut u8,

    /// Shared memory size
    shm_size: usize,

    /// Frame sequence counter
    sequence: Arc<AtomicU64>,

    /// Frame stats
    stats: Arc<Mutex<FrameStats>>,
}

/// Byte order constants
const BYTE_ORDER_LSB_FIRST: u8 = 0;

impl X11Capturer {
    /// Create a new X11 capturer
    pub fn new(
        conn: Arc<XCBConnection>,
        screen_num: i32,
    ) -> Result<Self, x11rb::errors::ConnectionError> {
        let screen = &conn.setup().roots[screen_num as usize];
        let root = screen.root;
        let width = screen.width_in_pixels as u32;
        let height = screen.height_in_pixels as u32;
        let depth = screen.root_depth;
        let byte_order = u8::from(conn.setup().image_byte_order);
        let (bytes_per_pixel, bytes_per_line) =
            compute_format(conn.as_ref(), width, depth);

        let mut capturer = Self {
            conn,
            root,
            width,
            height,
            depth,
            byte_order,
            bytes_per_pixel,
            bytes_per_line,
            use_shm: false,
            shmseg: 0,
            shmid: -1,
            shmaddr: std::ptr::null_mut(),
            shm_size: 0,
            sequence: Arc::new(AtomicU64::new(0)),
            stats: Arc::new(Mutex::new(FrameStats::default())),
        };

        capturer.try_init_shm();
        Ok(capturer)
    }

    fn try_init_shm(&mut self) {
        let shm_query = shm::query_version(self.conn.as_ref());
        if shm_query.is_err() {
            debug!("XShm not available, using XGetImage");
            return;
        }
        if shm_query.unwrap().reply().is_err() {
            debug!("XShm not available, using XGetImage");
            return;
        }

        let shmseg = match self.conn.generate_id() {
            Ok(id) => id,
            Err(_) => return,
        };

        let size = self.bytes_per_line * self.height as usize;
        let shmid = unsafe {
            libc::shmget(
                libc::IPC_PRIVATE,
                size,
                libc::IPC_CREAT | 0o600,
            )
        };

        if shmid < 0 {
            return;
        }

        let shmaddr = unsafe { libc::shmat(shmid, std::ptr::null(), 0) };
        if shmaddr as isize == -1 {
            unsafe {
                libc::shmctl(shmid, libc::IPC_RMID, std::ptr::null_mut());
            }
            return;
        }

        let attach_cookie = shm::attach(self.conn.as_ref(), shmseg, shmid as u32, false);
        if attach_cookie.is_err() {
            unsafe {
                libc::shmdt(shmaddr);
                libc::shmctl(shmid, libc::IPC_RMID, std::ptr::null_mut());
            }
            return;
        }
        let _ = self.conn.flush();

        self.use_shm = true;
        self.shmseg = shmseg;
        self.shmid = shmid;
        self.shmaddr = shmaddr as *mut u8;
        self.shm_size = size;
        debug!("Using XShm capture");
    }

    /// Capture a frame
    pub fn capture(&mut self) -> Result<Frame, Box<dyn std::error::Error>> {
        let start = std::time::Instant::now();

        let data = if self.use_shm {
            let format = u8::from(ImageFormat::Z_PIXMAP);
            let cookie = shm::get_image(
                self.conn.as_ref(),
                self.root,
                0,
                0,
                self.width as u16,
                self.height as u16,
                u32::MAX,
                format,
                self.shmseg,
                0,
            )?;
            let _ = cookie.reply();
            let src = unsafe {
                std::slice::from_raw_parts(self.shmaddr as *const u8, self.shm_size)
            };
            let effective_height = self.effective_height(src.len());
            self.convert_raw_to_rgb(src, effective_height)
        } else {
            let image = self
                .conn
                .get_image(
                    ImageFormat::Z_PIXMAP,
                    self.root,
                    0,
                    0,
                    self.width as u16,
                    self.height as u16,
                    u32::MAX,
                )?
                .reply()?;
            let effective_height = self.effective_height(image.data.len());
            self.convert_raw_to_rgb(&image.data, effective_height)
        };

        let seq = self.sequence.fetch_add(1, Ordering::SeqCst);

        let capture_time_us = start.elapsed().as_micros() as u64;
        self.stats.lock().unwrap().record_capture(data.len(), capture_time_us);

        let height = (data.len() / (self.width as usize * 3))
            .min(self.height as usize) as u32;
        Ok(Frame {
            width: self.width,
            height,
            data,
            timestamp: start,
            sequence: seq,
            is_dirty: true,
        })
    }

    /// Convert raw image data to RGB format
    fn convert_raw_to_rgb(&self, src: &[u8], height: u32) -> Vec<u8> {
        let mut dst = Vec::with_capacity((self.width * height * 3) as usize);

        // Handle different image formats
        match self.depth {
            24 => {
                for _y in 0..height {
                    for _x in 0..self.width {
                        let offset = (_y as usize * self.bytes_per_line)
                            + (_x as usize * self.bytes_per_pixel);
                        if offset + 3 <= src.len() {
                            let (r, g, b) = if self.byte_order == BYTE_ORDER_LSB_FIRST {
                                // Little endian: BGR (typically)
                                (src[offset + 2], src[offset + 1], src[offset])
                            } else {
                                // Big endian: RGB
                                (src[offset], src[offset + 1], src[offset + 2])
                            };
                            dst.push(r);
                            dst.push(g);
                            dst.push(b);
                        }
                    }
                }
            }
            16 => {
                // 16-bit RGB565
                for _y in 0..height {
                    for _x in 0..self.width {
                        let offset = (_y as usize * self.bytes_per_line)
                            + (_x as usize * self.bytes_per_pixel);
                        if offset + 2 <= src.len() {
                            let pixel = u16::from_le_bytes([src[offset], src[offset + 1]]);
                            let r = ((pixel >> 11) & 0x1F) as u8;
                            let g = ((pixel >> 5) & 0x3F) as u8;
                            let b = (pixel & 0x1F) as u8;
                            dst.push(r << 3);
                            dst.push(g << 2);
                            dst.push(b << 3);
                        }
                    }
                }
            }
            _ => {
                debug!("Unsupported depth: {}, using grayscale", self.depth);
                // Fall back to grayscale
                let step = self.bytes_per_pixel.max(1);
                for byte in src.iter().step_by(step) {
                    dst.push(*byte);
                    dst.push(*byte);
                    dst.push(*byte);
                }
            }
        }

        dst
    }

    fn effective_height(&self, data_len: usize) -> u32 {
        let max_rows = data_len / self.bytes_per_line;
        let height = (max_rows.min(self.height as usize)) as u32;
        if height != self.height {
            debug!(
                "Truncated frame: expected {} rows, got {} rows",
                self.height, height
            );
        }
        height
    }

    /// Get screen dimensions
    pub fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }
}

impl Drop for X11Capturer {
    fn drop(&mut self) {
        if self.use_shm && !self.shmaddr.is_null() {
            unsafe {
                let _ = shm::detach(self.conn.as_ref(), self.shmseg);
                libc::shmdt(self.shmaddr as *mut _);
                libc::shmctl(self.shmid, libc::IPC_RMID, std::ptr::null_mut());
            }
        }
    }
}

fn compute_format(conn: &XCBConnection, width: u32, depth: u8) -> (usize, usize) {
    let mut bytes_per_pixel = 4usize;
    let mut bytes_per_line = width as usize * bytes_per_pixel;
    for format in &conn.setup().pixmap_formats {
        if format.depth == depth {
            let bpp = format.bits_per_pixel as usize;
            let pad = format.scanline_pad as usize;
            bytes_per_pixel = (bpp / 8).max(1);
            let bits_per_line = width as usize * bpp;
            let padded_bits = ((bits_per_line + pad - 1) / pad) * pad;
            bytes_per_line = padded_bits / 8;
            return (bytes_per_pixel, bytes_per_line);
        }
    }
    (bytes_per_pixel, bytes_per_line)
}
