//! Эмуляция клавиатуры и мыши через Win32 `SendInput` (windows-sys).
//!
//! Инъектор помнит все «зажатые» клавиши и кнопки мыши, чтобы при остановке
//! или закрытии терминала гарантированно их отпустить (иначе клавиша залипнет
//! на уровне всей системы).

use std::collections::HashSet;
use std::thread::sleep;
use std::time::Duration;

use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYEVENTF_KEYUP,
    KEYEVENTF_UNICODE, MOUSEEVENTF_HWHEEL, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP,
    MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN,
    MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_WHEEL, MOUSEEVENTF_XDOWN, MOUSEEVENTF_XUP, MOUSEINPUT,
};

/// Константы winuser.h, которых нет в windows-sys (стабильный ABI).
const WHEEL_DELTA: i32 = 120;
const XBUTTON1: u32 = 1;
const XBUTTON2: u32 = 2;

/// Магическая метка в dwExtraInfo — «эти события послал XboxDesk».
const EXTRA_INFO: usize = 0x0058_4244_5300; // "XBDS"

/// Кнопки мыши, которые можно назначить на геймпад.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    X1,
    X2,
}

impl MouseButton {
    pub fn parse(token: &str) -> Option<Self> {
        Some(match token.trim().to_ascii_lowercase().as_str() {
            "left" | "l" => Self::Left,
            "right" | "r" => Self::Right,
            "middle" | "m" => Self::Middle,
            "x1" | "back" => Self::X1,
            "x2" | "forward" => Self::X2,
            _ => return None,
        })
    }

    pub fn token(&self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Right => "right",
            Self::Middle => "middle",
            Self::X1 => "x1",
            Self::X2 => "x2",
        }
    }

    /// Русское имя для интерфейса.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Left => "ЛКМ",
            Self::Right => "ПКМ",
            Self::Middle => "СКМ",
            Self::X1 => "X1 (назад)",
            Self::X2 => "X2 (вперед)",
        }
    }

    fn down_up_flags(&self) -> (u32, u32, u32) {
        // (down-флаг, up-флаг, mouseData)
        match self {
            Self::Left => (MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, 0),
            Self::Right => (MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, 0),
            Self::Middle => (MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP, 0),
            Self::X1 => (MOUSEEVENTF_XDOWN, MOUSEEVENTF_XUP, XBUTTON1),
            Self::X2 => (MOUSEEVENTF_XDOWN, MOUSEEVENTF_XUP, XBUTTON2),
        }
    }
}

/// Посылает системные события ввода и следит за «зажатым» состоянием.
#[derive(Default)]
pub struct Injector {
    keys_down: HashSet<u16>,
    mouse_down: HashSet<MouseButton>,
}

fn key_input(vk: u16, up: bool) -> INPUT {
    let mut input: INPUT = unsafe { std::mem::zeroed() };
    input.r#type = INPUT_KEYBOARD;
    input.Anonymous = INPUT_0 {
        ki: KEYBDINPUT {
            wVk: vk,
            wScan: 0,
            dwFlags: if up { KEYEVENTF_KEYUP } else { 0 },
            time: 0,
            dwExtraInfo: EXTRA_INFO,
        },
    };
    input
}

fn unicode_input(unit: u16, up: bool) -> INPUT {
    let mut input: INPUT = unsafe { std::mem::zeroed() };
    input.r#type = INPUT_KEYBOARD;
    input.Anonymous = INPUT_0 {
        ki: KEYBDINPUT {
            wVk: 0,
            wScan: unit,
            dwFlags: KEYEVENTF_UNICODE | if up { KEYEVENTF_KEYUP } else { 0 },
            time: 0,
            dwExtraInfo: EXTRA_INFO,
        },
    };
    input
}

fn mouse_input(flags: u32, dx: i32, dy: i32, data: u32) -> INPUT {
    let mut input: INPUT = unsafe { std::mem::zeroed() };
    input.r#type = INPUT_MOUSE;
    input.Anonymous = INPUT_0 {
        mi: MOUSEINPUT {
            dx,
            dy,
            mouseData: data,
            dwFlags: flags,
            time: 0,
            dwExtraInfo: EXTRA_INFO,
        },
    };
    input
}

fn send(inputs: &[INPUT]) {
    if inputs.is_empty() {
        return;
    }
    unsafe {
        SendInput(
            inputs.len() as u32,
            inputs.as_ptr(),
            std::mem::size_of::<INPUT>() as i32,
        );
    }
}

impl Injector {
    pub fn new() -> Self {
        Self::default()
    }

    /// Нажать и отпустить комбинацию (модификаторы держим вокруг основной
    /// клавиши). Порядок: все down → пауза → все up в обратном порядке.
    pub fn tap_keys(&mut self, combo: &[u16]) {
        if combo.is_empty() {
            return;
        }
        let downs: Vec<INPUT> = combo.iter().map(|&vk| key_input(vk, false)).collect();
        let ups: Vec<INPUT> = combo.iter().rev().map(|&vk| key_input(vk, true)).collect();
        send(&downs);
        sleep(Duration::from_millis(12));
        send(&ups);
    }

    /// Ввести текст посимвольно (Unicode-события, раскладка не важна).
    pub fn tap_text(&mut self, text: &str) {
        for unit in text.encode_utf16() {
            send(&[unicode_input(unit, false), unicode_input(unit, true)]);
            sleep(Duration::from_millis(4));
        }
    }

    /// Клик кнопкой мыши.
    pub fn click_mouse(&mut self, button: MouseButton) {
        let (down, up, data) = button.down_up_flags();
        send(&[mouse_input(down, 0, 0, data)]);
        sleep(Duration::from_millis(12));
        send(&[mouse_input(up, 0, 0, data)]);
    }

    /// Относительное перемещение курсора.
    pub fn move_mouse(&mut self, dx: i32, dy: i32) {
        if dx == 0 && dy == 0 {
            return;
        }
        send(&[mouse_input(MOUSEEVENTF_MOVE, dx, dy, 0)]);
    }

    /// Прокрутка колеса: notches > 0 — вверх/от себя.
    pub fn wheel(&mut self, notches: i32, horizontal: bool) {
        if notches == 0 {
            return;
        }
        let flags = if horizontal {
            MOUSEEVENTF_HWHEEL
        } else {
            MOUSEEVENTF_WHEEL
        };
        send(&[mouse_input(flags, 0, 0, (notches * WHEEL_DELTA) as u32)]);
    }

    /// Зажать/отпустить клавишу (стрелки стика и т.п.). Держим состояние,
    /// чтобы повторные вызовы не дублировали события.
    pub fn set_key(&mut self, vk: u16, down: bool) {
        let is_down = self.keys_down.contains(&vk);
        if down == is_down {
            return;
        }
        if down {
            self.keys_down.insert(vk);
        } else {
            self.keys_down.remove(&vk);
        }
        send(&[key_input(vk, !down)]);
    }

    /// Отпустить всё, что удерживается, — вызывается при остановке движка,
    /// закрытии окна и закрытии терминала.
    pub fn release_all(&mut self) {
        for &vk in &self.keys_down.clone() {
            send(&[key_input(vk, true)]);
        }
        for &button in &self.mouse_down.clone() {
            let (_, up, data) = button.down_up_flags();
            send(&[mouse_input(up, 0, 0, data)]);
        }
        self.keys_down.clear();
        self.mouse_down.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mouse_button_tokens() {
        assert_eq!(MouseButton::parse("LEFT"), Some(MouseButton::Left));
        assert_eq!(MouseButton::parse("x2"), Some(MouseButton::X2));
        assert_eq!(MouseButton::parse("wheel"), None);
        assert_eq!(MouseButton::Middle.token(), "middle");
    }
}
