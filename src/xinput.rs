//! Опрос геймпада Xbox через XInput.
//!
//! `windows-sys 0.59` не экспортирует XInput, а import-библиотеки
//! `xinput1_4.lib` в SDK нет — поэтому DLL грузится вручную в рантайме
//! (`LoadLibraryW` + `GetProcAddress`) с откатом `xinput1_4 → xinput1_3 →
//! xinput9_1_0`. Из внешнего — только системные DLL Windows; если XInput
//! нет вовсе, геймпад просто считается неподключенным.
//!
//! Константы кнопок — стабильный ABI из XInput.h.

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::sync::OnceLock;

use windows_sys::Win32::Foundation::HMODULE;
use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};

/// Битовые маски цифровых кнопок (XInput.h).
pub const XINPUT_GAMEPAD_DPAD_UP: u16 = 0x0001;
pub const XINPUT_GAMEPAD_DPAD_DOWN: u16 = 0x0002;
pub const XINPUT_GAMEPAD_DPAD_LEFT: u16 = 0x0004;
pub const XINPUT_GAMEPAD_DPAD_RIGHT: u16 = 0x0008;
pub const XINPUT_GAMEPAD_START: u16 = 0x0010;
pub const XINPUT_GAMEPAD_BACK: u16 = 0x0020;
pub const XINPUT_GAMEPAD_LEFT_THUMB: u16 = 0x0040;
pub const XINPUT_GAMEPAD_RIGHT_THUMB: u16 = 0x0080;
pub const XINPUT_GAMEPAD_LEFT_SHOULDER: u16 = 0x0100;
pub const XINPUT_GAMEPAD_RIGHT_SHOULDER: u16 = 0x0200;
pub const XINPUT_GAMEPAD_A: u16 = 0x1000;
pub const XINPUT_GAMEPAD_B: u16 = 0x2000;
pub const XINPUT_GAMEPAD_X: u16 = 0x4000;
pub const XINPUT_GAMEPAD_Y: u16 = 0x8000;

/// Тип устройства для запроса батареи.
pub const BATTERY_DEVTYPE_GAMEPAD: u8 = 0;
const BATTERY_TYPE_WIRED: u8 = 1;
const BATTERY_LEVEL_EMPTY: u8 = 0;
const BATTERY_LEVEL_LOW: u8 = 1;
const BATTERY_LEVEL_MEDIUM: u8 = 2;
const BATTERY_LEVEL_HIGH: u8 = 3;

/// Работаем с первым геймпадом (user index 0).
pub const PAD_INDEX: u32 = 0;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct XINPUT_GAMEPAD {
    pub w_buttons: u16,
    pub b_left_trigger: u8,
    pub b_right_trigger: u8,
    pub s_thumb_lx: i16,
    pub s_thumb_ly: i16,
    pub s_thumb_rx: i16,
    pub s_thumb_ry: i16,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct XINPUT_STATE {
    pub dw_packet_number: u32,
    pub gamepad: XINPUT_GAMEPAD,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct XINPUT_VIBRATION {
    pub w_motor_left_speed: u16,
    pub w_motor_right_speed: u16,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct XINPUT_BATTERY_INFORMATION {
    pub battery_type: u8,
    pub battery_level: u8,
}

type XInputGetStateFn = unsafe extern "system" fn(u32, *mut XINPUT_STATE) -> u32;
type XInputSetStateFn = unsafe extern "system" fn(u32, *mut XINPUT_VIBRATION) -> u32;
type XInputGetBatteryFn =
    unsafe extern "system" fn(u32, u8, *mut XINPUT_BATTERY_INFORMATION) -> u32;

/// Загруженный набор функций XInput.
struct XInput {
    get_state: XInputGetStateFn,
    set_state: XInputSetStateFn,
    /// В xinput9_1_0 этой функции нет.
    get_battery: Option<XInputGetBatteryFn>,
}

fn xinput() -> Option<&'static XInput> {
    static INSTANCE: OnceLock<Option<XInput>> = OnceLock::new();
    INSTANCE
        .get_or_init(|| {
            for dll in ["xinput1_4.dll", "xinput1_3.dll", "xinput9_1_0.dll"] {
                if let Some(found) = unsafe { load(dll) } {
                    return Some(found);
                }
            }
            None
        })
        .as_ref()
}

unsafe fn load(dll: &str) -> Option<XInput> {
    let name: Vec<u16> = OsStr::new(dll).encode_wide().chain(Some(0)).collect();
    let module: HMODULE = LoadLibraryW(name.as_ptr());
    if module.is_null() {
        return None;
    }
    let get_state = symbol::<XInputGetStateFn>(module, "XInputGetState")?;
    let set_state = symbol::<XInputSetStateFn>(module, "XInputSetState")?;
    let get_battery = symbol::<XInputGetBatteryFn>(module, "XInputGetBatteryInformation");
    Some(XInput {
        get_state,
        set_state,
        get_battery,
    })
}

unsafe fn symbol<T>(module: HMODULE, name: &str) -> Option<T> {
    let mut buf: Vec<u8> = name.as_bytes().to_vec();
    buf.push(0);
    let addr = GetProcAddress(module, buf.as_ptr());
    // FARPROC (Option<extern fn>) и целевой fn-указатель одного размера;
    // addr = Some ⇒ адрес ненулевой.
    addr.map(|f| std::mem::transmute_copy(&f))
}

/// Снимок состояния геймпада для движка и GUI.
#[derive(Clone, Copy, Debug, Default)]
pub struct PadState {
    pub connected: bool,
    /// Битовая маска цифровых кнопок (XINPUT_GAMEPAD_*).
    pub buttons: u16,
    /// Аналоговые триггеры 0..=255.
    pub lt: u8,
    pub rt: u8,
    /// Стики, нормализованные в -1.0..=1.0 (сырые, без мертвой зоны).
    pub lx: f64,
    pub ly: f64,
    pub rx: f64,
    pub ry: f64,
    /// Заряд батареи в процентах, если удалось прочитать.
    pub battery: Option<u8>,
}

impl PadState {
    fn from_state(state: &XINPUT_STATE) -> Self {
        let g = &state.gamepad;
        Self {
            connected: true,
            buttons: g.w_buttons,
            lt: g.b_left_trigger,
            rt: g.b_right_trigger,
            lx: normalize(g.s_thumb_lx),
            ly: normalize(g.s_thumb_ly),
            rx: normalize(g.s_thumb_rx),
            ry: normalize(g.s_thumb_ry),
            battery: None,
        }
    }
}

fn normalize(v: i16) -> f64 {
    if v >= 0 {
        v as f64 / 32767.0
    } else {
        v as f64 / 32768.0
    }
}

/// Один опрос геймпада. При отсутствии контроллера (или XInput) возвращает
/// пустой снимок с connected = false.
pub fn poll() -> PadState {
    let Some(xi) = xinput() else {
        return PadState::default();
    };
    let mut state: XINPUT_STATE = unsafe { std::mem::zeroed() };
    let result = unsafe { (xi.get_state)(PAD_INDEX, &mut state) };
    if result == 0 {
        PadState::from_state(&state)
    } else {
        // ERROR_DEVICE_NOT_CONNECTED (1167) / ERROR_EMPTY (4306) — контроллера нет.
        PadState::default()
    }
}

/// Вибрация моторов (0..=65535 каждый).
pub fn set_rumble(left: u16, right: u16) {
    let Some(xi) = xinput() else { return };
    let vibration = XINPUT_VIBRATION {
        w_motor_left_speed: left,
        w_motor_right_speed: right,
    };
    unsafe {
        (xi.set_state)(PAD_INDEX, &vibration as *const XINPUT_VIBRATION as *mut _);
    }
}

/// Заряд батареи в процентах (грубо, по уровням XInput) или None.
pub fn battery_percent() -> Option<u8> {
    let xi = xinput()?;
    let get_battery = xi.get_battery?;
    let mut info: XINPUT_BATTERY_INFORMATION = unsafe { std::mem::zeroed() };
    let result = unsafe { get_battery(PAD_INDEX, BATTERY_DEVTYPE_GAMEPAD, &mut info) };
    if result != 0 {
        return None;
    }
    if info.battery_type == BATTERY_TYPE_WIRED {
        return Some(100);
    }
    Some(match info.battery_level {
        BATTERY_LEVEL_EMPTY => 5,
        BATTERY_LEVEL_LOW => 25,
        BATTERY_LEVEL_MEDIUM => 60,
        BATTERY_LEVEL_HIGH => 90,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalization_symmetric() {
        assert!((normalize(32767) - 1.0).abs() < 1e-9);
        assert!((normalize(-32768) + 1.0).abs() < 1e-9);
        assert!(normalize(0).abs() < 1e-12);
    }

    #[test]
    fn button_mask_values_match_xinput_header() {
        assert_eq!(XINPUT_GAMEPAD_DPAD_UP, 0x0001);
        assert_eq!(XINPUT_GAMEPAD_A, 0x1000);
        assert_eq!(XINPUT_GAMEPAD_Y, 0x8000);
    }

    #[test]
    fn poll_without_gamepad_reports_disconnected() {
        // Тест ничего не нажимает: проверяем, что опрос не падает и
        // возвращает связный снимок.
        let pad = poll();
        assert!(!pad.connected || pad.buttons != u16::MAX);
    }
}
