//! X11 Input injection
//!
//! Uses XTest extension to simulate keyboard and mouse input.

pub mod injector;
pub use injector::{InputConfig, InputEvent, InputEventData, InputInjector};
