//! Key names in Textual's spelling (`ctrl+s`, `f2`, `escape`, `shift+tab`,
//! single characters), so bindings and test scripts read like the Python.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

/// The Textual name of a key press.
pub fn key_name(k: &KeyEvent) -> String {
    let base = match k.code {
        KeyCode::Char(c) => {
            if k.modifiers.contains(KeyModifiers::CONTROL)
                || k.modifiers.contains(KeyModifiers::ALT)
            {
                c.to_lowercase().to_string()
            } else {
                return c.to_string();
            }
        }
        KeyCode::Enter => "enter".into(),
        KeyCode::Esc => "escape".into(),
        KeyCode::Tab => "tab".into(),
        KeyCode::BackTab => return "shift+tab".into(),
        KeyCode::Backspace => "backspace".into(),
        KeyCode::Delete => "delete".into(),
        KeyCode::Left => "left".into(),
        KeyCode::Right => "right".into(),
        KeyCode::Up => "up".into(),
        KeyCode::Down => "down".into(),
        KeyCode::Home => "home".into(),
        KeyCode::End => "end".into(),
        KeyCode::PageUp => "pageup".into(),
        KeyCode::PageDown => "pagedown".into(),
        KeyCode::F(n) => format!("f{n}"),
        _ => return String::new(),
    };
    let mut out = String::new();
    if k.modifiers.contains(KeyModifiers::CONTROL) {
        out.push_str("ctrl+");
    }
    if k.modifiers.contains(KeyModifiers::ALT) {
        out.push_str("alt+");
    }
    if k.modifiers.contains(KeyModifiers::SHIFT) && !matches!(k.code, KeyCode::Char(_)) {
        out.push_str("shift+");
    }
    out + &base
}

/// A key event from a Textual name (test scripts).
pub fn parse_key(name: &str) -> KeyEvent {
    let mut mods = KeyModifiers::NONE;
    let mut rest = name;
    loop {
        if let Some(r) = rest.strip_prefix("ctrl+") {
            mods |= KeyModifiers::CONTROL;
            rest = r;
        } else if let Some(r) = rest.strip_prefix("alt+") {
            mods |= KeyModifiers::ALT;
            rest = r;
        } else if let Some(r) = rest.strip_prefix("shift+").filter(|r| r.len() > 1) {
            mods |= KeyModifiers::SHIFT;
            rest = r;
        } else {
            break;
        }
    }
    let code = match rest {
        "enter" => KeyCode::Enter,
        "escape" => KeyCode::Esc,
        "tab" if mods.contains(KeyModifiers::SHIFT) => {
            mods.remove(KeyModifiers::SHIFT);
            KeyCode::BackTab
        }
        "tab" => KeyCode::Tab,
        "backspace" => KeyCode::Backspace,
        "delete" => KeyCode::Delete,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "pageup" => KeyCode::PageUp,
        "pagedown" => KeyCode::PageDown,
        "space" => KeyCode::Char(' '),
        f if f.len() > 1 && f.starts_with('f') && f[1..].parse::<u8>().is_ok() => {
            KeyCode::F(f[1..].parse().unwrap_or(1))
        }
        other => KeyCode::Char(other.chars().next().unwrap_or(' ')),
    };
    KeyEvent {
        code,
        modifiers: mods,
        kind: KeyEventKind::Press,
        state: KeyEventState::NONE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip() {
        for n in [
            "ctrl+s",
            "f2",
            "escape",
            "enter",
            "shift+tab",
            "a",
            "A",
            "down",
            "pageup",
            "ctrl+left",
        ] {
            assert_eq!(key_name(&parse_key(n)), n, "{n}");
        }
        assert_eq!(key_name(&parse_key("space")), " ");
    }
}
