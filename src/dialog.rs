//! Диалоги в стиле основного окна: редактор привязки (захват кнопки геймпада
//! и комбинаций с клавиатуры) и редактор настроек движения/скролла/вибрации.
//!
//! Оба — рукописные модальные окна на Win32 (owner выключается на время работы).

use std::sync::{Arc, Mutex, Once};

use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Controls::*;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, GetKeyState, SetFocus};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::binding::{Action, Binding, GameButton, MacroStep, TriggerKind};
use crate::config::{Settings, StickMode};
use crate::engine::{Capture, Shared};
use crate::send::MouseButton;
use crate::ui::*;
use crate::vk::{format_combo, is_modifier, modifier_order, parse_combo};

const VK_RETURN: u16 = 0x0D;
const VK_ESCAPE: u16 = 0x1B;

const DLG_W: i32 = 480;
const DLG_H: i32 = 446;
const TIMER_DLG: usize = 2;
const TIMER_DLG_MS: u32 = 50;
const DLG_STYLE: u32 = WS_POPUP | WS_CAPTION | WS_SYSMENU;

// Числовые значения уведомлений/состояний стабильны в Win32.
const CBN_SELCHANGE_CODE: u16 = 1;
const ODS_SELECTED_FLAG: u32 = 0x0001;
const ODS_DISABLED_FLAG: u32 = 0x0004;
const ODS_FOCUS_FLAG: u32 = 0x0010;
/// SS_LEFT (0) — статик с переносом строк; windows-sys константу не экспортирует.
const SS_LEFT_STYLE: u32 = 0;

// Стили контролов локально в u32 (как в alibi-quic): в windows-sys они i32.
const ES_CENTER: u32 = 0x0001;
const ES_AUTOHSCROLL: u32 = 0x0080;
const ES_READONLY: u32 = 0x0800;
const BS_AUTOCHECKBOX: u32 = 0x0003;
const BS_OWNERDRAW: u32 = 0x000B;
const CBS_DROPDOWNLIST: u32 = 0x0003;

/// Что вернул диалог настроек.
pub struct SettingsOutcome {
    pub settings: Settings,
    pub reset_bindings: bool,
}

// ─────────────────────── общие вспомогательные ───────────────────────

unsafe fn place_over(owner: HWND, w: i32, h: i32) -> (i32, i32, i32, i32) {
    let mut rect = RECT {
        left: 0,
        top: 0,
        right: w,
        bottom: h,
    };
    AdjustWindowRectEx(&mut rect, DLG_STYLE, 0, 0);
    let full_w = rect.right - rect.left;
    let full_h = rect.bottom - rect.top;
    let mut owner_rect: RECT = std::mem::zeroed();
    GetWindowRect(owner, &mut owner_rect);
    let x = owner_rect.left + ((owner_rect.right - owner_rect.left) - full_w) / 2;
    let y = owner_rect.top + ((owner_rect.bottom - owner_rect.top) - full_h) / 2;
    (x.max(0), y.max(0), full_w, full_h)
}

/// Модальный цикл: owner выключен, Esc/Tab обрабатывает IsDialogMessage,
/// кроме моментов захвата клавиатуры (там нужны «сырые» WM_KEYDOWN).
unsafe fn modal_loop<F: Fn() -> bool>(dlg: HWND, owner: HWND, capturing_keys: F) {
    EnableWindow(owner, 0);
    let mut msg: MSG = std::mem::zeroed();
    while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
        let skip = capturing_keys() && msg.hwnd == dlg;
        if !skip && IsDialogMessageW(dlg, &msg) != 0 {
            continue;
        }
        TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }
    EnableWindow(owner, 1);
    SetForegroundWindow(owner);
}

unsafe fn dialog_child(
    parent: HWND,
    class: &str,
    text: &str,
    id: u16,
    dpi: i32,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    style: u32,
    font: HFONT,
) -> HWND {
    let p = |n: i32| n * dpi / 96;
    let hwnd = CreateWindowExW(
        0,
        wide(class).as_ptr(),
        wide(text).as_ptr(),
        WS_CHILD | WS_VISIBLE | style,
        p(x),
        p(y),
        p(w),
        p(h),
        parent,
        id as isize as _,
        GetModuleHandleW(std::ptr::null()),
        std::ptr::null(),
    );
    SendMessageW(hwnd, WM_SETFONT, font as WPARAM, 1);
    hwnd
}

fn combo_index(hwnd: HWND) -> i32 {
    unsafe { SendMessageW(hwnd, CB_GETCURSEL, 0, 0) as i32 }
}

fn is_checked(hwnd: HWND) -> bool {
    unsafe { SendMessageW(hwnd, BM_GETCHECK, 0, 0) == BST_CHECKED as LRESULT }
}

// ─────────────────────────── редактор привязки ───────────────────────────

const IDC_PAD_FIELD: u16 = 2001;
const IDC_PAD_CAPTURE: u16 = 2002;
const IDC_TRIGGER: u16 = 2003;
const IDC_MILLIS: u16 = 2004;
const IDC_ACTION: u16 = 2006;
const IDC_VALUE_KEYS: u16 = 2007;
const IDC_VALUE_TEXT: u16 = 2008;
const IDC_VALUE_MOUSE: u16 = 2009;
const IDC_VALUE_WHEEL: u16 = 2010;
const IDC_VALUE_MACRO: u16 = 2011;
const IDC_KEYS_CAPTURE: u16 = 2012;
const IDC_HINT: u16 = 2013;
const IDC_RUMBLE: u16 = 2014;
const IDC_OK: u16 = 2015;
const IDC_CANCEL: u16 = 2016;

/// Открывает редактор привязки; возвращает готовую привязку или None (отмена).
pub fn run_binding_dialog(
    owner: HWND,
    dpi: i32,
    shared: Arc<Mutex<Shared>>,
    existing: Option<Binding>,
) -> Option<Binding> {
    unsafe {
        static REGISTER: Once = Once::new();
        let instance = GetModuleHandleW(std::ptr::null());
        let class_name = wide("XboxDeskBindingDialog");
        REGISTER.call_once(|| {
            let wc = WNDCLASSW {
                lpfnWndProc: Some(binding_wnd_proc),
                hInstance: instance,
                hCursor: LoadCursorW(std::ptr::null_mut(), IDC_ARROW),
                hIcon: LoadIconW(instance, 1usize as *const u16),
                hbrBackground: CreateSolidBrush(BACKGROUND),
                lpszClassName: class_name.as_ptr(),
                ..std::mem::zeroed()
            };
            RegisterClassW(&wc);
        });

        let title = if existing.is_some() {
            "Изменить привязку"
        } else {
            "Новая привязка"
        };
        let mut state = Box::new(BindingDialog::new(dpi, shared, existing));
        let (x, y, w, h) = place_over(owner, state.p(DLG_W), state.p(DLG_H));
        let dlg = CreateWindowExW(
            0,
            class_name.as_ptr(),
            wide(title).as_ptr(),
            DLG_STYLE | WS_VISIBLE,
            x,
            y,
            w,
            h,
            owner,
            std::ptr::null_mut(),
            instance,
            (&mut *state as *mut BindingDialog).cast(),
        );
        if dlg.is_null() {
            show_message(Some(owner), "Не удалось открыть окно привязки.");
            return None;
        }
        modal_loop(dlg, owner, || state.capturing_keys);
        state.result.take()
    }
}

#[derive(Default)]
struct BindingControls {
    pad_field: HWND,
    pad_capture: HWND,
    trigger: HWND,
    millis: HWND,
    action: HWND,
    value_keys: HWND,
    keys_capture: HWND,
    value_text: HWND,
    value_mouse: HWND,
    value_wheel: HWND,
    value_macro: HWND,
    hint: HWND,
    rumble: HWND,
    ok: HWND,
    cancel: HWND,
}

struct BindingDialog {
    hwnd: HWND,
    dpi: i32,
    fonts: Fonts,
    shared: Arc<Mutex<Shared>>,
    controls: BindingControls,
    white_brush: HBRUSH,
    capturing_keys: bool,
    capturing_pad: bool,
    pending_existing: Option<Binding>,
    keys_before_capture: String,
    result: Option<Binding>,
}

impl Drop for BindingDialog {
    fn drop(&mut self) {
        unsafe { DeleteObject(self.white_brush) };
    }
}

impl BindingDialog {
    fn new(dpi: i32, shared: Arc<Mutex<Shared>>, existing: Option<Binding>) -> Self {
        Self {
            hwnd: std::ptr::null_mut(),
            dpi,
            fonts: unsafe { Fonts::new(dpi) },
            shared,
            controls: BindingControls::default(),
            white_brush: unsafe { CreateSolidBrush(WHITE) },
            capturing_keys: false,
            capturing_pad: false,
            keys_before_capture: String::new(),
            result: None,
            pending_existing: existing,
        }
    }

    fn p(&self, n: i32) -> i32 {
        n * self.dpi / 96
    }

    unsafe fn create_controls(&mut self) -> bool {
        let dpi = self.dpi;
        let body = self.fonts.body;
        let small = self.fonts.small;
        let hwnd = self.hwnd;
        let child =
            |class: &str, text: &str, id: u16, x: i32, y: i32, w: i32, h: i32, style: u32| {
                dialog_child(hwnd, class, text, id, dpi, x, y, w, h, style, body)
            };
        let c = &mut self.controls;
        c.pad_field = child(
            "EDIT",
            "",
            IDC_PAD_FIELD,
            20,
            40,
            290,
            28,
            ES_READONLY | ES_AUTOHSCROLL | ES_CENTER | WS_TABSTOP | WS_BORDER,
        );
        c.pad_capture = child(
            "BUTTON",
            "Нажать…",
            IDC_PAD_CAPTURE,
            320,
            38,
            140,
            32,
            WS_TABSTOP | BS_OWNERDRAW,
        );
        c.trigger = child(
            "COMBOBOX",
            "",
            IDC_TRIGGER,
            20,
            104,
            240,
            240,
            WS_TABSTOP | CBS_DROPDOWNLIST | WS_VSCROLL,
        );
        for item in [
            "Нажатие",
            "Отпускание",
            "Удержание",
            "Двойное нажатие",
            "Повтор",
        ] {
            SendMessageW(c.trigger, CB_ADDSTRING, 0, wide(item).as_ptr() as LPARAM);
        }
        SendMessageW(c.trigger, CB_SETCURSEL, 0, 0);
        c.millis = child(
            "EDIT",
            "",
            IDC_MILLIS,
            272,
            104,
            70,
            28,
            ES_AUTOHSCROLL | WS_TABSTOP | WS_BORDER,
        );
        c.action = child(
            "COMBOBOX",
            "",
            IDC_ACTION,
            20,
            168,
            440,
            260,
            WS_TABSTOP | CBS_DROPDOWNLIST | WS_VSCROLL,
        );
        for item in [
            "Клавиши / комбинация",
            "Ввод текста",
            "Клик мыши",
            "Колесо прокрутки",
            "Колесо горизонтальное",
            "Макропоследовательность",
        ] {
            SendMessageW(c.action, CB_ADDSTRING, 0, wide(item).as_ptr() as LPARAM);
        }
        SendMessageW(c.action, CB_SETCURSEL, 0, 0);
        c.value_keys = child(
            "EDIT",
            "",
            IDC_VALUE_KEYS,
            20,
            232,
            300,
            28,
            ES_AUTOHSCROLL | WS_TABSTOP | WS_BORDER,
        );
        c.keys_capture = child(
            "BUTTON",
            "С клавиатуры",
            IDC_KEYS_CAPTURE,
            330,
            230,
            130,
            32,
            WS_TABSTOP | BS_OWNERDRAW,
        );
        c.value_text = child(
            "EDIT",
            "",
            IDC_VALUE_TEXT,
            20,
            232,
            440,
            28,
            ES_AUTOHSCROLL | WS_TABSTOP | WS_BORDER,
        );
        c.value_mouse = child(
            "COMBOBOX",
            "",
            IDC_VALUE_MOUSE,
            20,
            232,
            440,
            240,
            WS_TABSTOP | CBS_DROPDOWNLIST | WS_VSCROLL,
        );
        for item in [
            "Левая (ЛКМ)",
            "Правая (ПКМ)",
            "Средняя (СКМ)",
            "Боковая X1 — назад",
            "Боковая X2 — вперед",
        ] {
            SendMessageW(
                c.value_mouse,
                CB_ADDSTRING,
                0,
                wide(item).as_ptr() as LPARAM,
            );
        }
        SendMessageW(c.value_mouse, CB_SETCURSEL, 0, 0);
        c.value_wheel = child(
            "EDIT",
            "3",
            IDC_VALUE_WHEEL,
            20,
            232,
            90,
            28,
            ES_AUTOHSCROLL | WS_TABSTOP | WS_BORDER,
        );
        c.value_macro = child(
            "EDIT",
            "",
            IDC_VALUE_MACRO,
            20,
            232,
            440,
            28,
            ES_AUTOHSCROLL | WS_TABSTOP | WS_BORDER,
        );
        c.hint = dialog_child(
            hwnd,
            "STATIC",
            "",
            IDC_HINT,
            dpi,
            20,
            272,
            440,
            46,
            SS_LEFT_STYLE,
            small,
        );
        c.rumble = child(
            "BUTTON",
            "Вибрация при срабатывании",
            IDC_RUMBLE,
            20,
            330,
            440,
            24,
            WS_TABSTOP | BS_AUTOCHECKBOX,
        );
        c.ok = child(
            "BUTTON",
            "Сохранить",
            IDC_OK,
            200,
            386,
            120,
            40,
            WS_TABSTOP | BS_OWNERDRAW,
        );
        c.cancel = child(
            "BUTTON",
            "Отмена",
            IDC_CANCEL,
            330,
            386,
            130,
            40,
            WS_TABSTOP | BS_OWNERDRAW,
        );
        // Заголовки полей.
        dialog_child(
            hwnd,
            "STATIC",
            "Кнопка геймпада",
            9001,
            dpi,
            20,
            16,
            440,
            18,
            SS_LEFT_STYLE,
            body,
        );
        dialog_child(
            hwnd,
            "STATIC",
            "Событие",
            9002,
            dpi,
            20,
            80,
            200,
            18,
            SS_LEFT_STYLE,
            body,
        );
        dialog_child(
            hwnd,
            "STATIC",
            "мс",
            9003,
            dpi,
            350,
            108,
            30,
            18,
            SS_LEFT_STYLE,
            small,
        );
        dialog_child(
            hwnd,
            "STATIC",
            "Действие",
            9004,
            dpi,
            20,
            144,
            200,
            18,
            SS_LEFT_STYLE,
            body,
        );
        dialog_child(
            hwnd,
            "STATIC",
            "Значение",
            9005,
            dpi,
            20,
            208,
            200,
            18,
            SS_LEFT_STYLE,
            body,
        );
        SendMessageW(c.millis, EM_LIMITTEXT, 6, 0);
        SendMessageW(c.value_wheel, EM_LIMITTEXT, 6, 0);
        [
            c.pad_field,
            c.pad_capture,
            c.trigger,
            c.millis,
            c.action,
            c.value_keys,
            c.keys_capture,
            c.value_text,
            c.value_mouse,
            c.value_wheel,
            c.value_macro,
            c.hint,
            c.rumble,
            c.ok,
            c.cancel,
        ]
        .iter()
        .all(|h| !h.is_null())
    }

    /// Раскладывает редактируемую привязку по полям.
    unsafe fn apply_existing(&mut self) {
        let Some(binding) = self.pending_existing.take() else {
            return;
        };
        set_text(self.controls.pad_field, binding.button.token());
        let (trigger_index, millis) = match binding.trigger {
            TriggerKind::Press => (0, None),
            TriggerKind::Release => (1, None),
            TriggerKind::Hold(ms) => (2, Some(ms)),
            TriggerKind::Double(ms) => (3, Some(ms)),
            TriggerKind::Repeat(ms) => (4, Some(ms)),
        };
        SendMessageW(
            self.controls.trigger,
            CB_SETCURSEL,
            trigger_index as WPARAM,
            0,
        );
        if let Some(ms) = millis {
            set_text(self.controls.millis, &ms.to_string());
        }
        let action_index = match &binding.action {
            Action::Keys(_) => 0,
            Action::Text(_) => 1,
            Action::Mouse(_) => 2,
            Action::Wheel { horizontal, .. } => {
                if *horizontal {
                    4
                } else {
                    3
                }
            }
            Action::Macro(_) => 5,
        };
        SendMessageW(
            self.controls.action,
            CB_SETCURSEL,
            action_index as WPARAM,
            0,
        );
        match &binding.action {
            Action::Keys(keys) => set_text(self.controls.value_keys, &format_combo(keys)),
            Action::Text(text) => set_text(self.controls.value_text, text),
            Action::Mouse(button) => {
                let order = [
                    MouseButton::Left,
                    MouseButton::Right,
                    MouseButton::Middle,
                    MouseButton::X1,
                    MouseButton::X2,
                ];
                let index = order.iter().position(|b| b == button).unwrap_or(0);
                SendMessageW(self.controls.value_mouse, CB_SETCURSEL, index, 0);
            }
            Action::Wheel { notches, .. } => {
                set_text(self.controls.value_wheel, &notches.to_string())
            }
            Action::Macro(steps) => {
                let text = steps
                    .iter()
                    .map(|step| match step {
                        MacroStep::Keys(keys) => format!("keys:{}", format_combo(keys)),
                        MacroStep::Text(text) => format!("text:{text}"),
                        MacroStep::Delay(ms) => format!("delay:{ms}"),
                    })
                    .collect::<Vec<_>>()
                    .join(";");
                set_text(self.controls.value_macro, &text);
            }
        }
        if binding.rumble {
            SendMessageW(self.controls.rumble, BM_SETCHECK, BST_CHECKED as WPARAM, 0);
        }
    }

    /// Показывает нужное поле значения и подсказку под текущие комбобоксы.
    unsafe fn apply_action_visibility(&self) {
        let action_index = combo_index(self.controls.action);
        let trigger_index = combo_index(self.controls.trigger);
        let show = |hwnd: HWND, visible: bool| {
            ShowWindow(hwnd, if visible { SW_SHOW } else { SW_HIDE });
        };
        show(self.controls.value_keys, action_index == 0);
        show(self.controls.keys_capture, action_index == 0);
        show(self.controls.value_text, action_index == 1);
        show(self.controls.value_mouse, action_index == 2);
        show(
            self.controls.value_wheel,
            action_index == 3 || action_index == 4,
        );
        show(self.controls.value_macro, action_index == 5);
        EnableWindow(self.controls.millis, i32::from(trigger_index >= 2));
        // Подсказка по действию + типичные значения времени события.
        let hint = match action_index {
            0 => "Например: CTRL+SHIFT+T, WIN+D, F5, VOLUME_UP. Нажмите «С клавиатуры» и введите комбинацию.",
            1 => "Текст вводится посимвольно — раскладка и язык не важны.",
            2 => "Одинарный клик выбранной кнопкой мыши.",
            3 | 4 => "Деления колеса: 3 — три шага вверх, -2 — два шага вниз.",
            _ => "Шаги через точку с запятой: keys:CTRL+A;delay:200;text:привет;keys:ENTER",
        };
        let time_hint = match trigger_index {
            2 => " Удержание: по умолчанию 500 мс.",
            3 => " Двойное: окно по умолчанию 300 мс.",
            4 => " Повтор: интервал по умолчанию 150 мс.",
            _ => "",
        };
        set_text(self.controls.hint, &format!("{hint}{time_hint}"));
    }

    /// Предзаполнить миллисекунды типовым значением, если поле пустое.
    unsafe fn prefill_millis(&self) {
        if !get_text(self.controls.millis).trim().is_empty() {
            return;
        }
        let default = match combo_index(self.controls.trigger) {
            2 => "500",
            3 => "300",
            4 => "150",
            _ => "",
        };
        if !default.is_empty() {
            set_text(self.controls.millis, default);
        }
    }

    unsafe fn on_timer(&mut self) {
        if !self.capturing_pad {
            return;
        }
        let captured = {
            let Ok(mut guard) = self.shared.lock() else {
                return;
            };
            guard
                .capture
                .as_mut()
                .and_then(|capture| capture.captured.take())
        };
        if let Some(button) = captured {
            self.capturing_pad = false;
            set_text(self.controls.pad_field, button.token());
            if let Ok(mut guard) = self.shared.lock() {
                guard.capture = None;
            }
            self.apply_action_visibility();
        }
    }

    fn handle_command(&mut self, id: u16, code: u16) {
        match (id, code) {
            (IDC_PAD_CAPTURE, 0) => self.start_pad_capture(),
            (IDC_KEYS_CAPTURE, 0) => unsafe { self.start_key_capture() },
            (IDC_TRIGGER, CBN_SELCHANGE_CODE) => unsafe {
                self.prefill_millis();
                self.apply_action_visibility();
            },
            (IDC_ACTION, CBN_SELCHANGE_CODE) => unsafe { self.apply_action_visibility() },
            (IDC_OK, 0) => unsafe { self.collect() },
            (IDC_CANCEL, 0) => self.cancel(),
            _ => {}
        }
    }

    fn start_pad_capture(&mut self) {
        self.capturing_pad = true;
        unsafe {
            set_text(self.controls.pad_field, "");
            set_text(
                self.controls.hint,
                "Нажмите любую кнопку на геймпаде… Привязки временно не выполняются. Отмена — «Отмена».",
            );
        }
        if let Ok(mut guard) = self.shared.lock() {
            guard.capture = Some(Capture::default());
        }
    }

    unsafe fn start_key_capture(&mut self) {
        self.capturing_keys = true;
        self.keys_before_capture = get_text(self.controls.value_keys);
        set_text(self.controls.value_keys, "");
        set_text(
            self.controls.hint,
            "Нажмите комбинацию на клавиатуре. Enter — готово, Esc — отмена.",
        );
        SetFocus(self.hwnd);
    }

    /// Обработка WM_KEYDOWN во время захвата комбинации.
    unsafe fn on_key_capture(&mut self, vk: u16) {
        if vk == VK_ESCAPE {
            self.capturing_keys = false;
            set_text(self.controls.value_keys, &self.keys_before_capture.clone());
            self.apply_action_visibility();
            return;
        }
        if vk == VK_RETURN {
            self.capturing_keys = false;
            self.apply_action_visibility();
            return;
        }
        let mut combo = active_modifiers();
        if !is_modifier(vk) {
            combo.push(vk);
            self.capturing_keys = false;
        }
        set_text(self.controls.value_keys, &format_combo(&combo));
        if !self.capturing_keys {
            self.apply_action_visibility();
        }
    }

    fn cancel(&mut self) {
        if let Ok(mut guard) = self.shared.lock() {
            guard.capture = None;
        }
        unsafe { DestroyWindow(self.hwnd) };
    }

    fn finish_ok(&mut self, binding: Binding) {
        if let Ok(mut guard) = self.shared.lock() {
            guard.capture = None;
        }
        self.result = Some(binding);
        unsafe { DestroyWindow(self.hwnd) };
    }

    unsafe fn collect(&mut self) {
        if let Some(binding) = self.try_collect() {
            self.finish_ok(binding);
        }
    }

    unsafe fn try_collect(&mut self) -> Option<Binding> {
        let button_text = get_text(self.controls.pad_field);
        let Some(button) = GameButton::parse(&button_text) else {
            show_message(
                Some(self.hwnd),
                "Сначала назначьте кнопку: нажмите «Нажать…» и klikните кнопку на геймпаде.",
            );
            return None;
        };
        let millis = |default: u32| -> Option<u32> {
            let text = get_text(self.controls.millis);
            let text = text.trim();
            if text.is_empty() {
                return Some(default);
            }
            match text.parse::<u32>() {
                Ok(0) | Err(_) => {
                    show_message(
                        Some(self.hwnd),
                        "Время события — целое число миллисекунд больше нуля.",
                    );
                    None
                }
                Ok(value) => Some(value),
            }
        };
        let trigger = match combo_index(self.controls.trigger) {
            0 => TriggerKind::Press,
            1 => TriggerKind::Release,
            2 => TriggerKind::Hold(millis(500)?),
            3 => TriggerKind::Double(millis(300)?),
            4 => TriggerKind::Repeat(millis(150)?),
            _ => return None,
        };
        let action = match combo_index(self.controls.action) {
            0 => {
                let text = get_text(self.controls.value_keys);
                match parse_combo(&text) {
                    Some(keys) => Action::Keys(keys),
                    None => {
                        show_message(Some(self.hwnd), "Введите комбинацию (например CTRL+C) или захватите её кнопкой «С клавиатуры».");
                        return None;
                    }
                }
            }
            1 => {
                let text = get_text(self.controls.value_text);
                if text.is_empty() {
                    show_message(Some(self.hwnd), "Введите текст.");
                    return None;
                }
                Action::Text(text)
            }
            2 => {
                const ORDER: [MouseButton; 5] = [
                    MouseButton::Left,
                    MouseButton::Right,
                    MouseButton::Middle,
                    MouseButton::X1,
                    MouseButton::X2,
                ];
                let index = combo_index(self.controls.value_mouse);
                if !(0..5).contains(&index) {
                    show_message(Some(self.hwnd), "Выберите кнопку мыши.");
                    return None;
                }
                Action::Mouse(ORDER[index as usize])
            }
            3 | 4 => {
                let text = get_text(self.controls.value_wheel);
                match text.trim().parse::<i32>() {
                    Ok(0) | Err(_) => {
                        show_message(Some(self.hwnd), "Деления колеса — целое число больше или меньше нуля, например 3 или -2.");
                        return None;
                    }
                    Ok(notches) => Action::Wheel {
                        notches,
                        horizontal: combo_index(self.controls.action) == 4,
                    },
                }
            }
            5 => {
                let text = get_text(self.controls.value_macro);
                match Action::parse(&format!("macro:{text}")) {
                    Ok(action) => action,
                    Err(problem) => {
                        show_message(Some(self.hwnd), &format!("Макро не разобран: {problem}\nФормат: keys:CTRL+A;delay:200;text:привет"));
                        return None;
                    }
                }
            }
            _ => return None,
        };
        let rumble = is_checked(self.controls.rumble);
        Some(Binding {
            button,
            trigger,
            action,
            rumble,
        })
    }
}

/// Какие модификаторы зажаты прямо сейчас (Ctrl, Alt, Shift, Win).
unsafe fn active_modifiers() -> Vec<u16> {
    modifier_order()
        .into_iter()
        .filter(|(vk, _)| GetKeyState(*vk as i32) as u16 & 0x8000 != 0)
        .map(|(vk, _)| vk)
        .collect()
}

unsafe extern "system" fn binding_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_NCCREATE {
        let create = &*(lparam as *const CREATESTRUCTW);
        let state = create.lpCreateParams as *mut BindingDialog;
        if !state.is_null() {
            (*state).hwnd = hwnd;
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, state as isize);
        }
    }
    let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut BindingDialog;
    match msg {
        WM_CREATE => {
            if let Some(state) = state_ptr.as_mut() {
                if !state.create_controls() {
                    return -1;
                }
                state.apply_existing();
                state.apply_action_visibility();
                SetTimer(hwnd, TIMER_DLG, TIMER_DLG_MS, None);
                SetFocus(state.controls.pad_capture);
            }
            0
        }
        WM_TIMER => {
            if let Some(state) = state_ptr.as_mut() {
                state.on_timer();
            }
            0
        }
        WM_COMMAND => {
            if let Some(state) = state_ptr.as_mut() {
                let id = (wparam & 0xffff) as u16;
                let code = ((wparam >> 16) & 0xffff) as u16;
                state.handle_command(id, code);
            }
            0
        }
        WM_KEYDOWN => {
            if let Some(state) = state_ptr.as_mut() {
                if state.capturing_keys {
                    state.on_key_capture(wparam as u16);
                    return 0;
                }
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_CTLCOLOREDIT => {
            if let Some(state) = state_ptr.as_ref() {
                let dc = wparam as HDC;
                SetTextColor(dc, INK);
                SetBkColor(dc, WHITE);
                return state.white_brush as LRESULT;
            }
            0
        }
        WM_CTLCOLORSTATIC => {
            let dc = wparam as HDC;
            SetTextColor(dc, INK);
            SetBkColor(dc, BACKGROUND);
            return GetStockObject(NULL_BRUSH) as LRESULT;
        }
        WM_DRAWITEM => {
            if let Some(state) = state_ptr.as_ref() {
                let item = &*(lparam as *const DRAWITEMSTRUCT);
                let style = if item.CtlID == IDC_OK as u32 {
                    ButtonStyle::Primary
                } else {
                    ButtonStyle::Normal
                };
                paint_owner_button(
                    item.hDC,
                    item.rcItem,
                    &get_text(item.hwndItem),
                    &state.fonts,
                    state.dpi,
                    style,
                    item.itemState & ODS_SELECTED_FLAG != 0,
                    item.itemState & ODS_DISABLED_FLAG != 0,
                    item.itemState & ODS_FOCUS_FLAG != 0,
                );
                return 1;
            }
            0
        }
        WM_CLOSE => {
            if let Some(state) = state_ptr.as_mut() {
                state.cancel();
            }
            0
        }
        WM_DESTROY => {
            KillTimer(hwnd, TIMER_DLG);
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

// ─────────────────────────── редактор настроек ───────────────────────────

const IDC_LEFT_MODE: u16 = 2101;
const IDC_LEFT_SENS: u16 = 2102;
const IDC_RIGHT_MODE: u16 = 2103;
const IDC_RIGHT_SENS: u16 = 2104;
const IDC_SCROLL: u16 = 2105;
const IDC_SCROLL_SPEED: u16 = 2106;
const IDC_DEADZONE: u16 = 2107;
const IDC_RUMBLE_ALL: u16 = 2108;
const IDC_AUTOSTART: u16 = 2109;
const IDC_RESET_BINDINGS: u16 = 2110;
const IDC_OK_S: u16 = 2111;
const IDC_CANCEL_S: u16 = 2112;

/// Открывает диалог настроек; возвращает результат или None (отмена).
pub fn run_settings_dialog(
    owner: HWND,
    dpi: i32,
    shared: Arc<Mutex<Shared>>,
) -> Option<SettingsOutcome> {
    unsafe {
        static REGISTER: Once = Once::new();
        let instance = GetModuleHandleW(std::ptr::null());
        let class_name = wide("XboxDeskSettingsDialog");
        REGISTER.call_once(|| {
            let wc = WNDCLASSW {
                lpfnWndProc: Some(settings_wnd_proc),
                hInstance: instance,
                hCursor: LoadCursorW(std::ptr::null_mut(), IDC_ARROW),
                hIcon: LoadIconW(instance, 1usize as *const u16),
                hbrBackground: CreateSolidBrush(BACKGROUND),
                lpszClassName: class_name.as_ptr(),
                ..std::mem::zeroed()
            };
            RegisterClassW(&wc);
        });

        let mut state = Box::new(SettingsDialog::new(dpi, shared));
        let (x, y, w, h) = place_over(owner, state.p(DLG_W), state.p(DLG_H));
        let dlg = CreateWindowExW(
            0,
            class_name.as_ptr(),
            wide("Настройки").as_ptr(),
            DLG_STYLE | WS_VISIBLE,
            x,
            y,
            w,
            h,
            owner,
            std::ptr::null_mut(),
            instance,
            (&mut *state as *mut SettingsDialog).cast(),
        );
        if dlg.is_null() {
            show_message(Some(owner), "Не удалось открыть окно настроек.");
            return None;
        }
        modal_loop(dlg, owner, || false);
        state.result.take()
    }
}

#[derive(Default)]
struct SettingsControls {
    left_mode: HWND,
    left_sens: HWND,
    right_mode: HWND,
    right_sens: HWND,
    scroll: HWND,
    scroll_speed: HWND,
    deadzone: HWND,
    rumble: HWND,
    autostart: HWND,
    reset: HWND,
    ok: HWND,
    cancel: HWND,
}

struct SettingsDialog {
    hwnd: HWND,
    dpi: i32,
    fonts: Fonts,
    shared: Arc<Mutex<Shared>>,
    controls: SettingsControls,
    white_brush: HBRUSH,
    reset_requested: bool,
    result: Option<SettingsOutcome>,
}

impl Drop for SettingsDialog {
    fn drop(&mut self) {
        unsafe { DeleteObject(self.white_brush) };
    }
}

impl SettingsDialog {
    fn new(dpi: i32, shared: Arc<Mutex<Shared>>) -> Self {
        Self {
            hwnd: std::ptr::null_mut(),
            dpi,
            fonts: unsafe { Fonts::new(dpi) },
            shared,
            controls: SettingsControls::default(),
            white_brush: unsafe { CreateSolidBrush(WHITE) },
            reset_requested: false,
            result: None,
        }
    }

    fn p(&self, n: i32) -> i32 {
        n * self.dpi / 96
    }

    unsafe fn create_controls(&mut self) -> bool {
        let dpi = self.dpi;
        let body = self.fonts.body;
        let small = self.fonts.small;
        let hwnd = self.hwnd;
        let child =
            |class: &str, text: &str, id: u16, x: i32, y: i32, w: i32, h: i32, style: u32| {
                dialog_child(hwnd, class, text, id, dpi, x, y, w, h, style, body)
            };
        let label = |text: &str, id: u16, x: i32, y: i32, w: i32| {
            dialog_child(
                hwnd,
                "STATIC",
                text,
                id,
                dpi,
                x,
                y,
                w,
                18,
                SS_LEFT_STYLE,
                small,
            )
        };
        let c = &mut self.controls;
        dialog_child(
            hwnd,
            "STATIC",
            "Левый стик",
            9101,
            dpi,
            20,
            16,
            200,
            18,
            SS_LEFT_STYLE,
            body,
        );
        c.left_mode = child(
            "COMBOBOX",
            "",
            IDC_LEFT_MODE,
            20,
            38,
            240,
            220,
            WS_TABSTOP | CBS_DROPDOWNLIST | WS_VSCROLL,
        );
        label("чувствительность 1–30", 9102, 272, 44, 130);
        c.left_sens = child(
            "EDIT",
            "",
            IDC_LEFT_SENS,
            400,
            38,
            60,
            28,
            ES_AUTOHSCROLL | WS_TABSTOP | WS_BORDER,
        );
        dialog_child(
            hwnd,
            "STATIC",
            "Правый стик",
            9103,
            dpi,
            20,
            84,
            200,
            18,
            SS_LEFT_STYLE,
            body,
        );
        c.right_mode = child(
            "COMBOBOX",
            "",
            IDC_RIGHT_MODE,
            20,
            106,
            240,
            220,
            WS_TABSTOP | CBS_DROPDOWNLIST | WS_VSCROLL,
        );
        label("чувствительность 1–30", 9104, 272, 112, 130);
        c.right_sens = child(
            "EDIT",
            "",
            IDC_RIGHT_SENS,
            400,
            106,
            60,
            28,
            ES_AUTOHSCROLL | WS_TABSTOP | WS_BORDER,
        );
        for combo in [c.left_mode, c.right_mode] {
            for mode in StickMode::ALL {
                SendMessageW(
                    combo,
                    CB_ADDSTRING,
                    0,
                    wide(mode.label()).as_ptr() as LPARAM,
                );
            }
        }
        c.scroll = child(
            "BUTTON",
            "Триггеры LT/RT крутят колесо прокрутки",
            IDC_SCROLL,
            20,
            152,
            440,
            24,
            WS_TABSTOP | BS_AUTOCHECKBOX,
        );
        label("скорость колеса 1–10", 9105, 20, 190, 200);
        c.scroll_speed = child(
            "EDIT",
            "",
            IDC_SCROLL_SPEED,
            230,
            184,
            60,
            28,
            ES_AUTOHSCROLL | WS_TABSTOP | WS_BORDER,
        );
        label("мертвая зона стиков, % 0–40", 9106, 20, 226, 240);
        c.deadzone = child(
            "EDIT",
            "",
            IDC_DEADZONE,
            270,
            220,
            60,
            28,
            ES_AUTOHSCROLL | WS_TABSTOP | WS_BORDER,
        );
        c.rumble = child(
            "BUTTON",
            "Вибрация — мастер-выключатель",
            IDC_RUMBLE_ALL,
            20,
            258,
            440,
            24,
            WS_TABSTOP | BS_AUTOCHECKBOX,
        );
        c.autostart = child(
            "BUTTON",
            "Запускать движок вместе с программой",
            IDC_AUTOSTART,
            20,
            290,
            440,
            24,
            WS_TABSTOP | BS_AUTOCHECKBOX,
        );
        c.reset = child(
            "BUTTON",
            "Сбросить привязки на стандартные",
            IDC_RESET_BINDINGS,
            20,
            330,
            300,
            36,
            WS_TABSTOP | BS_OWNERDRAW,
        );
        c.ok = child(
            "BUTTON",
            "Сохранить",
            IDC_OK_S,
            200,
            386,
            120,
            40,
            WS_TABSTOP | BS_OWNERDRAW,
        );
        c.cancel = child(
            "BUTTON",
            "Отмена",
            IDC_CANCEL_S,
            330,
            386,
            130,
            40,
            WS_TABSTOP | BS_OWNERDRAW,
        );
        for edit in [c.left_sens, c.right_sens, c.scroll_speed, c.deadzone] {
            SendMessageW(edit, EM_LIMITTEXT, 3, 0);
        }
        let created = [
            c.left_mode,
            c.left_sens,
            c.right_mode,
            c.right_sens,
            c.scroll,
            c.scroll_speed,
            c.deadzone,
            c.rumble,
            c.autostart,
            c.reset,
            c.ok,
            c.cancel,
        ]
        .iter()
        .all(|h| !h.is_null());
        self.fill_values();
        created
    }

    unsafe fn fill_values(&self) {
        let settings = {
            let Ok(guard) = self.shared.lock() else {
                return;
            };
            guard.cfg.settings.clone()
        };
        let c = &self.controls;
        let select_mode = |hwnd: HWND, mode: StickMode| {
            let index = StickMode::ALL.iter().position(|m| *m == mode).unwrap_or(0);
            SendMessageW(hwnd, CB_SETCURSEL, index, 0);
        };
        select_mode(c.left_mode, settings.left_stick);
        select_mode(c.right_mode, settings.right_stick);
        set_text(c.left_sens, &settings.left_sensitivity.to_string());
        set_text(c.right_sens, &settings.right_sensitivity.to_string());
        set_text(c.scroll_speed, &settings.scroll_speed.to_string());
        set_text(c.deadzone, &settings.deadzone_pct.to_string());
        let check = |hwnd: HWND, on: bool| {
            SendMessageW(
                hwnd,
                BM_SETCHECK,
                (if on { BST_CHECKED } else { BST_UNCHECKED }) as WPARAM,
                0,
            )
        };
        check(c.scroll, settings.trigger_scroll);
        check(c.rumble, settings.rumble);
        check(c.autostart, settings.auto_start);
        EnableWindow(c.scroll_speed, i32::from(settings.trigger_scroll));
    }

    fn handle_command(&mut self, id: u16) {
        match id {
            IDC_SCROLL => unsafe {
                EnableWindow(
                    self.controls.scroll_speed,
                    i32::from(is_checked(self.controls.scroll)),
                );
            },
            IDC_RESET_BINDINGS => {
                if !self.reset_requested
                    && confirm(
                        Some(self.hwnd),
                        "Заменить все привязки стандартным набором? Текущие будут потеряны.",
                    )
                {
                    self.reset_requested = true;
                    unsafe {
                        set_text(self.controls.reset, "Привязки будут сброшены ✓")
                    };
                }
            }
            IDC_OK_S => unsafe { self.collect() },
            IDC_CANCEL_S => unsafe {
                DestroyWindow(self.hwnd);
            },
            _ => {}
        }
    }

    unsafe fn collect(&mut self) {
        if let Some(outcome) = self.try_collect() {
            self.result = Some(outcome);
            DestroyWindow(self.hwnd);
        }
    }

    unsafe fn try_collect(&mut self) -> Option<SettingsOutcome> {
        let number = |hwnd: HWND, what: &str| -> Option<u32> {
            match get_text(hwnd).trim().parse::<u32>() {
                Ok(value) => Some(value),
                Err(_) => {
                    show_message(Some(self.hwnd), &format!("{what} — целое число."));
                    None
                }
            }
        };
        let mode = |hwnd: HWND| -> StickMode {
            let index = combo_index(hwnd).clamp(0, StickMode::ALL.len() as i32 - 1);
            StickMode::ALL[index as usize]
        };
        let settings = Settings {
            left_stick: mode(self.controls.left_mode),
            left_sensitivity: number(self.controls.left_sens, "Чувствительность левого стика")?,
            right_stick: mode(self.controls.right_mode),
            right_sensitivity: number(self.controls.right_sens, "Чувствительность правого стика")?,
            trigger_scroll: is_checked(self.controls.scroll),
            scroll_speed: number(self.controls.scroll_speed, "Скорость колеса")?,
            deadzone_pct: number(self.controls.deadzone, "Мертвая зона")?,
            rumble: is_checked(self.controls.rumble),
            auto_start: is_checked(self.controls.autostart),
        }
        .clamped();
        Some(SettingsOutcome {
            settings,
            reset_bindings: self.reset_requested,
        })
    }
}

unsafe extern "system" fn settings_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_NCCREATE {
        let create = &*(lparam as *const CREATESTRUCTW);
        let state = create.lpCreateParams as *mut SettingsDialog;
        if !state.is_null() {
            (*state).hwnd = hwnd;
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, state as isize);
        }
    }
    let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut SettingsDialog;
    match msg {
        WM_CREATE => {
            if let Some(state) = state_ptr.as_mut() {
                if !state.create_controls() {
                    return -1;
                }
                SetFocus(state.controls.ok);
            }
            0
        }
        WM_COMMAND => {
            if let Some(state) = state_ptr.as_mut() {
                let id = (wparam & 0xffff) as u16;
                let code = ((wparam >> 16) & 0xffff) as u16;
                if code == 0 {
                    state.handle_command(id);
                }
            }
            0
        }
        WM_CTLCOLOREDIT => {
            if let Some(state) = state_ptr.as_ref() {
                let dc = wparam as HDC;
                SetTextColor(dc, INK);
                SetBkColor(dc, WHITE);
                return state.white_brush as LRESULT;
            }
            0
        }
        WM_CTLCOLORSTATIC => {
            let dc = wparam as HDC;
            SetTextColor(dc, INK);
            SetBkColor(dc, BACKGROUND);
            return GetStockObject(NULL_BRUSH) as LRESULT;
        }
        WM_DRAWITEM => {
            if let Some(state) = state_ptr.as_ref() {
                let item = &*(lparam as *const DRAWITEMSTRUCT);
                let style = if item.CtlID == IDC_OK_S as u32 {
                    ButtonStyle::Primary
                } else {
                    ButtonStyle::Normal
                };
                paint_owner_button(
                    item.hDC,
                    item.rcItem,
                    &get_text(item.hwndItem),
                    &state.fonts,
                    state.dpi,
                    style,
                    item.itemState & ODS_SELECTED_FLAG != 0,
                    item.itemState & ODS_DISABLED_FLAG != 0,
                    item.itemState & ODS_FOCUS_FLAG != 0,
                );
                return 1;
            }
            0
        }
        WM_CLOSE => {
            DestroyWindow(hwnd);
            0
        }
        WM_DESTROY => {
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
