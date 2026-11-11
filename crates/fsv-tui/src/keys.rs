//! crossterm key events ↔ Textual key names (`"slash"`, `"ctrl+p"`, `"G"`),
//! so the key map and test scripts read like the Python handler.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

fn char_name(c: char) -> String {
    match c {
        '/' => "slash".into(),
        '?' => "question_mark".into(),
        '[' => "left_square_bracket".into(),
        ']' => "right_square_bracket".into(),
        '{' => "left_curly_bracket".into(),
        '}' => "right_curly_bracket".into(),
        '!' => "exclamation_mark".into(),
        ' ' => "space".into(),
        c => c.to_string(),
    }
}

fn name_char(name: &str) -> Option<char> {
    Some(match name {
        "slash" => '/',
        "question_mark" => '?',
        "left_square_bracket" => '[',
        "right_square_bracket" => ']',
        "left_curly_bracket" => '{',
        "right_curly_bracket" => '}',
        "exclamation_mark" => '!',
        "space" => ' ',
        _ => {
            let mut it = name.chars();
            let c = it.next()?;
            if it.next().is_some() {
                return None;
            }
            c
        }
    })
}

/// Textual name of a key press.
pub fn name(ev: &KeyEvent) -> String {
    let ctrl = ev.modifiers.contains(KeyModifiers::CONTROL);
    let base = match ev.code {
        KeyCode::Char(c) if ctrl => return format!("ctrl+{}", c.to_ascii_lowercase()),
        KeyCode::Char(c) => return char_name(c),
        KeyCode::Enter => "enter".to_string(),
        KeyCode::Esc => "escape".to_string(),
        KeyCode::Tab => "tab".to_string(),
        KeyCode::BackTab => return "shift+tab".to_string(),
        KeyCode::Backspace => "backspace".to_string(),
        KeyCode::Delete => "delete".to_string(),
        KeyCode::Up => "up".to_string(),
        KeyCode::Down => "down".to_string(),
        KeyCode::Left => "left".to_string(),
        KeyCode::Right => "right".to_string(),
        KeyCode::Home => "home".to_string(),
        KeyCode::End => "end".to_string(),
        KeyCode::PageUp => "pageup".to_string(),
        KeyCode::PageDown => "pagedown".to_string(),
        KeyCode::F(n) => format!("f{n}"),
        _ => return String::new(),
    };
    if ctrl { format!("ctrl+{base}") } else { base }
}

/// The printable character a key types into an input, if any.
pub fn printable(ev: &KeyEvent) -> Option<char> {
    if ev
        .modifiers
        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
    {
        return None;
    }
    match ev.code {
        KeyCode::Char(c) => Some(c),
        _ => None,
    }
}

/// Build a key event from a Textual name (test scripts, `press("ctrl+p")`).
pub fn parse(name: &str) -> KeyEvent {
    let (mods, rest) = match name.strip_prefix("ctrl+") {
        Some(r) => (KeyModifiers::CONTROL, r),
        None => (KeyModifiers::NONE, name),
    };
    let code = match rest {
        "enter" => KeyCode::Enter,
        "escape" => KeyCode::Esc,
        "tab" => KeyCode::Tab,
        "backspace" => KeyCode::Backspace,
        "delete" => KeyCode::Delete,
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "pageup" => KeyCode::PageUp,
        "pagedown" => KeyCode::PageDown,
        "shift+tab" => KeyCode::BackTab,
        f if f.len() > 1 && f.starts_with('f') && f[1..].parse::<u8>().is_ok() => {
            KeyCode::F(f[1..].parse().unwrap_or(1))
        }
        other => KeyCode::Char(name_char(other).unwrap_or(' ')),
    };
    let mods = if let KeyCode::Char(c) = code {
        if c.is_ascii_uppercase() {
            mods | KeyModifiers::SHIFT
        } else {
            mods
        }
    } else {
        mods
    };
    KeyEvent::new(code, mods)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_names() {
        for n in [
            "slash",
            "question_mark",
            "ctrl+p",
            "ctrl+n",
            "G",
            "g",
            "escape",
            "tab",
            "1",
            "left_square_bracket",
            "exclamation_mark",
            "f3",
        ] {
            assert_eq!(name(&parse(n)), n);
        }
    }
}
