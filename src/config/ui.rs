//! UI configuration derived from runtime config and environment overrides.

use crate::config::Config;
use log::warn;
use serde::Serialize;
use std::env;

#[derive(Debug, Clone, Serialize)]
pub struct UiConfig {
    pub version: String,
    pub ui: UiVisibility,
    pub video: UiVideo,
    pub screen: UiScreen,
    pub audio: UiAudio,
    pub input: UiInput,
    pub clipboard: UiToggle,
    pub stats: UiToggle,
}

#[derive(Debug, Clone, Serialize)]
pub struct UiVisibility {
    pub show_sidebar: bool,
    pub show_video_settings: bool,
    pub show_screen_settings: bool,
    pub show_audio_settings: bool,
    pub show_stats: bool,
    pub show_clipboard: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct UiVideo {
    pub encoder: UiEnum,
    pub framerate: UiRangeU32,
}

#[derive(Debug, Clone, Serialize)]
pub struct UiScreen {
    pub manual_resolution: UiBool,
    pub width: UiValueU32,
    pub height: UiValueU32,
}

#[derive(Debug, Clone, Serialize)]
pub struct UiAudio {
    pub enabled: UiBool,
    pub bitrate: UiRangeU32,
}

#[derive(Debug, Clone, Serialize)]
pub struct UiInput {
    pub mouse: UiBool,
    pub keyboard: UiBool,
}

#[derive(Debug, Clone, Serialize)]
pub struct UiToggle {
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct UiBool {
    pub value: bool,
    pub locked: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct UiEnum {
    pub value: String,
    pub options: Vec<String>,
    pub locked: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct UiRangeU32 {
    pub value: u32,
    pub min: u32,
    pub max: u32,
    pub locked: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct UiValueU32 {
    pub value: u32,
    pub locked: bool,
}

impl UiConfig {
    pub fn from_env(config: &Config) -> Self {
        let ui = UiVisibility {
            show_sidebar: env_bool("SELKIES_UI_SHOW_SIDEBAR", true).value,
            show_video_settings: env_bool("SELKIES_UI_SIDEBAR_SHOW_VIDEO_SETTINGS", true).value,
            show_screen_settings: env_bool("SELKIES_UI_SIDEBAR_SHOW_SCREEN_SETTINGS", true).value,
            show_audio_settings: env_bool("SELKIES_UI_SIDEBAR_SHOW_AUDIO_SETTINGS", true).value,
            show_stats: env_bool("SELKIES_UI_SIDEBAR_SHOW_STATS", true).value,
            show_clipboard: env_bool("SELKIES_UI_SIDEBAR_SHOW_CLIPBOARD", true).value,
        };

        let encoder = env_encoder("SELKIES_ENCODER");
        let framerate = env_range_u32(
            "SELKIES_FRAMERATE",
            config.encoding.target_fps,
            1,
            config.encoding.max_fps.max(1),
        );

        let (manual_resolution, width, height) = env_manual_resolution(
            config.display.width,
            config.display.height,
        );

        let audio_enabled = env_bool("SELKIES_AUDIO_ENABLED", config.audio.enabled);
        let audio_bitrate = env_range_u32(
            "SELKIES_AUDIO_BITRATE",
            config.audio.bitrate,
            64_000,
            320_000,
        );

        let mouse = env_bool("SELKIES_MOUSE_ENABLED", config.input.enable_mouse);
        let keyboard = env_bool("SELKIES_KEYBOARD_ENABLED", config.input.enable_keyboard);

        let clipboard_enabled = env_bool("SELKIES_CLIPBOARD_ENABLED", config.input.enable_clipboard);

        UiConfig {
            version: "1".to_string(),
            ui,
            video: UiVideo {
                encoder,
                framerate,
            },
            screen: UiScreen {
                manual_resolution,
                width,
                height,
            },
            audio: UiAudio {
                enabled: audio_enabled,
                bitrate: audio_bitrate,
            },
            input: UiInput {
                mouse,
                keyboard,
            },
            clipboard: UiToggle {
                enabled: clipboard_enabled.value,
            },
            stats: UiToggle { enabled: true },
        }
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".to_string())
    }
}

fn env_bool(key: &str, default_value: bool) -> UiBool {
    match env::var(key) {
        Ok(raw) => {
            let (value_raw, locked) = split_locked(&raw);
            let value = match value_raw.to_ascii_lowercase().as_str() {
                "true" | "1" => true,
                "false" | "0" => false,
                _ => {
                    warn!("Invalid boolean for {}: {}", key, value_raw);
                    default_value
                }
            };
            UiBool { value, locked }
        }
        Err(_) => UiBool {
            value: default_value,
            locked: false,
        },
    }
}

fn env_range_u32(key: &str, default_value: u32, default_min: u32, default_max: u32) -> UiRangeU32 {
    let mut result = UiRangeU32 {
        value: default_value.clamp(default_min, default_max),
        min: default_min,
        max: default_max,
        locked: false,
    };

    let raw = match env::var(key) {
        Ok(raw) => raw,
        Err(_) => return result,
    };

    let (value_raw, locked_hint) = split_locked(&raw);
    if let Some((min, max, is_range)) = parse_range_u32(&value_raw) {
        result.min = min;
        result.max = max;
        result.value = if is_range {
            result.value.clamp(min, max)
        } else {
            min
        };
        result.locked = locked_hint || !is_range;
    } else {
        warn!("Invalid range for {}: {}", key, value_raw);
    }

    result
}

fn env_encoder(key: &str) -> UiEnum {
    let default = UiEnum {
        value: "jpeg".to_string(),
        options: vec!["jpeg".to_string()],
        locked: true,
    };

    let raw = match env::var(key) {
        Ok(raw) => raw,
        Err(_) => return default,
    };

    let (value_raw, locked_hint) = split_locked(&raw);
    let mut options: Vec<String> = value_raw
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    if options.is_empty() {
        return default;
    }

    if !options.iter().any(|opt| opt == "jpeg") {
        warn!("Unsupported encoder list {:?}, forcing jpeg", options);
        return default;
    }

    options.retain(|opt| opt == "jpeg");
    let locked = locked_hint || options.len() == 1;
    UiEnum {
        value: "jpeg".to_string(),
        options,
        locked,
    }
}

fn env_manual_resolution(default_width: u32, default_height: u32) -> (UiBool, UiValueU32, UiValueU32) {
    let mut enabled = false;
    let mut locked = false;
    let mut width = default_width;
    let mut height = default_height;

    if let Ok(raw) = env::var("SELKIES_MANUAL_WIDTH") {
        if let Ok(value) = raw.parse::<u32>() {
            width = value;
            enabled = true;
            locked = true;
        } else {
            warn!("Invalid SELKIES_MANUAL_WIDTH: {}", raw);
        }
    }

    if let Ok(raw) = env::var("SELKIES_MANUAL_HEIGHT") {
        if let Ok(value) = raw.parse::<u32>() {
            height = value;
            enabled = true;
            locked = true;
        } else {
            warn!("Invalid SELKIES_MANUAL_HEIGHT: {}", raw);
        }
    }

    if let Ok(raw) = env::var("SELKIES_IS_MANUAL_RESOLUTION_MODE") {
        let (value_raw, _locked_hint) = split_locked(&raw);
        match value_raw.to_ascii_lowercase().as_str() {
            "true" | "1" => {
                enabled = true;
                locked = true;
                if env::var("SELKIES_MANUAL_WIDTH").is_err()
                    && env::var("SELKIES_MANUAL_HEIGHT").is_err()
                {
                    width = 1024;
                    height = 768;
                }
            }
            "false" | "0" => {}
            _ => warn!("Invalid SELKIES_IS_MANUAL_RESOLUTION_MODE: {}", value_raw),
        }
    }

    (
        UiBool {
            value: enabled,
            locked,
        },
        UiValueU32 {
            value: width,
            locked,
        },
        UiValueU32 {
            value: height,
            locked,
        },
    )
}

fn split_locked(raw: &str) -> (String, bool) {
    if let Some((value, suffix)) = raw.rsplit_once('|') {
        if suffix == "locked" {
            return (value.to_string(), true);
        }
    }
    (raw.to_string(), false)
}

fn parse_range_u32(raw: &str) -> Option<(u32, u32, bool)> {
    if let Some((min_raw, max_raw)) = raw.split_once('-') {
        let min = min_raw.trim().parse::<u32>().ok()?;
        let max = max_raw.trim().parse::<u32>().ok()?;
        let (min, max) = if min > max { (max, min) } else { (min, max) };
        return Some((min, max, true));
    }

    let value = raw.trim().parse::<u32>().ok()?;
    Some((value, value, false))
}
