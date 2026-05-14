use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;
use std::str::FromStr;

use crossterm::event::KeyCode;
use crossterm::event::KeyModifiers;

use super::KeyBind;

impl Display for KeyBind {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result { f.write_str(&self.display()) }
}

impl FromStr for KeyBind {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> { parse_keybind(s) }
}

/// Canonical forms:
/// - `KeyCode::Char('=')` for the `=`/`+` physical key
/// - `KeyCode::Tab` for `BackTab` (Shift is added to modifiers)
pub(super) const fn normalize_code(code: KeyCode) -> KeyCode {
    match code {
        KeyCode::Char('+') => KeyCode::Char('='),
        KeyCode::BackTab => KeyCode::Tab,
        other => other,
    }
}

pub(super) fn code_label(code: KeyCode) -> String {
    match code {
        KeyCode::Char('=') => "+".to_string(),
        KeyCode::Char(c) => c.to_string(),
        KeyCode::Enter => "Enter".to_string(),
        KeyCode::Esc => "Esc".to_string(),
        KeyCode::Tab | KeyCode::BackTab => "Tab".to_string(),
        KeyCode::Backspace => "Backspace".to_string(),
        KeyCode::Delete => "Delete".to_string(),
        KeyCode::Home => "Home".to_string(),
        KeyCode::End => "End".to_string(),
        KeyCode::Up => "Up".to_string(),
        KeyCode::Down => "Down".to_string(),
        KeyCode::Left => "Left".to_string(),
        KeyCode::Right => "Right".to_string(),
        KeyCode::PageUp => "PageUp".to_string(),
        KeyCode::PageDown => "PageDown".to_string(),
        KeyCode::F(n) => format!("F{n}"),
        _ => format!("{code:?}"),
    }
}

fn parse_keybind(s: &str) -> Result<KeyBind, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("empty key string".to_string());
    }

    // Bare "+" is the plus/equals key, not a modifier separator.
    if s == "+" || s == "=" {
        return Ok(KeyBind::plain(KeyCode::Char('+')));
    }

    let parts: Vec<&str> = s.split('+').collect();

    // Single-character key with no modifiers: e.g. "q", "/", "-"
    if parts.len() == 1 {
        let code = parse_key_code(parts[0])?;
        return Ok(KeyBind::new(code, KeyModifiers::NONE));
    }

    // Last part is the key, preceding parts are modifiers.
    let (modifier_parts, key_part) = parts.split_at(parts.len() - 1);
    let key_part = key_part[0];

    if key_part.is_empty() {
        return Err(format!("modifier with no key: \"{s}\""));
    }

    let mut modifiers = KeyModifiers::NONE;
    for modifier in modifier_parts {
        match modifier.to_lowercase().as_str() {
            "ctrl" | "control" => modifiers |= KeyModifiers::CONTROL,
            "alt" | "option" => modifiers |= KeyModifiers::ALT,
            "shift" => modifiers |= KeyModifiers::SHIFT,
            other => return Err(format!("unknown modifier: \"{other}\"")),
        }
    }

    let code = parse_key_code(key_part)?;
    Ok(KeyBind::new(code, modifiers))
}

fn parse_key_code(s: &str) -> Result<KeyCode, String> {
    // Named keys (case-insensitive).
    match s.to_lowercase().as_str() {
        "enter" | "return" => return Ok(KeyCode::Enter),
        "esc" | "escape" => return Ok(KeyCode::Esc),
        "tab" => return Ok(KeyCode::Tab),
        "backspace" => return Ok(KeyCode::Backspace),
        "delete" | "del" => return Ok(KeyCode::Delete),
        "home" => return Ok(KeyCode::Home),
        "end" => return Ok(KeyCode::End),
        "up" => return Ok(KeyCode::Up),
        "down" => return Ok(KeyCode::Down),
        "left" => return Ok(KeyCode::Left),
        "right" => return Ok(KeyCode::Right),
        "pageup" => return Ok(KeyCode::PageUp),
        "pagedown" => return Ok(KeyCode::PageDown),
        "space" => return Ok(KeyCode::Char(' ')),
        _ => {},
    }

    // F-keys: "F1" .. "F12".
    if let Some(n) = s.strip_prefix('F').or_else(|| s.strip_prefix('f'))
        && let Ok(n) = n.parse::<u8>()
        && (1..=12).contains(&n)
    {
        return Ok(KeyCode::F(n));
    }

    // Single character.
    let mut chars = s.chars();
    if let Some(c) = chars.next()
        && chars.next().is_none()
    {
        return Ok(KeyCode::Char(c));
    }

    Err(format!("unknown key: \"{s}\""))
}
