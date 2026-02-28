//! Key name → X11 keysym mapping and combo key parsing for MCP keyboard tools.

/// Parse a key combination string like "Ctrl+c" or "Alt+F4" into
/// (modifier_keysyms, main_keysym).
pub fn parse_key_combo(key: &str) -> Result<(Vec<u32>, u32), String> {
    let parts: Vec<&str> = key.split('+').collect();
    if parts.is_empty() || (parts.len() == 1 && parts[0].is_empty()) {
        return Err("empty key string".into());
    }

    let main_key = parts.last().unwrap();
    let modifier_names = &parts[..parts.len() - 1];

    let mut modifiers = Vec::new();
    for m in modifier_names {
        let sym = modifier_keysym(m)
            .ok_or_else(|| format!("unknown modifier: {}", m))?;
        modifiers.push(sym);
    }

    let main_sym = get_keysym(main_key)
        .ok_or_else(|| format!("unknown key: {}", main_key))?;

    Ok((modifiers, main_sym))
}

/// Check if a character requires Shift to type on a US keyboard.
pub fn char_needs_shift(c: char) -> bool {
    matches!(c,
        '~' | '!' | '@' | '#' | '$' | '%' | '^' | '&' | '*' | '(' | ')' |
        '_' | '+' | '{' | '}' | '|' | ':' | '"' | '<' | '>' | '?' |
        'A'..='Z'
    )
}

/// Get the unshifted base character for a shifted character on US keyboard.
pub fn get_unshifted_char(c: char) -> char {
    match c {
        '~' => '`', '!' => '1', '@' => '2', '#' => '3', '$' => '4',
        '%' => '5', '^' => '6', '&' => '7', '*' => '8', '(' => '9',
        ')' => '0', '_' => '-', '+' => '=', '{' => '[', '}' => ']',
        '|' => '\\', ':' => ';', '"' => '\'', '<' => ',', '>' => '.',
        '?' => '/',
        c if c.is_ascii_uppercase() => c.to_ascii_lowercase(),
        other => other,
    }
}

/// Get keysym for a modifier name (case-insensitive).
fn modifier_keysym(name: &str) -> Option<u32> {
    match name.to_lowercase().as_str() {
        "ctrl" | "control" => Some(0xffe3), // Control_L
        "shift"            => Some(0xffe1), // Shift_L
        "alt"              => Some(0xffe9), // Alt_L
        "super" | "meta" | "cmd" | "win" => Some(0xffeb), // Super_L
        _ => None,
    }
}

/// Map a key name to its X11 keysym. Supports:
/// - Single characters (letters, digits, symbols)
/// - Named keys (Return, Tab, Escape, F1-F12, arrows, etc.)
pub fn get_keysym(name: &str) -> Option<u32> {
    // Single character
    if name.len() == 1 {
        let c = name.chars().next().unwrap();
        return Some(char_to_keysym(c));
    }

    // Named keys (case-insensitive)
    match name.to_lowercase().as_str() {
        // Whitespace / control
        "return" | "enter"     => Some(0xff0d),
        "tab"                  => Some(0xff09),
        "backspace" | "back"   => Some(0xff08),
        "delete" | "del"       => Some(0xffff),
        "escape" | "esc"       => Some(0xff1b),
        "space"                => Some(0x0020),
        // Navigation
        "home"                 => Some(0xff50),
        "end"                  => Some(0xff57),
        "pageup" | "page_up"   => Some(0xff55),
        "pagedown" | "page_down" => Some(0xff56),
        "insert"               => Some(0xff63),
        // Arrow keys
        "up" | "arrowup"       => Some(0xff52),
        "down" | "arrowdown"   => Some(0xff54),
        "left" | "arrowleft"   => Some(0xff51),
        "right" | "arrowright" => Some(0xff53),
        // Function keys
        "f1"  => Some(0xffbe), "f2"  => Some(0xffbf),
        "f3"  => Some(0xffc0), "f4"  => Some(0xffc1),
        "f5"  => Some(0xffc2), "f6"  => Some(0xffc3),
        "f7"  => Some(0xffc4), "f8"  => Some(0xffc5),
        "f9"  => Some(0xffc6), "f10" => Some(0xffc7),
        "f11" => Some(0xffc8), "f12" => Some(0xffc9),
        // Modifiers (also valid as main keys)
        "ctrl" | "control"     => Some(0xffe3),
        "shift"                => Some(0xffe1),
        "alt"                  => Some(0xffe9),
        "super" | "meta" | "cmd" | "win" => Some(0xffeb),
        // Misc
        "capslock" | "caps_lock" => Some(0xffe5),
        "numlock" | "num_lock"   => Some(0xff7f),
        "scrolllock" | "scroll_lock" => Some(0xff14),
        "print" | "printscreen"  => Some(0xff61),
        "pause"                  => Some(0xff13),
        "menu"                   => Some(0xff67),
        _ => None,
    }
}

/// Convert a single character to its X11 keysym.
pub fn char_to_keysym(c: char) -> u32 {
    match c {
        // ASCII printable range maps directly
        ' '..='~' => c as u32,
        // For Unicode characters outside ASCII, use Unicode keysym encoding
        _ => 0x01000000 | (c as u32),
    }
}
