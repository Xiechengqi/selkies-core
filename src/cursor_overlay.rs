//! Cursor overlay module
//!
//! Draws a simple cursor on captured frames since XFixes GetCursorImage
//! is not supported on Xvfb.

use crate::capture::Frame;

/// Cursor position
#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub struct CursorPosition {
    pub x: i16,
    pub y: i16,
}

/// Draw a simple arrow cursor on the frame
///
/// Since Xvfb doesn't support XFixes GetCursorImage, we draw a simple
/// arrow cursor directly on the frame buffer.
#[allow(dead_code)]
pub fn draw_cursor_on_frame(frame: &mut Frame, cursor_pos: CursorPosition) {
    let x = cursor_pos.x as i32;
    let y = cursor_pos.y as i32;

    // Check bounds
    if x < 0 || y < 0 || x >= frame.width as i32 || y >= frame.height as i32 {
        return;
    }

    // Simple arrow cursor pattern (11x16 pixels)
    // 1 = white, 0 = black, 2 = transparent (skip)
    #[rustfmt::skip]
    let cursor_pattern: &[(i32, i32, u8)] = &[
        // Row 0
        (0, 0, 1),
        // Row 1
        (0, 1, 1), (1, 1, 1),
        // Row 2
        (0, 2, 1), (1, 2, 0), (2, 2, 1),
        // Row 3
        (0, 3, 1), (1, 3, 0), (2, 3, 0), (3, 3, 1),
        // Row 4
        (0, 4, 1), (1, 4, 0), (2, 4, 0), (3, 4, 0), (4, 4, 1),
        // Row 5
        (0, 5, 1), (1, 5, 0), (2, 5, 0), (3, 5, 0), (4, 5, 0), (5, 5, 1),
        // Row 6
        (0, 6, 1), (1, 6, 0), (2, 6, 0), (3, 6, 0), (4, 6, 0), (5, 6, 0), (6, 6, 1),
        // Row 7
        (0, 7, 1), (1, 7, 0), (2, 7, 0), (3, 7, 0), (4, 7, 0), (5, 7, 0), (6, 7, 0), (7, 7, 1),
    ];

    draw_cursor_pattern(frame, x, y, cursor_pattern);
}

/// Helper function to draw cursor pattern on frame
#[allow(dead_code)]
fn draw_cursor_pattern(frame: &mut Frame, x: i32, y: i32, pattern: &[(i32, i32, u8)]) {
    let width = frame.width as i32;
    let height = frame.height as i32;
    let bytes_per_pixel = 4; // Assuming BGRA format

    for &(dx, dy, color) in pattern {
        let px = x + dx;
        let py = y + dy;

        // Check bounds
        if px < 0 || py < 0 || px >= width || py >= height {
            continue;
        }

        let offset = ((py * width + px) * bytes_per_pixel) as usize;
        if offset + 3 >= frame.data.len() {
            continue;
        }

        // Set pixel color (BGRA format)
        match color {
            0 => {
                // Black
                frame.data[offset] = 0;     // B
                frame.data[offset + 1] = 0; // G
                frame.data[offset + 2] = 0; // R
                frame.data[offset + 3] = 255; // A
            }
            1 => {
                // White
                frame.data[offset] = 255;   // B
                frame.data[offset + 1] = 255; // G
                frame.data[offset + 2] = 255; // R
                frame.data[offset + 3] = 255; // A
            }
            _ => {} // Transparent, skip
        }
    }
}
