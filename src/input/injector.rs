//! X11 input injection using XTest
//!
//! Provides keyboard and mouse simulation via XTest extension.

use std::collections::HashMap;
use std::sync::Arc;
use x11rb::connection::Connection;
use x11rb::errors::ConnectionError;
use x11rb::protocol::xtest;
use x11rb::protocol::xproto::*;
use x11rb::xcb_ffi::XCBConnection;

/// XTest input constants
const INPUT_KEY_PRESS: u8 = 2;
const INPUT_KEY_RELEASE: u8 = 3;
const INPUT_BUTTON_PRESS: u8 = 4;
const INPUT_BUTTON_RELEASE: u8 = 5;

/// Input event types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputEvent {
    /// Mouse move event
    MouseMove,
    /// Mouse button event
    MouseButton,
    /// Mouse wheel event
    MouseWheel,
    /// Keyboard event
    Keyboard,
    /// Text input event
    TextInput,
    /// Clipboard event
    Clipboard,
    /// Ping event
    Ping,
}

/// Input event data
#[derive(Debug, Clone)]
pub struct InputEventData {
    /// Event type
    pub event_type: InputEvent,
    /// Mouse X position
    pub mouse_x: i32,
    /// Mouse Y position
    pub mouse_y: i32,
    /// Mouse button number (1-5)
    pub mouse_button: u8,
    /// Button pressed (true) or released (false)
    pub button_pressed: bool,
    /// Mouse wheel delta X
    pub wheel_delta_x: i16,
    /// Mouse wheel delta Y
    pub wheel_delta_y: i16,
    /// Keyboard keysym
    pub keysym: u32,
    /// Key pressed (true) or released (false)
    pub key_pressed: bool,
    /// Button mask for mouse events
    pub button_mask: u32,
    /// Text content for text input/clipboard
    pub text: String,
    /// Timestamp for ping events
    pub timestamp: u64,
}

impl Default for InputEventData {
    fn default() -> Self {
        Self {
            event_type: InputEvent::MouseMove,
            mouse_x: 0,
            mouse_y: 0,
            mouse_button: 0,
            button_pressed: false,
            wheel_delta_x: 0,
            wheel_delta_y: 0,
            keysym: 0,
            key_pressed: false,
            button_mask: 0,
            text: String::new(),
            timestamp: 0,
        }
    }
}

/// X11 Input injector using XTest extension
pub struct InputInjector {
    /// XCB connection
    conn: Arc<XCBConnection>,
    /// Root window
    root: Window,
    /// Current mouse position
    mouse_x: i32,
    /// Current mouse position
    mouse_y: i32,
    /// Input configuration
    config: InputConfig,
    /// Keysym to keycode cache
    keysym_cache: HashMap<u32, u8>,
}

/// Input configuration
#[derive(Debug, Clone)]
pub struct InputConfig {
    /// Enable keyboard input
    pub enable_keyboard: bool,
    /// Enable mouse input
    pub enable_mouse: bool,
    /// Mouse sensitivity
    pub mouse_sensitivity: f64,
}

impl Default for InputConfig {
    fn default() -> Self {
        Self {
            enable_keyboard: true,
            enable_mouse: true,
            mouse_sensitivity: 1.0,
        }
    }
}

impl InputInjector {
    /// Create a new input injector
    pub fn new(
        conn: Arc<XCBConnection>,
        screen_num: i32,
        config: InputConfig,
    ) -> Result<Self, ConnectionError> {
        let screen = &conn.setup().roots[screen_num as usize];
        let root = screen.root;

        // Build keysym-to-keycode cache
        let mut keysym_cache = HashMap::new();
        let min_keycode = conn.setup().min_keycode;
        let max_keycode = conn.setup().max_keycode;
        if let Ok(cookie) = conn.get_keyboard_mapping(min_keycode, max_keycode - min_keycode + 1) {
            if let Ok(mapping) = cookie.reply() {
                let keysyms_per_keycode = mapping.keysyms_per_keycode as usize;
                for i in 0..=(max_keycode - min_keycode) as usize {
                    let offset = i * keysyms_per_keycode;
                    if offset < mapping.keysyms.len() && mapping.keysyms[offset] != 0 {
                        keysym_cache.entry(mapping.keysyms[offset])
                            .or_insert((min_keycode as usize + i) as u8);
                    }
                }
            }
        }

        Ok(Self {
            conn,
            root,
            mouse_x: 0,
            mouse_y: 0,
            config,
            keysym_cache,
        })
    }

    /// Inject mouse movement
    pub fn mouse_move(&mut self, x: i32, y: i32) -> Result<(), Box<dyn std::error::Error>> {
        if !self.config.enable_mouse {
            return Ok(());
        }

        // Coordinates are absolute screen positions; sensitivity does not apply
        self.mouse_x = x;
        self.mouse_y = y;

        // Warp pointer to new position (clamp to i16 range for X11 protocol)
        let wx = self.mouse_x.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
        let wy = self.mouse_y.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
        self.conn
            .warp_pointer(0u32, self.root, 0, 0, 0, 0, wx, wy)?;

        self.conn.flush()?;
        Ok(())
    }

    /// Inject mouse button press/release
    pub fn mouse_button(
        &mut self,
        button: u8,
        pressed: bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if !self.config.enable_mouse {
            return Ok(());
        }

        let input_type = if pressed { INPUT_BUTTON_PRESS } else { INPUT_BUTTON_RELEASE };

        xtest::fake_input(
            &self.conn,
            input_type,
            button,
            0,
            self.root,
            self.mouse_x as i16,
            self.mouse_y as i16,
            0,
        )?;

        self.conn.flush()?;
        Ok(())
    }

    /// Inject mouse wheel scroll
    pub fn mouse_wheel(&mut self, delta_x: i16, delta_y: i16) -> Result<(), Box<dyn std::error::Error>> {
        if !self.config.enable_mouse {
            return Ok(());
        }

        // Map wheel deltas to button events (4=up, 5=down, 6=left, 7=right)
        // Browser wheel events send deltas in multiples of 120; normalize to discrete steps
        let norm_y = (delta_y as i32) / 120;
        let steps_y = norm_y.unsigned_abs().max(if delta_y != 0 { 1 } else { 0 }).min(10) as usize;
        for _ in 0..steps_y {
            if delta_y < 0 {
                self.mouse_button(4, true)?;
                self.mouse_button(4, false)?;
            } else if delta_y > 0 {
                self.mouse_button(5, true)?;
                self.mouse_button(5, false)?;
            }
        }

        let norm_x = (delta_x as i32) / 120;
        let steps_x = norm_x.unsigned_abs().max(if delta_x != 0 { 1 } else { 0 }).min(10) as usize;
        for _ in 0..steps_x {
            if delta_x < 0 {
                self.mouse_button(6, true)?;
                self.mouse_button(6, false)?;
            } else if delta_x > 0 {
                self.mouse_button(7, true)?;
                self.mouse_button(7, false)?;
            }
        }

        self.conn.flush()?;
        Ok(())
    }

    /// Inject keyboard press/release
    pub fn keyboard(&mut self, keysym: u32, pressed: bool) -> Result<(), Box<dyn std::error::Error>> {
        if !self.config.enable_keyboard {
            return Ok(());
        }

        // Convert keysym to keycode
        let keycode = self.keysym_to_keycode(keysym);

        if let Some(kc) = keycode {
            let input_type = if pressed { INPUT_KEY_PRESS } else { INPUT_KEY_RELEASE };

            xtest::fake_input(
                &self.conn,
                input_type,
                kc,
                0,
                self.root,
                self.mouse_x as i16,
                self.mouse_y as i16,
                0,
            )?;

            self.conn.flush()?;
        }

        Ok(())
    }

    /// Convert X keysym to X keycode (cached)
    fn keysym_to_keycode(&self, keysym: u32) -> Option<u8> {
        if let Some(&kc) = self.keysym_cache.get(&keysym) {
            return Some(kc);
        }

        // Fallback: linear scan for keysyms not in cache
        let min_keycode = self.conn.setup().min_keycode;
        let max_keycode = self.conn.setup().max_keycode;

        for kc in min_keycode..=max_keycode {
            let keysyms = self
                .conn
                .get_keyboard_mapping(kc as u8, 1)
                .ok()
                .and_then(|cookie| cookie.reply().ok());

            if let Some(mapping) = keysyms {
                if !mapping.keysyms.is_empty() && mapping.keysyms[0] == keysym as u32 {
                    return Some(kc as u8);
                }
            }
        }

        None
    }

    /// Process a single event payload
    pub fn process_event(&mut self, event: InputEventData) -> Result<(), Box<dyn std::error::Error>> {
        match event.event_type {
            InputEvent::MouseMove => self.mouse_move(event.mouse_x, event.mouse_y),
            InputEvent::MouseButton => self.mouse_button(event.mouse_button, event.button_pressed),
            InputEvent::MouseWheel => self.mouse_wheel(event.wheel_delta_x, event.wheel_delta_y),
            InputEvent::Keyboard => self.keyboard(event.keysym, event.key_pressed),
            InputEvent::TextInput => {
                // Text input events are handled at a higher level
                Ok(())
            }
            InputEvent::Clipboard => {
                // Clipboard events are handled at a higher level
                Ok(())
            }
            InputEvent::Ping => {
                // Ping events don't require X11 injection
                Ok(())
            }
        }
    }
}
