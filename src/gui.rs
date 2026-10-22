//! Главное окно XboxDesk в стиле alibi-quic: рукописный Win32 на windows-sys,
//! карточки со скруглениями, owner-draw кнопки, двойная буферизация, DPI.
//!
//! Окно живет в процессе с консолью: закрытие терминала останавливает всё.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::SystemInformation::GetLocalTime;
use windows_sys::Win32::UI::Controls::*;
use windows_sys::Win32::UI::HiDpi::*;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    EnableWindow, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT,
};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::binding::{default_bindings, Binding, GameButton};
use crate::config::{self, Config};
use crate::dialog;
use crate::engine::{self, Shared};
use crate::ui::*;
use crate::xinput::{
    PadState, XINPUT_GAMEPAD_A, XINPUT_GAMEPAD_B, XINPUT_GAMEPAD_BACK, XINPUT_GAMEPAD_DPAD_DOWN,
    XINPUT_GAMEPAD_DPAD_LEFT, XINPUT_GAMEPAD_DPAD_RIGHT, XINPUT_GAMEPAD_DPAD_UP,
    XINPUT_GAMEPAD_LEFT_SHOULDER, XINPUT_GAMEPAD_LEFT_THUMB, XINPUT_GAMEPAD_RIGHT_SHOULDER,
    XINPUT_GAMEPAD_RIGHT_THUMB, XINPUT_GAMEPAD_START, XINPUT_GAMEPAD_X, XINPUT_GAMEPAD_Y,
};

const TIMER_ID: usize = 1;
const TIMER_MS: u32 = 60;

const IDC_TOGGLE: u16 = 1001;
const IDC_SETTINGS: u16 = 1002;
const IDC_LIST: u16 = 1003;
const IDC_ADD: u16 = 1004;
const IDC_EDIT: u16 = 1005;
const IDC_DEL: u16 = 1006;
const IDC_CLEAR: u16 = 1008;
const IDC_LOG: u16 = 1009;

const LOG_LIMIT: usize = 240_000;

// Уведомления и состояния owner-draw (числовые значения стабильны).
const LBN_SELCHANGE_CODE: u16 = 1;
const LBN_DBLCLK_CODE: u16 = 2;
const ODT_LISTBOX_TYPE: u32 = 2;
const ODT_BUTTON_TYPE: u32 = 4;
const ODS_SELECTED_BIT: u32 = 0x0001;
const ODS_DISABLED_BIT: u32 = 0x0004;
const ODS_FOCUS_BIT: u32 = 0x0010;

// Стили контролов локально в u32 (как в alibi-quic): в windows-sys они i32.
const ES_MULTILINE: u32 = 0x0004;
const ES_AUTOVSCROLL: u32 = 0x0040;
const ES_READONLY: u32 = 0x0800;
const BS_OWNERDRAW: u32 = 0x000B;
const LBS_NOTIFY: u32 = 0x0001;
const LBS_OWNERDRAWFIXED: u32 = 0x1000;
const LBS_HASSTRINGS: u32 = 0x4000;

// Логические размеры (96 dpi), как в alibi-quic всё масштабируется через p().
const WIN_W: i32 = 1080;
const WIN_H: i32 = 820;
const MIN_W: i32 = 980;
const MIN_H: i32 = 720;

pub fn run(shared: Arc<Mutex<Shared>>) -> i32 {
    match unsafe { run_window(shared) } {
        Ok(code) => code,
        Err(error) => {
            show_message(None, &error);
            1
        }
    }
}

unsafe fn run_window(shared: Arc<Mutex<Shared>>) -> Result<i32, String> {
    SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    let instance = GetModuleHandleW(std::ptr::null());
    let class_name = wide("XboxDeskMainWindow");
    let wc = WNDCLASSW {
        lpfnWndProc: Some(wnd_proc),
        hInstance: instance,
        hCursor: LoadCursorW(std::ptr::null_mut(), IDC_ARROW),
        // MAKEINTRESOURCEW(1) — иконка приложения из ресурсов exe.
        hIcon: LoadIconW(instance, 1usize as *const u16),
        lpszClassName: class_name.as_ptr(),
        ..std::mem::zeroed()
    };
    if RegisterClassW(&wc) == 0 {
        return Err("Не удалось подготовить окно приложения.".into());
    }
    let mut state = Box::new(GuiState::new(shared));
    let mut work_area: RECT = std::mem::zeroed();
    SystemParametersInfoW(SPI_GETWORKAREA, 0, (&mut work_area as *mut RECT).cast(), 0);
    let window_width = state
        .p(WIN_W)
        .min((work_area.right - work_area.left - state.p(24)).max(state.p(MIN_W)));
    let window_height = state
        .p(WIN_H)
        .min((work_area.bottom - work_area.top - state.p(24)).max(state.p(MIN_H)));
    let title = wide("XboxDesk — геймпад как клавиатура и мышь");
    let hwnd = CreateWindowExW(
        0,
        class_name.as_ptr(),
        title.as_ptr(),
        WS_OVERLAPPEDWINDOW | WS_VISIBLE | WS_CLIPCHILDREN,
        work_area.left + ((work_area.right - work_area.left - window_width) / 2).max(0),
        work_area.top + ((work_area.bottom - work_area.top - window_height) / 2).max(0),
        window_width,
        window_height,
        std::ptr::null_mut(),
        std::ptr::null_mut(),
        instance,
        (&mut *state as *mut GuiState).cast(),
    );
    if hwnd.is_null() {
        UnregisterClassW(class_name.as_ptr(), instance);
        return Err("Не удалось открыть окно приложения.".into());
    }
    let mut msg: MSG = std::mem::zeroed();
    while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
        if IsDialogMessageW(hwnd, &msg) == 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    UnregisterClassW(class_name.as_ptr(), instance);
    Ok(msg.wParam as i32)
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_NCCREATE {
        let create = &*(lparam as *const CREATESTRUCTW);
        let state = create.lpCreateParams as *mut GuiState;
        if !state.is_null() {
            (*state).hwnd = hwnd;
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, state as isize);
        }
    }
    let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut GuiState;
    match msg {
        WM_CREATE => {
            if let Some(state) = state_ptr.as_mut() {
                if !state.create_controls() {
                    return -1;
                }
                state.initialize();
                state.layout();
                SetTimer(hwnd, TIMER_ID, TIMER_MS, None);
            }
            0
        }
        WM_SIZE => {
            if let Some(state) = state_ptr.as_mut() {
                state.layout();
            }
            0
        }
        WM_DPICHANGED => {
            if let Some(state) = state_ptr.as_mut() {
                state.dpi = (wparam & 0xffff) as i32;
                state.replace_fonts();
                let rect = &*(lparam as *const RECT);
                SetWindowPos(
                    hwnd,
                    std::ptr::null_mut(),
                    rect.left,
                    rect.top,
                    rect.right - rect.left,
                    rect.bottom - rect.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
                state.layout();
            }
            0
        }
        WM_GETMINMAXINFO => {
            let dpi = state_ptr.as_ref().map(|s| s.dpi).unwrap_or(96);
            let limits = &mut *(lparam as *mut MINMAXINFO);
            limits.ptMinTrackSize.x = MIN_W * dpi / 96;
            limits.ptMinTrackSize.y = MIN_H * dpi / 96;
            0
        }
        WM_MEASUREITEM => {
            let measure = &mut *(lparam as *mut MEASUREITEMSTRUCT);
            if measure.CtlID == IDC_LIST as u32 {
                let dpi = state_ptr.as_ref().map(|s| s.dpi).unwrap_or(96);
                measure.itemHeight = (30 * dpi / 96) as u32;
                return 1;
            }
            0
        }
        WM_COMMAND => {
            if let Some(state) = state_ptr.as_mut() {
                let id = (wparam & 0xffff) as u16;
                let code = ((wparam >> 16) & 0xffff) as u16;
                if id == IDC_LIST {
                    match code {
                        LBN_SELCHANGE_CODE => state.on_list_selection_changed(),
                        LBN_DBLCLK_CODE => state.handle_command(IDC_EDIT),
                        _ => {}
                    }
                } else if code == 0 {
                    state.handle_command(id);
                }
            }
            0
        }
        WM_TIMER => {
            if let Some(state) = state_ptr.as_mut() {
                state.on_timer();
            }
            0
        }
        WM_MOUSEMOVE => {
            if let Some(state) = state_ptr.as_mut() {
                // A quiet, console-like cursor: a dot surrounded by a focus ring.
                state.cursor_x = (lparam as i16) as i32;
                state.cursor_y = ((lparam >> 16) as i16) as i32;
                state.cursor_visible = true;
                let mut track: TRACKMOUSEEVENT = std::mem::zeroed();
                track.cbSize = std::mem::size_of::<TRACKMOUSEEVENT>() as u32;
                track.dwFlags = TME_LEAVE;
                track.hwndTrack = hwnd;
                TrackMouseEvent(&mut track);
                InvalidateRect(hwnd, std::ptr::null(), 0);
            }
            0
        }
        WM_MOUSELEAVE => {
            if let Some(state) = state_ptr.as_mut() {
                state.cursor_visible = false;
                InvalidateRect(hwnd, std::ptr::null(), 0);
            }
            0
        }
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            if let Some(state) = state_ptr.as_ref() {
                state.paint();
            } else {
                return DefWindowProcW(hwnd, msg, wparam, lparam);
            }
            0
        }
        WM_DRAWITEM => {
            if let Some(state) = state_ptr.as_ref() {
                let item = &*(lparam as *const DRAWITEMSTRUCT);
                match item.CtlType {
                    ODT_BUTTON_TYPE => state.paint_button(&item),
                    ODT_LISTBOX_TYPE => state.paint_list_item(&item),
                    _ => return 0,
                }
                return 1;
            }
            0
        }
        WM_CTLCOLOREDIT | WM_CTLCOLORSTATIC | WM_CTLCOLORLISTBOX => {
            if let Some(state) = state_ptr.as_ref() {
                let dc = wparam as HDC;
                SetTextColor(dc, INK);
                SetBkColor(dc, WHITE);
                return state.white_brush as LRESULT;
            }
            0
        }
        WM_CLOSE => {
            DestroyWindow(hwnd);
            0
        }
        WM_DESTROY => {
            KillTimer(hwnd, TIMER_ID);
            PostQuitMessage(0);
            0
        }
        WM_NCDESTROY => {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

#[derive(Default)]
struct Controls {
    toggle: HWND,
    settings: HWND,
    list: HWND,
    add: HWND,
    edit: HWND,
    del: HWND,
    clear: HWND,
    log: HWND,
}

struct GuiState {
    hwnd: HWND,
    dpi: i32,
    fonts: Fonts,
    controls: Controls,
    white_brush: HBRUSH,
    shared: Arc<Mutex<Shared>>,
    config_path: Option<PathBuf>,
    pad: PadState,
    deadzone_pct: u32,
    rows: Vec<ListRow>,
    last_toggle_label: String,
    // Client-space cursor overlay state. The native arrow remains available,
    // while this ring gives the pointer a modern console focus affordance.
    cursor_x: i32,
    cursor_y: i32,
    cursor_visible: bool,
}

/// Строка списка привязок: цветная фишка кнопки + текст + отметка вибрации.
struct ListRow {
    token: String,
    color: u32,
    text: String,
    rumble: bool,
}

/// Цвет фишки кнопки: ABXY в цветах Xbox, остальное в слейтовых тонах.
fn chip_color(button: GameButton) -> u32 {
    match button {
        GameButton::A => XBOX_GREEN,
        GameButton::B => XBOX_RED,
        GameButton::X => XBOX_BLUE,
        GameButton::Y => XBOX_YELLOW,
        GameButton::LB | GameButton::RB | GameButton::Back | GameButton::Start => SLATE,
        GameButton::LT | GameButton::RT => MUTED,
        GameButton::DPadUp
        | GameButton::DPadDown
        | GameButton::DPadLeft
        | GameButton::DPadRight
        | GameButton::LThumb
        | GameButton::RThumb => SLATE_SOFT,
    }
}

impl Drop for GuiState {
    fn drop(&mut self) {
        unsafe {
            DeleteObject(self.white_brush);
        }
    }
}

impl GuiState {
    fn new(shared: Arc<Mutex<Shared>>) -> Self {
        let dpi = unsafe { GetDpiForSystem() }.max(96) as i32;
        Self {
            hwnd: std::ptr::null_mut(),
            dpi,
            fonts: unsafe { Fonts::new(dpi) },
            controls: Controls::default(),
            white_brush: unsafe { CreateSolidBrush(WHITE) },
            shared,
            config_path: config::default_path(),
            pad: PadState::default(),
            deadzone_pct: 15,
            rows: Vec::new(),
            last_toggle_label: String::new(),
            cursor_x: 0,
            cursor_y: 0,
            cursor_visible: false,
        }
    }

    fn p(&self, n: i32) -> i32 {
        n * self.dpi / 96
    }

    unsafe fn replace_fonts(&mut self) {
        let old = std::mem::replace(&mut self.fonts, Fonts::new(self.dpi));
        self.apply_fonts();
        drop(old);
    }

    unsafe fn apply_fonts(&self) {
        for hwnd in [
            self.controls.toggle,
            self.controls.settings,
            self.controls.add,
            self.controls.edit,
            self.controls.del,
            self.controls.clear,
            self.controls.list,
        ] {
            SendMessageW(hwnd, WM_SETFONT, self.fonts.body as WPARAM, 1);
        }
        SendMessageW(self.controls.log, WM_SETFONT, self.fonts.log as WPARAM, 1);
    }

    unsafe fn create_controls(&mut self) -> bool {
        self.controls.toggle =
            self.child("BUTTON", "Запустить", IDC_TOGGLE, WS_TABSTOP | BS_OWNERDRAW);
        self.controls.settings = self.child(
            "BUTTON",
            "Настройки…",
            IDC_SETTINGS,
            WS_TABSTOP | BS_OWNERDRAW,
        );
        self.controls.list = self.child(
            "LISTBOX",
            "",
            IDC_LIST,
            WS_TABSTOP | WS_VSCROLL | WS_BORDER | LBS_OWNERDRAWFIXED | LBS_NOTIFY | LBS_HASSTRINGS,
        );
        self.controls.add = self.child("BUTTON", "Назначить…", IDC_ADD, WS_TABSTOP | BS_OWNERDRAW);
        self.controls.edit = self.child("BUTTON", "Изменить…", IDC_EDIT, WS_TABSTOP | BS_OWNERDRAW);
        self.controls.del = self.child("BUTTON", "Удалить", IDC_DEL, WS_TABSTOP | BS_OWNERDRAW);
        self.controls.clear =
            self.child("BUTTON", "Очистить", IDC_CLEAR, WS_TABSTOP | BS_OWNERDRAW);
        self.controls.log = self.child(
            "EDIT",
            "",
            IDC_LOG,
            WS_TABSTOP | WS_VSCROLL | WS_BORDER | ES_MULTILINE | ES_AUTOVSCROLL | ES_READONLY,
        );
        self.apply_fonts();
        SendMessageW(
            self.controls.log,
            EM_LIMITTEXT,
            (LOG_LIMIT + 16384) as WPARAM,
            0,
        );
        [
            self.controls.toggle,
            self.controls.settings,
            self.controls.list,
            self.controls.add,
            self.controls.edit,
            self.controls.del,
            self.controls.clear,
            self.controls.log,
        ]
        .iter()
        .all(|h| !h.is_null())
    }

    unsafe fn child(&self, class: &str, text: &str, id: u16, style: u32) -> HWND {
        CreateWindowExW(
            0,
            wide(class).as_ptr(),
            wide(text).as_ptr(),
            WS_CHILD | WS_VISIBLE | style,
            0,
            0,
            0,
            0,
            self.hwnd,
            id as isize as _,
            GetModuleHandleW(std::ptr::null()),
            std::ptr::null(),
        )
    }

    fn initialize(&mut self) {
        self.rebuild_list();
        unsafe { self.refresh_labels() };
    }

    /// Логические прямоугольники карточек под размер клиента.
    fn cards(&self) -> (RECT, RECT, RECT, RECT) {
        let mut client: RECT = unsafe { std::mem::zeroed() };
        unsafe { GetClientRect(self.hwnd, &mut client) };
        let w = client.right * 96 / self.dpi.max(1);
        let h = client.bottom * 96 / self.dpi.max(1);
        let hero = self.logical(24, 20, w - 24, 124);
        let pad = self.logical(24, 144, 524, 500);
        let bind = self.logical(548, 144, w - 24, 500);
        let journal = self.logical(24, 516, w - 24, h - 24);
        (hero, pad, bind, journal)
    }

    /// Прямоугольник из логических координат (левый/верхний/правый/нижний край).
    fn logical(&self, x1: i32, y1: i32, x2: i32, y2: i32) -> RECT {
        RECT {
            left: self.p(x1),
            top: self.p(y1),
            right: self.p(x2.max(x1)),
            bottom: self.p(y2.max(y1)),
        }
    }

    unsafe fn layout(&self) {
        let (hero, _pad, bind, journal) = self.cards();
        self.move_control(self.controls.toggle, hero, -198, 30, 174, 44);
        self.move_control(self.controls.settings, hero, -386, 30, 174, 44);
        // Список привязок и кнопки внутри карточки绑定.
        let bind_w = bind.right * 96 / self.dpi - 548;
        self.move_control_raw(
            self.controls.list,
            bind.left + self.p(20),
            bind.top + self.p(48),
            self.p((bind_w - 40).max(120)),
            self.p(248),
        );
        let buttons_y = bind.top + self.p(356 - 48);
        self.move_control_raw(
            self.controls.add,
            bind.left + self.p(20),
            buttons_y,
            self.p(116),
            self.p(34),
        );
        self.move_control_raw(
            self.controls.edit,
            bind.left + self.p(144),
            buttons_y,
            self.p(116),
            self.p(34),
        );
        self.move_control_raw(
            self.controls.del,
            bind.left + self.p(268),
            buttons_y,
            self.p(96),
            self.p(34),
        );
        self.move_control_raw(
            self.controls.clear,
            journal.right - self.p(105 + 20),
            journal.top + self.p(10),
            self.p(105),
            self.p(30),
        );
        let journal_w = journal.right * 96 / self.dpi - 24;
        self.move_control_raw(
            self.controls.log,
            journal.left + self.p(20),
            journal.top + self.p(48),
            self.p((journal_w - 40).max(120)),
            (journal.bottom - journal.top - self.p(64)).max(self.p(60)),
        );
        InvalidateRect(self.hwnd, std::ptr::null(), 0);
    }

    /// Позиция внутри карточки `base` со смещениями в логических единицах.
    unsafe fn move_control(&self, hwnd: HWND, base: RECT, dx: i32, dy: i32, w: i32, h: i32) {
        MoveWindow(
            hwnd,
            base.right + self.p(dx),
            base.top + self.p(dy),
            self.p(w),
            self.p(h),
            1,
        );
    }

    unsafe fn move_control_raw(&self, hwnd: HWND, x: i32, y: i32, w: i32, h: i32) {
        MoveWindow(hwnd, x, y, w, h, 1);
    }

    // ─────────────────────────── состояние ───────────────────────────

    fn on_list_selection_changed(&mut self) {
        unsafe { self.refresh_buttons_enabled() };
    }

    unsafe fn selected_index(&self) -> i32 {
        SendMessageW(self.controls.list, LB_GETCURSEL, 0, 0) as i32
    }

    unsafe fn refresh_buttons_enabled(&self) {
        let has = self.selected_index() >= 0;
        EnableWindow(self.controls.edit, has as i32);
        EnableWindow(self.controls.del, has as i32);
    }

    unsafe fn refresh_labels(&mut self) {
        let running = self.shared.lock().map(|s| s.running).unwrap_or(false);
        let label = if running {
            "Остановить"
        } else {
            "Запустить"
        };
        if label != self.last_toggle_label {
            set_text(self.controls.toggle, label);
            self.last_toggle_label = label.to_string();
        }
        self.refresh_buttons_enabled();
    }

    fn rebuild_list(&mut self) {
        let Ok(guard) = self.shared.lock() else {
            return;
        };
        let bindings = guard.cfg.bindings.clone();
        drop(guard);
        let keep = unsafe { self.selected_index() };
        self.rows = bindings
            .iter()
            .map(|b| ListRow {
                token: b.button.token().to_string(),
                color: chip_color(b.button),
                text: format!("{} → {}", b.trigger.label(), b.action.label()),
                rumble: b.rumble,
            })
            .collect();
        unsafe {
            SendMessageW(self.controls.list, LB_RESETCONTENT, 0, 0);
            for row in &self.rows {
                let text = wide(&row.token);
                SendMessageW(self.controls.list, LB_ADDSTRING, 0, text.as_ptr() as LPARAM);
            }
            if !self.rows.is_empty() {
                let index = keep.clamp(-1, self.rows.len() as i32 - 1);
                if index >= 0 {
                    SendMessageW(self.controls.list, LB_SETCURSEL, index as WPARAM, 0);
                }
            }
            self.refresh_buttons_enabled();
        }
    }

    fn on_timer(&mut self) {
        let drained: Vec<String> = {
            let Ok(mut guard) = self.shared.lock() else {
                return;
            };
            self.pad = guard.pad;
            self.deadzone_pct = guard.cfg.settings.deadzone_pct;
            guard.log.drain(..).collect()
        };
        for line in drained {
            unsafe { self.append_log(&line) };
        }
        unsafe {
            self.refresh_labels();
            InvalidateRect(self.hwnd, std::ptr::null(), 0);
        }
    }

    // ─────────────────────────── команды ───────────────────────────

    fn handle_command(&mut self, id: u16) {
        match id {
            IDC_TOGGLE => self.toggle_engine(),
            IDC_SETTINGS => self.open_settings(),
            IDC_ADD => self.open_binding_editor(None),
            IDC_EDIT => {
                let index = unsafe { self.selected_index() };
                let existing = self.binding_at(index);
                if existing.is_some() {
                    self.open_binding_editor(Some((index, existing.unwrap())));
                }
            }
            IDC_DEL => self.delete_selected(),
            IDC_CLEAR => unsafe { set_text(self.controls.log, "") },
            _ => {}
        }
    }

    fn toggle_engine(&mut self) {
        let now_running = {
            let Ok(mut guard) = self.shared.lock() else {
                return;
            };
            guard.running = !guard.running;
            if !guard.running {
                guard.capture = None;
            }
            guard.running
        };
        engine::log(
            &self.shared,
            if now_running {
                "Движок запущен — привязки активны."
            } else {
                "Движок остановлен — геймпад ничего не посылает."
            },
        );
        unsafe { self.refresh_labels() };
    }

    fn open_settings(&mut self) {
        let outcome = dialog::run_settings_dialog(self.hwnd, self.dpi, self.shared.clone());
        let Some(outcome) = outcome else { return };
        let cfg_snapshot = {
            let Ok(mut guard) = self.shared.lock() else {
                return;
            };
            guard.cfg.settings = outcome.settings;
            if outcome.reset_bindings {
                guard.cfg.bindings = default_bindings();
            }
            guard.version += 1;
            guard.cfg.clone()
        };
        self.save_config(&cfg_snapshot);
        self.rebuild_list();
        engine::log(&self.shared, "Настройки сохранены.");
    }

    fn open_binding_editor(&mut self, existing: Option<(i32, Binding)>) {
        let editing = existing.as_ref().map(|(_, binding)| binding.clone());
        let result = dialog::run_binding_dialog(self.hwnd, self.dpi, self.shared.clone(), editing);
        let Some(binding) = result else { return };
        let cfg_snapshot = {
            let Ok(mut guard) = self.shared.lock() else {
                return;
            };
            match existing {
                Some((index, _)) => {
                    if index >= 0 && (index as usize) < guard.cfg.bindings.len() {
                        guard.cfg.bindings[index as usize] = binding.clone();
                    } else {
                        guard.cfg.bindings.push(binding.clone());
                    }
                }
                None => guard.cfg.bindings.push(binding.clone()),
            }
            guard.version += 1;
            guard.cfg.clone()
        };
        self.save_config(&cfg_snapshot);
        self.rebuild_list();
        engine::log(
            &self.shared,
            &format!("Привязка сохранена: {}.", binding.display_plain()),
        );
    }

    fn binding_at(&self, index: i32) -> Option<Binding> {
        if index < 0 {
            return None;
        }
        let guard = self.shared.lock().ok()?;
        guard.cfg.bindings.get(index as usize).cloned()
    }

    fn delete_selected(&mut self) {
        let index = unsafe { self.selected_index() };
        let Some(binding) = self.binding_at(index) else {
            return;
        };
        if !confirm(
            Some(self.hwnd),
            &format!("Удалить привязку «{}»?", binding.display_plain()),
        ) {
            return;
        }
        let cfg_snapshot = {
            let Ok(mut guard) = self.shared.lock() else {
                return;
            };
            if (index as usize) < guard.cfg.bindings.len() {
                guard.cfg.bindings.remove(index as usize);
            }
            guard.version += 1;
            guard.cfg.clone()
        };
        self.save_config(&cfg_snapshot);
        self.rebuild_list();
        engine::log(
            &self.shared,
            &format!("Привязка удалена: {}.", binding.display_plain()),
        );
    }

    fn save_config(&mut self, cfg: &Config) {
        if let Err(problem) = config::save(self.config_path.as_deref(), cfg) {
            engine::log(
                &self.shared,
                &format!("Не удалось сохранить настройки: {problem}"),
            );
            show_message(
                Some(self.hwnd),
                &format!("Не удалось сохранить настройки:\n{problem}"),
            );
        }
    }

    // ─────────────────────────── отрисовка ───────────────────────────

    unsafe fn paint(&self) {
        let mut ps: PAINTSTRUCT = std::mem::zeroed();
        let dc = BeginPaint(self.hwnd, &mut ps);
        let mut rect: RECT = std::mem::zeroed();
        GetClientRect(self.hwnd, &mut rect);
        let mem = CreateCompatibleDC(dc);
        let bitmap = CreateCompatibleBitmap(dc, rect.right.max(1), rect.bottom.max(1));
        if !mem.is_null() && !bitmap.is_null() {
            let old = SelectObject(mem, bitmap);
            self.paint_content(mem, rect);
            BitBlt(dc, 0, 0, rect.right, rect.bottom, mem, 0, 0, SRCCOPY);
            SelectObject(mem, old);
        } else {
            self.paint_content(dc, rect);
        }
        if !bitmap.is_null() {
            DeleteObject(bitmap);
        }
        if !mem.is_null() {
            DeleteDC(mem);
        }
        EndPaint(self.hwnd, &ps);
    }

    unsafe fn paint_content(&self, dc: HDC, client: RECT) {
        fill(dc, &client, BACKGROUND);
        let (hero, pad, bind, journal) = self.cards();

        // ── Шапка ──
        card(dc, hero, self.p(14), self.p(3));
        // Бренд: зеленая планка (цвет Xbox) + название.
        let bar = RECT {
            left: hero.left + self.p(24),
            top: hero.top + self.p(16),
            right: hero.left + self.p(29),
            bottom: hero.top + self.p(44),
        };
        rounded(dc, bar, self.p(3), XBOX_GREEN, XBOX_GREEN);
        self.text(
            dc,
            APP_TITLE,
            hero,
            40,
            12,
            400,
            34,
            self.fonts.brand,
            INK,
            DT_LEFT,
        );
        let subtitle = match &self.config_path {
            Some(path) => format!(
                "Геймпад Xbox как клавиатура и мышь · настройки: {}",
                path.display()
            ),
            None => "Геймпад Xbox как клавиатура и мышь".to_string(),
        };
        self.text(
            dc,
            &subtitle,
            hero,
            40,
            50,
            640,
            16,
            self.fonts.small,
            MUTED,
            DT_LEFT,
        );

        let running = self.shared.lock().map(|s| s.running).unwrap_or(false);
        let (dot, pill_bg, status) = match (self.pad.connected, running) {
            (true, true) => {
                let battery = self
                    .pad
                    .battery
                    .map(|p| format!(" · заряд {p}%"))
                    .unwrap_or_default();
                (
                    GREEN,
                    PALE_GREEN,
                    format!("Геймпад подключен{battery} · движок работает"),
                )
            }
            (true, false) => (
                AMBER,
                PALE_AMBER,
                "Геймпад подключен · движок остановлен".to_string(),
            ),
            (false, _) => (
                RED,
                PALE_RED,
                "Геймпад не подключен — подсоедините контроллер Xbox".to_string(),
            ),
        };
        let pill = RECT {
            left: hero.left + self.p(24),
            top: hero.top + self.p(70),
            right: hero.left + self.p(24 + 510),
            bottom: hero.top + self.p(98),
        };
        rounded(dc, pill, self.p(14), pill_bg, pill_bg);
        let dot_radius = self.p(4);
        let dot_center_y = (pill.top + pill.bottom) / 2;
        circle(
            dc,
            RECT {
                left: pill.left + self.p(14),
                top: dot_center_y - dot_radius,
                right: pill.left + self.p(14) + dot_radius * 2,
                bottom: dot_center_y + dot_radius,
            },
            dot,
            dot,
            1,
        );
        self.text(
            dc,
            &status,
            pill,
            28,
            0,
            470,
            28,
            self.fonts.body,
            INK,
            DT_LEFT | DT_VCENTER,
        );

        // ── Карточка геймпада ──
        card(dc, pad, self.p(14), self.p(3));
        section_header(dc, "Геймпад", pad, &self.fonts, self.dpi, XBOX_GREEN);
        self.draw_battery(dc, pad);
        let viz = RECT {
            left: pad.left + self.p(20),
            top: pad.top + self.p(44),
            right: pad.right - self.p(20),
            bottom: pad.bottom - self.p(20),
        };
        self.draw_pad(dc, viz);

        // ── Карточка привязок ──
        card(dc, bind, self.p(14), self.p(3));
        section_header(dc, "Кнопки и действия", bind, &self.fonts, self.dpi, BLUE);
        let bind_w = bind.right * 96 / self.dpi - 548;
        self.text(
            dc,
            "двойной клик — изменить",
            bind,
            bind_w - 220,
            17,
            200,
            16,
            self.fonts.small,
            MUTED,
            DT_RIGHT,
        );

        // ── Журнал ──
        card(dc, journal, self.p(14), self.p(3));
        section_header(dc, "Журнал", journal, &self.fonts, self.dpi, SLATE);

        // PlayStation-inspired cursor: nested solid rings keep it visible on
        // both dark panels and text-heavy areas without alpha compositing.
        if self.cursor_visible {
            let r = self.p(12).max(8);
            let outer = RECT {
                left: self.cursor_x - r,
                top: self.cursor_y - r,
                right: self.cursor_x + r,
                bottom: self.cursor_y + r,
            };
            ring(dc, outer, rgb(8, 12, 24), self.p(4).max(2));
            ring(dc, outer, CURSOR_RING, self.p(2).max(1));
            let dot = self.p(3).max(2);
            circle(
                dc,
                RECT {
                    left: self.cursor_x - dot,
                    top: self.cursor_y - dot,
                    right: self.cursor_x + dot,
                    bottom: self.cursor_y + dot,
                },
                CURSOR_DOT,
                CURSOR_DOT,
                1,
            );
        }
    }

    /// Индикатор батареи в шапке карточки геймпада: иконка + проценты.
    unsafe fn draw_battery(&self, dc: HDC, card_rect: RECT) {
        let right = card_rect.right - self.p(20);
        let text_top = card_rect.top + self.p(15);
        let text_bottom = card_rect.top + self.p(33);
        if !self.pad.connected {
            let mut rect = RECT {
                left: right - self.p(130),
                top: text_top,
                right,
                bottom: text_bottom,
            };
            draw_text(
                dc,
                "нет соединения",
                &mut rect,
                self.fonts.small,
                MUTED,
                DT_RIGHT | DT_VCENTER | DT_SINGLELINE,
            );
            return;
        }
        let percent = self.pad.battery;
        let label = percent
            .map(|v| format!("{v}%"))
            .unwrap_or_else(|| "?".into());
        let mut text_rect = RECT {
            left: right - self.p(46),
            top: text_top,
            right,
            bottom: text_bottom,
        };
        draw_text(
            dc,
            &label,
            &mut text_rect,
            self.fonts.small,
            INK,
            DT_RIGHT | DT_VCENTER | DT_SINGLELINE,
        );
        // Корпус батарейки + контакт.
        let body = RECT {
            left: right - self.p(46 + 8 + 24),
            top: card_rect.top + self.p(18),
            right: right - self.p(54),
            bottom: card_rect.top + self.p(30),
        };
        rounded(dc, body, self.p(3), WHITE, MUTED);
        let level_color = match percent {
            Some(v) if v > 50 => GREEN,
            Some(v) if v > 20 => AMBER,
            Some(_) => RED,
            None => MUTED,
        };
        let inner_max = (body.right - body.left - self.p(4)).max(1);
        let inner_w = (inner_max as f64 * percent.unwrap_or(0) as f64 / 100.0) as i32;
        if inner_w > 0 {
            let inner = RECT {
                left: body.left + self.p(2),
                top: body.top + self.p(2),
                right: body.left + self.p(2) + inner_w,
                bottom: body.bottom - self.p(2),
            };
            fill(dc, &inner, level_color);
        }
        let cap = RECT {
            left: body.right + self.p(1),
            top: body.top + self.p(3),
            right: body.right + self.p(3),
            bottom: body.bottom - self.p(3),
        };
        fill(dc, &cap, MUTED);
    }

    /// Текст в логических координатах относительно карточки `base`.
    #[allow(clippy::too_many_arguments)]
    unsafe fn text(
        &self,
        dc: HDC,
        value: &str,
        base: RECT,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        font: HFONT,
        color: u32,
        flags: u32,
    ) {
        let mut rect = RECT {
            left: base.left + self.p(x),
            top: base.top + self.p(y),
            right: base.left + self.p(x + w),
            bottom: base.top + self.p(y + h),
        };
        draw_text(dc, value, &mut rect, font, color, flags | DT_SINGLELINE);
    }

    /// Живая визуализация геймпада (логические единицы 460×292 внутри viz).
    unsafe fn draw_pad(&self, dc: HDC, viz: RECT) {
        let px = |ux: i32| viz.left + self.p(ux);
        let py = |uy: i32| viz.top + self.p(uy);
        let rect = |x1: i32, y1: i32, x2: i32, y2: i32| RECT {
            left: px(x1),
            top: py(y1),
            right: px(x2),
            bottom: py(y2),
        };
        let pad = self.pad;
        let on = |bit: u16| pad.connected && pad.buttons & bit != 0;

        // Корпус: две рукоятки + средняя часть одним силуэтом.
        silhouette(
            dc,
            &[
                Shape::Ellipse(rect(25, 120, 160, 272)),
                Shape::Ellipse(rect(300, 120, 435, 272)),
                Shape::Round(rect(55, 72, 405, 196), self.p(48)),
            ],
            PAD_BODY,
            PAD_EDGE,
            self.p(2).max(1),
        );

        // Триггеры: вертикальные индикаторы-полоски.
        for (x1, value, lit) in [
            (105i32, pad.lt, pad.connected && pad.lt >= 128),
            (335, pad.rt, pad.connected && pad.rt >= 128),
        ] {
            rounded(dc, rect(x1, 18, x1 + 20, 54), self.p(6), PAD_DARK, PAD_EDGE);
            let fraction = value as f64 / 255.0;
            let bar_height = (32.0 * fraction) as i32;
            if bar_height > 1 {
                rounded(
                    dc,
                    rect(x1 + 3, 51 - bar_height, x1 + 17, 51),
                    self.p(4),
                    if lit { PAD_LIT } else { BLUE },
                    if lit { PAD_LIT } else { BLUE },
                );
            }
            self.pad_label(
                dc,
                rect(x1 - 4, 2, x1 + 24, 14),
                if x1 < 200 { "LT" } else { "RT" },
            );
        }

        // Бамперы.
        for (x1, label, pressed) in [
            (95i32, "LB", on(XINPUT_GAMEPAD_LEFT_SHOULDER)),
            (285, "RB", on(XINPUT_GAMEPAD_RIGHT_SHOULDER)),
        ] {
            rounded(
                dc,
                rect(x1, 60, x1 + 80, 76),
                self.p(8),
                if pressed { PAD_LIT } else { PAD_CONTROL },
                PAD_EDGE,
            );
            self.pad_label(dc, rect(x1, 60, x1 + 80, 76), label);
        }

        // Крестовина.
        let cross_color = PAD_CONTROL;
        rounded(
            dc,
            rect(186, 142, 204, 194),
            self.p(5),
            cross_color,
            PAD_EDGE,
        );
        rounded(
            dc,
            rect(169, 159, 221, 177),
            self.p(5),
            cross_color,
            PAD_EDGE,
        );
        if on(XINPUT_GAMEPAD_DPAD_UP) {
            rounded(dc, rect(186, 142, 204, 170), self.p(5), PAD_LIT, PAD_LIT);
        }
        if on(XINPUT_GAMEPAD_DPAD_DOWN) {
            rounded(dc, rect(186, 166, 204, 194), self.p(5), PAD_LIT, PAD_LIT);
        }
        if on(XINPUT_GAMEPAD_DPAD_LEFT) {
            rounded(dc, rect(169, 159, 197, 177), self.p(5), PAD_LIT, PAD_LIT);
        }
        if on(XINPUT_GAMEPAD_DPAD_RIGHT) {
            rounded(dc, rect(193, 159, 221, 177), self.p(5), PAD_LIT, PAD_LIT);
        }
        rounded(dc, rect(190, 163, 200, 173), self.p(2), PAD_DARK, PAD_DARK);

        // Кнопки ABXY в цветах Xbox; при нажатии — цветное гало.
        for (cx, cy, color, letter, bit) in [
            (330i32, 94i32, XBOX_YELLOW, "Y", XINPUT_GAMEPAD_Y),
            (306, 118, XBOX_BLUE, "X", XINPUT_GAMEPAD_X),
            (354, 118, XBOX_RED, "B", XINPUT_GAMEPAD_B),
            (330, 142, XBOX_GREEN, "A", XINPUT_GAMEPAD_A),
        ] {
            let pressed = on(bit);
            if pressed {
                ring(
                    dc,
                    rect(cx - 17, cy - 17, cx + 17, cy + 17),
                    color,
                    self.p(3).max(2),
                );
            }
            circle(
                dc,
                rect(cx - 13, cy - 13, cx + 13, cy + 13),
                if pressed { color } else { PAD_DARK },
                if pressed { WHITE } else { color },
                self.p(2).max(1),
            );
            let mut text_rect = rect(cx - 13, cy - 13, cx + 13, cy + 13);
            draw_text(
                dc,
                letter,
                &mut text_rect,
                self.fonts.small,
                if pressed { WHITE } else { color },
                DT_CENTER | DT_VCENTER | DT_SINGLELINE,
            );
        }

        // Back / Start / Guide.
        for (x1, bit) in [(212i32, XINPUT_GAMEPAD_BACK), (248, XINPUT_GAMEPAD_START)] {
            rounded(
                dc,
                rect(x1, 102, x1 + 18, 111),
                self.p(4),
                if on(bit) { PAD_LIT } else { PAD_CONTROL },
                PAD_EDGE,
            );
        }
        // Guide-кнопка светится зеленым, когда контроллер на связи.
        circle(
            dc,
            rect(231, 78, 247, 94),
            if pad.connected { XBOX_GREEN } else { PAD_DARK },
            PAD_CONTROL,
            self.p(2).max(1),
        );
        let mut guide_rect = rect(231, 78, 247, 94);
        draw_text(
            dc,
            "X",
            &mut guide_rect,
            self.fonts.small,
            if pad.connected { WHITE } else { PAD_KNOB },
            DT_CENTER | DT_VCENTER | DT_SINGLELINE,
        );

        // Стики.
        let dz = self.deadzone_pct as f64 / 100.0;
        self.draw_stick(
            dc,
            &rect,
            135,
            124,
            pad.lx,
            pad.ly,
            on(XINPUT_GAMEPAD_LEFT_THUMB),
            dz,
        );
        self.draw_stick(
            dc,
            &rect,
            272,
            168,
            pad.rx,
            pad.ry,
            on(XINPUT_GAMEPAD_RIGHT_THUMB),
            dz,
        );

        // Нет соединения — плашка поверх.
        if !pad.connected {
            let pill = rect(118, 126, 342, 158);
            rounded(dc, pill, self.p(16), WHITE, RED);
            let mut text_rect = pill;
            draw_text(
                dc,
                "Геймпад не подключен",
                &mut text_rect,
                self.fonts.body,
                RED,
                DT_CENTER | DT_VCENTER | DT_SINGLELINE,
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    unsafe fn draw_stick(
        &self,
        dc: HDC,
        rect: &impl Fn(i32, i32, i32, i32) -> RECT,
        cx: i32,
        cy: i32,
        x: f64,
        y: f64,
        pressed: bool,
        deadzone: f64,
    ) {
        circle(
            dc,
            rect(cx - 21, cy - 21, cx + 21, cy + 21),
            PAD_DARK,
            PAD_CONTROL,
            self.p(2).max(1),
        );
        let dz_radius = (21.0 * deadzone.min(0.95)) as i32;
        if dz_radius > 2 {
            ring(
                dc,
                rect(
                    cx - dz_radius,
                    cy - dz_radius,
                    cx + dz_radius,
                    cy + dz_radius,
                ),
                rgb(62, 76, 99),
                self.p(1).max(1),
            );
        }
        let knob_x = cx + (x * 12.0) as i32;
        let knob_y = cy - (y * 12.0) as i32;
        circle(
            dc,
            rect(knob_x - 12, knob_y - 12, knob_x + 12, knob_y + 12),
            if pressed { PAD_LIT } else { PAD_KNOB },
            PAD_EDGE,
            self.p(1).max(1),
        );
    }

    unsafe fn pad_label(&self, dc: HDC, rect: RECT, label: &str) {
        let mut rect = rect;
        draw_text(
            dc,
            label,
            &mut rect,
            self.fonts.small,
            rgb(182, 194, 214),
            DT_CENTER | DT_VCENTER | DT_SINGLELINE,
        );
    }

    unsafe fn paint_button(&self, item: &DRAWITEMSTRUCT) {
        let pressed = item.itemState & ODS_SELECTED_BIT != 0;
        let disabled = item.itemState & ODS_DISABLED_BIT != 0;
        let focused = item.itemState & ODS_FOCUS_BIT != 0;
        let running = self.shared.lock().map(|s| s.running).unwrap_or(false);
        let style = match item.CtlID as u16 {
            IDC_ADD => ButtonStyle::Primary,
            IDC_TOGGLE => {
                if running {
                    ButtonStyle::Danger
                } else {
                    ButtonStyle::Primary
                }
            }
            _ => ButtonStyle::Normal,
        };
        paint_owner_button(
            item.hDC,
            item.rcItem,
            &get_text(item.hwndItem),
            &self.fonts,
            self.dpi,
            style,
            pressed,
            disabled,
            focused,
        );
    }

    unsafe fn paint_list_item(&self, item: &DRAWITEMSTRUCT) {
        let index = item.itemID;
        if index == u32::MAX {
            fill(item.hDC, &item.rcItem, WHITE);
            return;
        }
        let Some(row) = self.rows.get(index as usize) else {
            fill(item.hDC, &item.rcItem, WHITE);
            return;
        };
        let selected = item.itemState & ODS_SELECTED_BIT != 0;
        let background = if selected {
            PALE_BLUE
        } else if index % 2 == 1 {
            ROW_ALT
        } else {
            WHITE
        };
        fill(item.hDC, &item.rcItem, background);
        if selected {
            fill(
                item.hDC,
                &RECT {
                    left: item.rcItem.left,
                    top: item.rcItem.top,
                    right: item.rcItem.left + self.p(4),
                    bottom: item.rcItem.bottom,
                },
                BLUE,
            );
        }
        // Фишка кнопки: скругленный цветной чип с токеном.
        let center_y = (item.rcItem.top + item.rcItem.bottom) / 2;
        let chip = RECT {
            left: item.rcItem.left + self.p(12),
            top: center_y - self.p(11),
            right: item.rcItem.left + self.p(12 + 78),
            bottom: center_y + self.p(11),
        };
        rounded(item.hDC, chip, self.p(6), row.color, row.color);
        let mut chip_text = chip;
        draw_text(
            item.hDC,
            &row.token,
            &mut chip_text,
            self.fonts.button,
            WHITE,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE,
        );
        // Отметка вибрации справа янтарным.
        if row.rumble {
            let mut rumble_rect = RECT {
                left: item.rcItem.right - self.p(64),
                top: item.rcItem.top,
                right: item.rcItem.right - self.p(12),
                bottom: item.rcItem.bottom,
            };
            draw_text(
                item.hDC,
                "вибро",
                &mut rumble_rect,
                self.fonts.small,
                AMBER,
                DT_RIGHT | DT_VCENTER | DT_SINGLELINE,
            );
        }
        // Событие → действие.
        let mut text_rect = RECT {
            left: item.rcItem.left + self.p(102),
            top: item.rcItem.top,
            right: item.rcItem.right - if row.rumble { self.p(70) } else { self.p(12) },
            bottom: item.rcItem.bottom,
        };
        draw_text(
            item.hDC,
            &row.text,
            &mut text_rect,
            self.fonts.body,
            if selected { BLUE_DARK } else { INK },
            DT_LEFT | DT_VCENTER | DT_SINGLELINE,
        );
    }

    unsafe fn append_log(&self, line: &str) {
        let mut time: SYSTEMTIME = std::mem::zeroed();
        GetLocalTime(&mut time);
        let text = format!(
            "{:02}:{:02}:{:02}   {}\r\n",
            time.wHour, time.wMinute, time.wSecond, line
        );
        let mut len = GetWindowTextLengthW(self.controls.log).max(0) as usize;
        if len > LOG_LIMIT {
            SendMessageW(self.controls.log, EM_SETSEL, 0, (len / 2) as LPARAM);
            SendMessageW(
                self.controls.log,
                EM_REPLACESEL,
                0,
                wide("").as_ptr() as LPARAM,
            );
            len = GetWindowTextLengthW(self.controls.log).max(0) as usize;
        }
        SendMessageW(self.controls.log, EM_SETSEL, len, len as LPARAM);
        SendMessageW(
            self.controls.log,
            EM_REPLACESEL,
            0,
            wide(&text).as_ptr() as LPARAM,
        );
    }
}
