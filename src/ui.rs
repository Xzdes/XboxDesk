//! Общие примитивы интерфейса в стиле alibi-quic: карточки со скруглениями,
//! owner-draw кнопки, Segoe UI, DPI-масштабирование, русский текст.

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;

use windows_sys::Win32::Foundation::{HWND, RECT};
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

pub const fn rgb(r: u32, g: u32, b: u32) -> u32 {
    r | (g << 8) | (b << 16)
}

// Deep-navy gaming palette: calm surfaces, bright focus accents.
pub const BACKGROUND: u32 = rgb(10, 16, 32);
pub const WHITE: u32 = rgb(17, 24, 39); // panel surface (kept for existing drawing API)
pub const INK: u32 = rgb(229, 231, 235);
pub const MUTED: u32 = rgb(148, 163, 184);
pub const BORDER: u32 = rgb(38, 52, 73);
pub const ROW_ALT: u32 = rgb(20, 30, 48);
pub const BLUE: u32 = rgb(96, 165, 250);
pub const BLUE_DARK: u32 = rgb(37, 99, 235);
pub const PALE_BLUE: u32 = rgb(24, 48, 82);
pub const GREEN: u32 = rgb(74, 222, 128);
pub const PALE_GREEN: u32 = rgb(17, 55, 48);
pub const AMBER: u32 = rgb(251, 191, 36);
pub const PALE_AMBER: u32 = rgb(67, 50, 18);
pub const RED: u32 = rgb(248, 113, 113);
pub const PALE_RED: u32 = rgb(69, 29, 39);
pub const SHADOW: u32 = rgb(5, 9, 20);
pub const SLATE: u32 = rgb(71, 85, 105);
pub const SLATE_SOFT: u32 = rgb(51, 65, 85);

// Палитра «геймпада» для визуализации (dark-theme variant).
pub const PAD_BODY: u32 = rgb(30, 42, 68);
pub const PAD_EDGE: u32 = rgb(10, 17, 31);
pub const PAD_DARK: u32 = rgb(8, 14, 27);
pub const PAD_CONTROL: u32 = rgb(52, 68, 96);
pub const PAD_LIT: u32 = rgb(0, 120, 215);
pub const PAD_KNOB: u32 = rgb(180, 182, 190);
pub const XBOX_GREEN: u32 = rgb(16, 186, 106);
pub const XBOX_RED: u32 = rgb(230, 40, 40);
pub const XBOX_BLUE: u32 = rgb(0, 120, 215);
pub const XBOX_YELLOW: u32 = rgb(255, 193, 7);

// PlayStation-style UI accents
pub const FOCUS_RING: u32 = rgb(96, 165, 250); // bright blue focus ring
pub const APP_TITLE: &str = "XboxDesk";

pub fn wide(value: &str) -> Vec<u16> {
    OsStr::new(value).encode_wide().chain(Some(0)).collect()
}

pub unsafe fn make_font(size: i32, weight: i32, dpi: i32) -> HFONT {
    CreateFontW(
        -size * dpi / 96,
        0,
        0,
        0,
        weight,
        0,
        0,
        0,
        DEFAULT_CHARSET as u32,
        OUT_DEFAULT_PRECIS as u32,
        CLIP_DEFAULT_PRECIS as u32,
        CLEARTYPE_QUALITY as u32,
        DEFAULT_PITCH as u32,
        wide("Segoe UI").as_ptr(),
    )
}

pub struct Fonts {
    pub brand: HFONT,
    pub heading: HFONT,
    pub body: HFONT,
    pub small: HFONT,
    pub button: HFONT,
    pub log: HFONT,
}

impl Fonts {
    pub unsafe fn new(dpi: i32) -> Self {
        Self {
            brand: make_font(26, 700, dpi),
            heading: make_font(16, 600, dpi),
            body: make_font(15, 400, dpi),
            small: make_font(12, 400, dpi),
            button: make_font(14, 600, dpi),
            log: make_font(13, 400, dpi),
        }
    }
}

impl Drop for Fonts {
    fn drop(&mut self) {
        unsafe {
            for font in [
                self.brand,
                self.heading,
                self.body,
                self.small,
                self.button,
                self.log,
            ] {
                if !font.is_null() {
                    DeleteObject(font);
                }
            }
        }
    }
}

pub unsafe fn fill(dc: HDC, rect: &RECT, color: u32) {
    let brush = CreateSolidBrush(color);
    FillRect(dc, rect, brush);
    DeleteObject(brush);
}

pub unsafe fn rounded(dc: HDC, rect: RECT, radius: i32, color: u32, border: u32) {
    let brush = CreateSolidBrush(color);
    let pen = CreatePen(PS_SOLID, 1, border);
    let old_brush = SelectObject(dc, brush);
    let old_pen = SelectObject(dc, pen);
    RoundRect(
        dc,
        rect.left,
        rect.top,
        rect.right,
        rect.bottom,
        radius,
        radius,
    );
    SelectObject(dc, old_pen);
    SelectObject(dc, old_brush);
    DeleteObject(pen);
    DeleteObject(brush);
}

/// Карточка интерфейса: мягкая тень снизу-справа + белый скругленный лист.
pub unsafe fn card(dc: HDC, rect: RECT, radius: i32, shadow_offset: i32) {
    let shadow = RECT {
        left: rect.left + shadow_offset,
        top: rect.top + shadow_offset,
        right: rect.right + shadow_offset,
        bottom: rect.bottom + shadow_offset,
    };
    rounded(dc, shadow, radius, SHADOW, SHADOW);
    rounded(dc, rect, radius, WHITE, BORDER);
}

/// Заголовок секции: акцентная синяя планка + текст.
pub unsafe fn section_header(
    dc: HDC,
    title: &str,
    card_rect: RECT,
    fonts: &Fonts,
    dpi: i32,
    color: u32,
) {
    let p = |n: i32| n * dpi / 96;
    let bar = RECT {
        left: card_rect.left + p(20),
        top: card_rect.top + p(16),
        right: card_rect.left + p(24),
        bottom: card_rect.top + p(33),
    };
    rounded(dc, bar, p(2), color, color);
    let mut text = RECT {
        left: card_rect.left + p(32),
        top: card_rect.top + p(12),
        right: card_rect.right - p(20),
        bottom: card_rect.top + p(38),
    };
    draw_text(
        dc,
        title,
        &mut text,
        fonts.heading,
        INK,
        DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX | DT_END_ELLIPSIS,
    );
}

/// Круг/эллипс по ограничивающему прямоугольнику.
pub unsafe fn circle(dc: HDC, rect: RECT, color: u32, border: u32, border_width: i32) {
    let brush = CreateSolidBrush(color);
    let pen = CreatePen(PS_SOLID, border_width.max(1), border);
    let old_brush = SelectObject(dc, brush);
    let old_pen = SelectObject(dc, pen);
    Ellipse(dc, rect.left, rect.top, rect.right, rect.bottom);
    SelectObject(dc, old_pen);
    SelectObject(dc, old_brush);
    DeleteObject(pen);
    DeleteObject(brush);
}

/// Только контур окружности (без закраски) — кольцо мертвой зоны и т.п.
pub unsafe fn ring(dc: HDC, rect: RECT, color: u32, width: i32) {
    let pen = CreatePen(PS_SOLID, width.max(1), color);
    let old_pen = SelectObject(dc, pen);
    let old_brush = SelectObject(dc, GetStockObject(NULL_BRUSH));
    Ellipse(dc, rect.left, rect.top, rect.right, rect.bottom);
    SelectObject(dc, old_brush);
    SelectObject(dc, old_pen);
    DeleteObject(pen);
}

/// Часть составного силуэта (региона).
pub enum Shape {
    Round(RECT, i32),
}

/// Силуэт из объединения нескольких фигур: одна заливка + общая рамка.
/// Используется для корпуса геймпада (прямоугольник + две рукоятки).
pub unsafe fn silhouette(dc: HDC, shapes: &[Shape], color: u32, border: u32, border_width: i32) {
    let Some((first, rest)) = shapes.split_first() else {
        return;
    };
    let region: HRGN = match first {
        Shape::Round(r, rad) => CreateRoundRectRgn(r.left, r.top, r.right, r.bottom, *rad, *rad),
    };
    if region.is_null() {
        return;
    }
    for shape in rest {
        let part = match shape {
            Shape::Round(r, rad) => {
                CreateRoundRectRgn(r.left, r.top, r.right, r.bottom, *rad, *rad)
            }
        };
        if !part.is_null() {
            CombineRgn(region, region, part, RGN_OR);
            DeleteObject(part);
        }
    }
    let brush = CreateSolidBrush(color);
    FillRgn(dc, region, brush);
    DeleteObject(brush);
    let frame_brush = CreateSolidBrush(border);
    FrameRgn(dc, region, frame_brush, border_width, border_width);
    DeleteObject(frame_brush);
    DeleteObject(region);
}

pub unsafe fn draw_text(dc: HDC, text: &str, rect: &mut RECT, font: HFONT, color: u32, flags: u32) {
    let old = SelectObject(dc, font);
    SetBkMode(dc, TRANSPARENT as i32);
    SetTextColor(dc, color);
    let mut text = wide(text);
    DrawTextW(
        dc,
        text.as_mut_ptr(),
        -1,
        rect,
        flags | DT_NOPREFIX | DT_END_ELLIPSIS,
    );
    SelectObject(dc, old);
}

pub unsafe fn set_text(hwnd: HWND, text: &str) {
    SetWindowTextW(hwnd, wide(text).as_ptr());
}

pub unsafe fn get_text(hwnd: HWND) -> String {
    let len = GetWindowTextLengthW(hwnd);
    if len <= 0 {
        return String::new();
    }
    let mut buf = vec![0u16; len as usize + 1];
    let read = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
    String::from_utf16_lossy(&buf[..read.max(0) as usize])
}

/// Стиль owner-draw кнопки.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ButtonStyle {
    Primary,
    Normal,
    Danger,
}

/// Отрисовка кнопки в стиле alibi-quic (по DRAWITEMSTRUCT-состояниям).
#[allow(clippy::too_many_arguments)]
pub unsafe fn paint_owner_button(
    dc: HDC,
    rc: RECT,
    text: &str,
    fonts: &Fonts,
    dpi: i32,
    style: ButtonStyle,
    pressed: bool,
    disabled: bool,
    focused: bool,
) {
    let p = |n: i32| n * dpi / 96;
    let (background, border, foreground) = if disabled {
        (ROW_ALT, BORDER, MUTED)
    } else {
        match style {
            ButtonStyle::Primary => {
                if pressed {
                    (BLUE_DARK, BLUE_DARK, WHITE)
                } else {
                    (BLUE, BLUE, WHITE)
                }
            }
            ButtonStyle::Danger => {
                if pressed {
                    (RED, RED, WHITE)
                } else {
                    (PALE_RED, RED, RED)
                }
            }
            ButtonStyle::Normal => {
                if pressed {
                    (PALE_BLUE, BLUE, INK)
                } else {
                    (WHITE, BORDER, INK)
                }
            }
        }
    };
    fill(dc, &rc, WHITE);
    rounded(dc, rc, p(10), background, border);
    let mut text_rect = rc;
    if pressed {
        text_rect.top += p(1);
    }
    draw_text(
        dc,
        text,
        &mut text_rect,
        fonts.button,
        foreground,
        DT_CENTER | DT_VCENTER | DT_SINGLELINE,
    );
    if focused && !disabled {
        let mut focus = rc;
        InflateRect(&mut focus, -p(3), -p(3));
        // High-contrast double ring, intentionally visible on dark surfaces.
        ring(dc, focus, FOCUS_RING, p(2).max(1));
        InflateRect(&mut focus, -p(3), -p(3));
        ring(dc, focus, rgb(10, 16, 32), p(1).max(1));
    }
}

pub fn show_message(owner: Option<HWND>, body: &str) {
    unsafe {
        MessageBoxW(
            owner.unwrap_or(std::ptr::null_mut()),
            wide(body).as_ptr(),
            wide(APP_TITLE).as_ptr(),
            MB_OK | MB_ICONINFORMATION,
        );
    }
}

pub fn confirm(owner: Option<HWND>, body: &str) -> bool {
    unsafe {
        MessageBoxW(
            owner.unwrap_or(std::ptr::null_mut()),
            wide(body).as_ptr(),
            wide(APP_TITLE).as_ptr(),
            MB_YESNO | MB_ICONQUESTION,
        ) == IDYES
    }
}
