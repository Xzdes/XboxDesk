//! Виртуальные коды клавиш Windows ↔ человекочитаемые имена.
//!
//! Без зависимостей: таблица имен и парсер комбинаций вроде `CTRL+SHIFT+T`.
//! Используется и конфигом (текст → VK), и GUI (захват клавиатуры → текст).

/// Основная таблица «имя → VK». Имена регистронезависимые при разборе.
static NAMES: &[(&str, u16)] = &[
    ("BACKSPACE", 0x08),
    ("BACK", 0x08),
    ("TAB", 0x09),
    ("ENTER", 0x0D),
    ("RETURN", 0x0D),
    ("SHIFT", 0x10),
    ("LSHIFT", 0xA0),
    ("RSHIFT", 0xA1),
    ("CTRL", 0x11),
    ("CONTROL", 0x11),
    ("LCTRL", 0xA2),
    ("RCTRL", 0xA3),
    ("ALT", 0x12),
    ("LALT", 0xA4),
    ("RALT", 0xA5),
    ("PAUSE", 0x13),
    ("CAPSLOCK", 0x14),
    ("ESC", 0x1B),
    ("ESCAPE", 0x1B),
    ("SPACE", 0x20),
    ("PGUP", 0x21),
    ("PAGEUP", 0x21),
    ("PGDN", 0x22),
    ("PAGEDOWN", 0x22),
    ("END", 0x23),
    ("HOME", 0x24),
    ("LEFT", 0x25),
    ("UP", 0x26),
    ("RIGHT", 0x27),
    ("DOWN", 0x28),
    ("PRINTSCREEN", 0x2C),
    ("PRTSC", 0x2C),
    ("INSERT", 0x2D),
    ("INS", 0x2D),
    ("DELETE", 0x2E),
    ("DEL", 0x2E),
    ("WIN", 0x5B),
    ("LWIN", 0x5B),
    ("RWIN", 0x5C),
    ("APPS", 0x5D),
    ("MENU", 0x5D),
    ("NUMMUL", 0x6A),
    ("NUMADD", 0x6B),
    ("NUMSUB", 0x6D),
    ("NUMDEC", 0x6E),
    ("NUMDIV", 0x6F),
    ("NUMLOCK", 0x90),
    ("SCROLLLOCK", 0x91),
    ("SEMICOLON", 0xBA),
    ("EQUALS", 0xBB),
    ("COMMA", 0xBC),
    ("MINUS", 0xBD),
    ("PERIOD", 0xBE),
    ("SLASH", 0xBF),
    ("GRAVE", 0xC0),
    ("TILDE", 0xC0),
    ("LBRACKET", 0xDB),
    ("BACKSLASH", 0xDC),
    ("RBRACKET", 0xDD),
    ("APOSTROPHE", 0xDE),
    ("QUOTE", 0xDE),
    ("VOLUME_MUTE", 0xAD),
    ("VOLUME_DOWN", 0xAE),
    ("VOLUME_UP", 0xAF),
    ("MEDIA_NEXT", 0xB0),
    ("MEDIA_PREV", 0xB1),
    ("MEDIA_STOP", 0xB2),
    ("MEDIA_PLAY_PAUSE", 0xB3),
    ("MUTE", 0xAD),
];

/// Имя для VK: сначала таблица, затем диапазоны (буквы/цифры/F-клавиши/цифровая
/// клавиатура), иначе `VK_xx`.
pub fn vk_to_name(vk: u16) -> String {
    // Приоритет у канонических имен: ищем первое совпадение в таблице.
    for &(name, code) in NAMES {
        if code == vk {
            return name.to_string();
        }
    }
    if (0x30..=0x39).contains(&vk) {
        // VK-коды цифр совпадают с ASCII '0'..='9'.
        return (vk as u8 as char).to_string();
    }
    if (0x41..=0x5A).contains(&vk) {
        // VK-коды букв совпадают с ASCII 'A'..='Z'.
        return (vk as u8 as char).to_string();
    }
    if (0x70..=0x87).contains(&vk) {
        return format!("F{}", vk - 0x70 + 1);
    }
    if (0x60..=0x69).contains(&vk) {
        return format!("NUMPAD{}", vk - 0x60);
    }
    format!("VK_{vk:02X}")
}

/// VK по имени: таблица (регистронезависимо), одиночные символы `A`–`Z`/`0`–`9`,
/// `F1`–`F24`, `NUMPAD0`–`NUMPAD9`.
pub fn name_to_vk(name: &str) -> Option<u16> {
    let upper = name.trim().to_ascii_uppercase();
    if upper.is_empty() {
        return None;
    }
    for &(n, code) in NAMES {
        if n == upper {
            return Some(code);
        }
    }
    let bytes = upper.as_bytes();
    if bytes.len() == 1 {
        let c = bytes[0];
        if c.is_ascii_uppercase() {
            return Some(0x41 + (c - b'A') as u16);
        }
        if c.is_ascii_digit() {
            return Some(0x30 + (c - b'0') as u16);
        }
    }
    if let Some(rest) = upper.strip_prefix('F') {
        if let Ok(n) = rest.parse::<u16>() {
            if (1..=24).contains(&n) {
                return Some(0x70 + n - 1);
            }
        }
    }
    if let Some(rest) = upper.strip_prefix("NUMPAD") {
        if let Ok(n) = rest.parse::<u16>() {
            if n <= 9 {
                return Some(0x60 + n);
            }
        }
    }
    None
}

/// Разбор комбинации `CTRL+SHIFT+T` → список VK (модификаторы первыми).
pub fn parse_combo(text: &str) -> Option<Vec<u16>> {
    let mut out = Vec::new();
    for part in text.split('+') {
        let vk = name_to_vk(part)?;
        if !out.contains(&vk) {
            out.push(vk);
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

/// Обратная операция: список VK → `CTRL+SHIFT+T`.
pub fn format_combo(keys: &[u16]) -> String {
    keys.iter()
        .map(|vk| vk_to_name(*vk))
        .collect::<Vec<_>>()
        .join("+")
}

/// Это «чистый» модификатор (Shift/Ctrl/Alt/Win)? Такие клавиши в захвате
/// клавиатуры не фиксируются сами по себе — ждем основную клавишу.
pub fn is_modifier(vk: u16) -> bool {
    matches!(vk, 0x10 | 0x11 | 0x12 | 0x5B | 0x5C | 0xA0..=0xA5)
}

/// Стандартные модификаторы в порядке Ctrl, Alt, Shift, Win — для захвата.
pub fn modifier_order() -> [(u16, &'static str); 4] {
    [
        (0x11, "CTRL"),
        (0x12, "ALT"),
        (0x10, "SHIFT"),
        (0x5B, "WIN"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_combo() {
        let combo = parse_combo("CTRL+SHIFT+T").expect("parse");
        assert_eq!(combo, vec![0x11, 0x10, 0x54]);
        assert_eq!(format_combo(&combo), "CTRL+SHIFT+T");
    }

    #[test]
    fn letters_digits_fkeys() {
        assert_eq!(name_to_vk("a"), Some(0x41));
        assert_eq!(name_to_vk("7"), Some(0x37));
        assert_eq!(name_to_vk("f12"), Some(0x7B));
        assert_eq!(name_to_vk("F24"), Some(0x87));
        assert_eq!(name_to_vk("numpad3"), Some(0x63));
        assert_eq!(name_to_vk("PAGEUP"), Some(0x21));
        assert_eq!(name_to_vk("НОЛЬ"), None);
    }

    #[test]
    fn names_back() {
        assert_eq!(vk_to_name(0x41), "A");
        assert_eq!(vk_to_name(0x20), "SPACE");
        assert_eq!(vk_to_name(0x70), "F1");
        assert_eq!(vk_to_name(0x65), "NUMPAD5");
        assert_eq!(vk_to_name(0xAF), "VOLUME_UP");
    }

    #[test]
    fn modifier_detection() {
        assert!(is_modifier(0xA2));
        assert!(is_modifier(0x10));
        assert!(!is_modifier(0x41));
    }
}
