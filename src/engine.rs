//! Движок: поток опроса геймпада 125 Гц, детектор событий (press/release/
//! hold/double/repeat), стики → мышь/стрелки/WASD, триггеры → колесо,
//! вибрация. Всё общение с GUI — через `Arc<Mutex<Shared>>`.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::thread::{sleep, JoinHandle};
use std::time::{Duration, Instant};

use windows_sys::Win32::System::SystemInformation::GetLocalTime;

use crate::binding::{Action, Binding, GameButton, MacroStep, TriggerKind};
use crate::config::{Config, Settings, StickMode};
use crate::send::Injector;
use crate::xinput::{self, PadState};

/// Период опроса геймпада: 8 мс ≈ 125 Гц.
const TICK: Duration = Duration::from_millis(8);
/// Порог срабатывания аналоговых триггеров как кнопок (0..=255).
const TRIGGER_THRESHOLD: u8 = 128;
/// Сколько строк журнала держим в очереди для GUI.
const LOG_CAPACITY: usize = 400;

/// Захват кнопки геймпада для редактора привязок.
#[derive(Clone, Copy, Debug, Default)]
pub struct Capture {
    pub captured: Option<GameButton>,
}

/// Общее состояние между GUI-потоком и потоком движка.
pub struct Shared {
    pub cfg: Config,
    /// Счетчик изменений cfg — движок по нему пересоздает runtime-состояния.
    pub version: u64,
    /// Выполнять ли привязки (кнопка «Остановить» в GUI).
    pub running: bool,
    /// Команда потоку движка завершиться (и отпустить все клавиши).
    pub stop: bool,
    /// Последний снимок геймпада для визуализации.
    pub pad: PadState,
    /// Some — режим захвата кнопки (движок не выполняет привязки).
    pub capture: Option<Capture>,
    /// Очередь строк журнала для GUI.
    pub log: VecDeque<String>,
}

pub fn new_shared(cfg: Config) -> Arc<Mutex<Shared>> {
    Arc::new(Mutex::new(Shared {
        running: cfg.settings.auto_start,
        cfg,
        version: 1,
        stop: false,
        pad: PadState::default(),
        capture: None,
        log: VecDeque::new(),
    }))
}

/// Запускает поток движка.
pub fn spawn(shared: Arc<Mutex<Shared>>) -> JoinHandle<()> {
    std::thread::Builder::new()
        .name("xboxdesk-engine".into())
        .spawn(move || Engine::new().run(shared))
        .expect("не удалось запустить поток движка")
}

///Runtime-состояние одной привязки (индекс параллелен списку bindings).
#[derive(Default)]
struct RtState {
    down_since: Option<Instant>,
    hold_fired: bool,
    last_release: Option<Instant>,
    next_repeat: Option<Instant>,
}

struct Engine {
    injector: Injector,
    rt: Vec<RtState>,
    rt_version: u64,
    prev_buttons: u16,
    prev_lt: bool,
    prev_rt: bool,
    mouse_acc_l: (f64, f64),
    mouse_acc_r: (f64, f64),
    scroll_acc_lt: f64,
    scroll_acc_rt: f64,
    rumble_until: Option<Instant>,
    battery: Option<u8>,
    next_battery: Instant,
    was_connected: bool,
}

impl Engine {
    fn new() -> Self {
        Self {
            injector: Injector::new(),
            rt: Vec::new(),
            rt_version: 0,
            prev_buttons: 0,
            prev_lt: false,
            prev_rt: false,
            mouse_acc_l: (0.0, 0.0),
            mouse_acc_r: (0.0, 0.0),
            scroll_acc_lt: 0.0,
            scroll_acc_rt: 0.0,
            rumble_until: None,
            battery: None,
            next_battery: Instant::now(),
            was_connected: false,
        }
    }

    fn run(&mut self, shared: Arc<Mutex<Shared>>) {
        loop {
            let tick_start = Instant::now();

            // --- снимок общих данных (короткая блокировка) ---
            let (bindings, settings, running, stop, capture_armed, version) = {
                let Ok(guard) = shared.lock() else { break };
                let s = &guard;
                (
                    s.cfg.bindings.clone(),
                    s.cfg.settings.clone(),
                    s.running,
                    s.stop,
                    s.capture.is_some(),
                    s.version,
                )
            };

            // --- батарея раз в 10 секунд ---
            if tick_start >= self.next_battery {
                self.battery = xinput::battery_percent();
                self.next_battery = tick_start + Duration::from_secs(10);
            }

            // --- опрос геймпада ---
            let mut pad = xinput::poll();
            pad.battery = self.battery.filter(|_| pad.connected);
            if pad.connected != self.was_connected {
                if pad.connected {
                    let battery = pad
                        .battery
                        .map(|p| format!(" · заряд {p}%"))
                        .unwrap_or_default();
                    log(&shared, &format!("Геймпад подключен{battery}."));
                    if settings.rumble {
                        self.pulse_rumble(150);
                    }
                } else {
                    log(&shared, "Геймпад отключен.");
                    self.injector.release_all();
                    xinput::set_rumble(0, 0);
                    self.rumble_until = None;
                }
                self.was_connected = pad.connected;
            }
            {
                if let Ok(mut guard) = shared.lock() {
                    guard.pad = pad;
                }
            }

            if stop {
                self.shutdown(&shared);
                break;
            }

            let lt_down = pad.lt >= TRIGGER_THRESHOLD;
            let rt_down = pad.rt >= TRIGGER_THRESHOLD;

            if capture_armed {
                // Режим назначения кнопки: привязки не выполняем, ловим нажатие.
                self.injector.release_all();
                if let Some(button) = self.newly_pressed(pad.buttons, lt_down, rt_down) {
                    if let Ok(mut guard) = shared.lock() {
                        if let Some(capture) = guard.capture.as_mut() {
                            capture.captured = Some(button);
                        }
                    }
                    if settings.rumble {
                        self.pulse_rumble(90);
                    }
                }
            } else if running && pad.connected {
                if version != self.rt_version {
                    self.rt = bindings.iter().map(|_| RtState::default()).collect();
                    self.rt_version = version;
                    // Привязки поменялись — отпускаем всё удержанное старыми.
                    self.injector.release_all();
                }
                self.dispatch(&shared, &bindings, &settings, pad, lt_down, rt_down);
                self.sticks(&settings, pad);
                self.trigger_scroll(&settings, pad);
            } else {
                self.injector.release_all();
                self.mouse_acc_l = (0.0, 0.0);
                self.mouse_acc_r = (0.0, 0.0);
                self.scroll_acc_lt = 0.0;
                self.scroll_acc_rt = 0.0;
            }

            // --- вибрация: гасим по истечении импульса ---
            if let Some(until) = self.rumble_until {
                if tick_start >= until {
                    xinput::set_rumble(0, 0);
                    self.rumble_until = None;
                }
            }

            self.prev_buttons = pad.buttons;
            self.prev_lt = lt_down;
            self.prev_rt = rt_down;

            let elapsed = tick_start.elapsed();
            if elapsed < TICK {
                sleep(TICK - elapsed);
            }
        }
    }

    fn shutdown(&mut self, shared: &Arc<Mutex<Shared>>) {
        self.injector.release_all();
        xinput::set_rumble(0, 0);
        log(shared, "Движок остановлен, все зажатые клавиши отпущены.");
    }

    fn pulse_rumble(&mut self, millis: u64) {
        xinput::set_rumble(2200, 2200);
        self.rumble_until = Some(Instant::now() + Duration::from_millis(millis));
    }

    /// Какая кнопка только что нажата (для захвата в редакторе).
    fn newly_pressed(&self, buttons: u16, lt_down: bool, rt_down: bool) -> Option<GameButton> {
        GameButton::ALL.into_iter().find(|button| {
            let now = button_down(*button, buttons, lt_down, rt_down);
            let before = button_down(*button, self.prev_buttons, self.prev_lt, self.prev_rt);
            now && !before
        })
    }

    /// Проход по привязкам: детектор событий и выполнение действий.
    fn dispatch(
        &mut self,
        shared: &Arc<Mutex<Shared>>,
        bindings: &[Binding],
        settings: &Settings,
        pad: PadState,
        lt_down: bool,
        rt_down: bool,
    ) {
        let now = Instant::now();
        while self.rt.len() < bindings.len() {
            self.rt.push(RtState::default());
        }
        for (index, binding) in bindings.iter().enumerate() {
            let down = button_down(binding.button, pad.buttons, lt_down, rt_down);
            let fired = {
                let rt = &mut self.rt[index];
                trigger_fired(binding.trigger, rt, down, now)
            };
            if fired {
                self.fire(shared, binding, settings);
            }
        }
    }

    fn fire(&mut self, shared: &Arc<Mutex<Shared>>, binding: &Binding, settings: &Settings) {
        match &binding.action {
            Action::Keys(keys) => self.injector.tap_keys(keys),
            Action::Text(text) => self.injector.tap_text(text),
            Action::Mouse(button) => self.injector.click_mouse(*button),
            Action::Wheel {
                notches,
                horizontal,
            } => self.injector.wheel(*notches, *horizontal),
            Action::Macro(steps) => {
                for step in steps {
                    match step {
                        MacroStep::Keys(keys) => self.injector.tap_keys(keys),
                        MacroStep::Text(text) => self.injector.tap_text(text),
                        MacroStep::Delay(ms) => sleep(Duration::from_millis(*ms as u64)),
                    }
                }
            }
        }
        if binding.rumble && settings.rumble {
            self.pulse_rumble(70);
        }
        log(
            shared,
            &format!(
                "{} · {} → {}",
                binding.button.token(),
                binding.trigger.label(),
                binding.action.label()
            ),
        );
    }

    /// Стики: мышь / стрелки / WASD.
    fn sticks(&mut self, settings: &Settings, pad: PadState) {
        let dz = settings.deadzone_pct as f64 / 100.0;

        // Мышь.
        if settings.left_stick == StickMode::Mouse {
            if let Some((dx, dy)) = stick_to_delta(
                pad.lx,
                pad.ly,
                dz,
                settings.left_sensitivity,
                &mut self.mouse_acc_l,
            ) {
                self.injector.move_mouse(dx, dy);
            }
        }
        if settings.right_stick == StickMode::Mouse {
            if let Some((dx, dy)) = stick_to_delta(
                pad.rx,
                pad.ry,
                dz,
                settings.right_sensitivity,
                &mut self.mouse_acc_r,
            ) {
                self.injector.move_mouse(dx, dy);
            }
        }

        // Клавиши-направления: объединяем желания обоих стиков, чтобы не
        // конфликтовали set_key/release.
        let wanted: Vec<(u16, bool)> = {
            let mut wanted = Vec::with_capacity(8);
            for (mode, x, y, sens) in [
                (
                    settings.left_stick,
                    pad.lx,
                    pad.ly,
                    settings.left_sensitivity,
                ),
                (
                    settings.right_stick,
                    pad.rx,
                    pad.ry,
                    settings.right_sensitivity,
                ),
            ] {
                let _ = sens;
                for (vk, on) in direction_keys(mode, x, y, dz) {
                    if let Some(existing) = wanted.iter_mut().find(|(k, _)| *k == vk) {
                        existing.1 |= on;
                    } else {
                        wanted.push((vk, on));
                    }
                }
            }
            wanted
        };
        for (vk, on) in wanted {
            self.injector.set_key(vk, on);
        }
    }

    /// Триггеры → колесо прокрутки (LT вверх, RT вниз).
    fn trigger_scroll(&mut self, settings: &Settings, pad: PadState) {
        if !settings.trigger_scroll {
            self.scroll_acc_lt = 0.0;
            self.scroll_acc_rt = 0.0;
            return;
        }
        let step = settings.scroll_speed as f64 * 0.012;
        self.scroll_acc_lt += pad.lt as f64 / 255.0 * step;
        self.scroll_acc_rt += pad.rt as f64 / 255.0 * step;
        if pad.lt < 8 {
            self.scroll_acc_lt = 0.0;
        }
        if pad.rt < 8 {
            self.scroll_acc_rt = 0.0;
        }
        while self.scroll_acc_lt >= 1.0 {
            self.injector.wheel(1, false);
            self.scroll_acc_lt -= 1.0;
        }
        while self.scroll_acc_rt >= 1.0 {
            self.injector.wheel(-1, false);
            self.scroll_acc_rt -= 1.0;
        }
    }
}

/// Состояние кнопки (цифровой бит или порог триггера).
fn button_down(button: GameButton, mask: u16, lt_down: bool, rt_down: bool) -> bool {
    match button.xinput_mask() {
        Some(bit) => mask & bit != 0,
        None => match button {
            GameButton::LT => lt_down,
            _ => rt_down,
        },
    }
}

/// Конечный автомат события одной привязки: сработало ли действие в этом тике.
fn trigger_fired(trigger: TriggerKind, rt: &mut RtState, down: bool, now: Instant) -> bool {
    let was_down = rt.down_since.is_some();
    let mut fired = false;
    if down && !was_down {
        rt.down_since = Some(now);
        rt.hold_fired = false;
        match trigger {
            TriggerKind::Press => fired = true,
            TriggerKind::Double(window) => {
                if let Some(release) = rt.last_release {
                    if now.duration_since(release) <= Duration::from_millis(window as u64) {
                        fired = true;
                    }
                }
                rt.last_release = None;
            }
            TriggerKind::Repeat(interval) => {
                fired = true;
                rt.next_repeat = Some(now + Duration::from_millis(interval as u64));
            }
            _ => {}
        }
    } else if down && was_down {
        match trigger {
            TriggerKind::Hold(duration) => {
                if !rt.hold_fired {
                    if let Some(since) = rt.down_since {
                        if now.duration_since(since) >= Duration::from_millis(duration as u64) {
                            rt.hold_fired = true;
                            fired = true;
                        }
                    }
                }
            }
            TriggerKind::Repeat(interval) => {
                if let Some(next) = rt.next_repeat {
                    if now >= next {
                        rt.next_repeat = Some(next + Duration::from_millis(interval as u64));
                        fired = true;
                    }
                }
            }
            _ => {}
        }
    } else if !down && was_down {
        rt.down_since = None;
        rt.last_release = Some(now);
        rt.next_repeat = None;
        if trigger == TriggerKind::Release {
            fired = true;
        }
    }
    fired
}

/// Круговая мертвая зона + квадратичная кривая → приращение курсора.
/// Дробная часть накапливается в acc, чтобы медленные движения не терялись.
fn stick_to_delta(
    x: f64,
    y: f64,
    deadzone: f64,
    sensitivity: u32,
    acc: &mut (f64, f64),
) -> Option<(i32, i32)> {
    let magnitude = (x * x + y * y).sqrt();
    if magnitude <= deadzone || magnitude < 1e-9 {
        acc.0 = 0.0;
        acc.1 = 0.0;
        return None;
    }
    let deadzone = deadzone.min(0.95);
    let scaled = ((magnitude - deadzone) / (1.0 - deadzone)).min(1.0);
    // Квадратичная кривая: мелкие движения точные, большие — быстрые.
    let strength = scaled * scaled * sensitivity as f64;
    let ux = x / magnitude;
    let uy = y / magnitude;
    acc.0 += ux * strength;
    acc.1 += -uy * strength; // экранный Y направлен вниз
    let dx = acc.0 as i32;
    let dy = acc.1 as i32;
    acc.0 -= dx as f64;
    acc.1 -= dy as f64;
    if dx == 0 && dy == 0 {
        None
    } else {
        Some((dx, dy))
    }
}

/// Какие клавиши-направления хочет стик в режиме стрелок/WASD.
fn direction_keys(mode: StickMode, x: f64, y: f64, deadzone: f64) -> Vec<(u16, bool)> {
    const THRESHOLD: f64 = 0.5;
    let keys = match mode {
        StickMode::Arrows => [0x26u16, 0x28, 0x25, 0x27], // UP DOWN LEFT RIGHT
        StickMode::Wasd => [0x57, 0x53, 0x41, 0x44],      // W S A D
        StickMode::Off | StickMode::Mouse => return Vec::new(),
    };
    let threshold = THRESHOLD.max(deadzone + 0.1);
    vec![
        (keys[0], y > threshold),
        (keys[1], y < -threshold),
        (keys[2], x < -threshold),
        (keys[3], x > threshold),
    ]
}

/// Строка журнала: в очередь GUI и в консоль (с локальным временем).
pub fn log(shared: &Arc<Mutex<Shared>>, line: &str) {
    if let Ok(mut guard) = shared.lock() {
        while guard.log.len() >= LOG_CAPACITY {
            guard.log.pop_front();
        }
        guard.log.push_back(line.to_string());
    }
    let mut time: windows_sys::Win32::Foundation::SYSTEMTIME = unsafe { std::mem::zeroed() };
    unsafe { GetLocalTime(&mut time) };
    println!(
        "{:02}:{:02}:{:02}  {}",
        time.wHour, time.wMinute, time.wSecond, line
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vk::parse_combo;

    fn binding(trigger: TriggerKind) -> Binding {
        Binding {
            button: GameButton::A,
            trigger,
            action: Action::Keys(parse_combo("SPACE").unwrap()),
            rumble: false,
        }
    }

    #[test]
    fn press_fires_once_on_edge() {
        let mut rt = RtState::default();
        let mut now = Instant::now();
        let b = binding(TriggerKind::Press);
        assert!(!trigger_fired(b.trigger, &mut rt, false, now), "покой");
        now += Duration::from_millis(8);
        assert!(trigger_fired(b.trigger, &mut rt, true, now), "edge");
        now += Duration::from_millis(8);
        assert!(!trigger_fired(b.trigger, &mut rt, true, now), "held");
        assert!(!trigger_fired(b.trigger, &mut rt, false, now), "release");
        now += Duration::from_millis(8);
        assert!(!trigger_fired(b.trigger, &mut rt, false, now), "idle");
    }

    #[test]
    fn release_fires_on_up() {
        let mut rt = RtState::default();
        let mut now = Instant::now();
        let b = binding(TriggerKind::Release);
        assert!(!trigger_fired(b.trigger, &mut rt, false, now));
        now += Duration::from_millis(8);
        assert!(!trigger_fired(b.trigger, &mut rt, true, now));
        now += Duration::from_millis(8);
        assert!(trigger_fired(b.trigger, &mut rt, false, now));
    }

    #[test]
    fn hold_fires_after_duration_once() {
        let mut rt = RtState::default();
        let mut now = Instant::now();
        let b = binding(TriggerKind::Hold(300));
        assert!(!trigger_fired(b.trigger, &mut rt, true, now));
        for _ in 0..36 {
            // 288 мс — еще рано
            now += Duration::from_millis(8);
            assert!(!trigger_fired(b.trigger, &mut rt, true, now));
        }
        now += Duration::from_millis(20);
        assert!(trigger_fired(b.trigger, &mut rt, true, now), "308 мс");
        now += Duration::from_millis(8);
        assert!(!trigger_fired(b.trigger, &mut rt, true, now), "один раз");
        // Отпустили и снова держим — сработает заново.
        assert!(!trigger_fired(b.trigger, &mut rt, false, now));
        now += Duration::from_millis(8);
        assert!(!trigger_fired(b.trigger, &mut rt, true, now));
        now += Duration::from_millis(400);
        assert!(trigger_fired(b.trigger, &mut rt, true, now));
    }

    #[test]
    fn double_fires_on_second_press_within_window() {
        let mut rt = RtState::default();
        let mut now = Instant::now();
        let b = binding(TriggerKind::Double(300));
        assert!(!trigger_fired(b.trigger, &mut rt, true, now), "1-е нажатие");
        now += Duration::from_millis(50);
        assert!(!trigger_fired(b.trigger, &mut rt, false, now), "отпустили");
        now += Duration::from_millis(100);
        assert!(trigger_fired(b.trigger, &mut rt, true, now), "2-е в окне");
        now += Duration::from_millis(50);
        assert!(!trigger_fired(b.trigger, &mut rt, false, now));
        now += Duration::from_millis(400);
        assert!(!trigger_fired(b.trigger, &mut rt, true, now), "поздно");
    }

    #[test]
    fn repeat_fires_immediately_then_every_interval() {
        let mut rt = RtState::default();
        let mut now = Instant::now();
        let b = binding(TriggerKind::Repeat(150));
        assert!(trigger_fired(b.trigger, &mut rt, true, now), "первый сразу");
        now += Duration::from_millis(144);
        assert!(!trigger_fired(b.trigger, &mut rt, true, now));
        now += Duration::from_millis(8);
        assert!(trigger_fired(b.trigger, &mut rt, true, now), "152 мс");
        now += Duration::from_millis(152);
        assert!(trigger_fired(b.trigger, &mut rt, true, now), "еще ~150");
        now += Duration::from_millis(8);
        assert!(!trigger_fired(b.trigger, &mut rt, false, now), "отпустили");
        now += Duration::from_millis(8);
        assert!(
            trigger_fired(b.trigger, &mut rt, true, now),
            "новое нажатие — сразу"
        );
    }

    #[test]
    fn deadzone_and_curve() {
        let mut acc = (0.0, 0.0);
        assert!(stick_to_delta(0.1, 0.05, 0.15, 14, &mut acc).is_none());
        // Полное отклонение вправо: dx положительный, dy ~0.
        let mut acc = (0.0, 0.0);
        let (dx, dy) = stick_to_delta(1.0, 0.0, 0.15, 14, &mut acc).expect("full deflection");
        assert_eq!((dx, dy), (14, 0));
        // Вверх (y=+1) → dy отрицательный (курсор вверх).
        let mut acc = (0.0, 0.0);
        let (dx, dy) = stick_to_delta(0.0, 1.0, 0.15, 10, &mut acc).unwrap();
        assert_eq!((dx, dy), (0, -10));
        // Малое отклонение за мертвой зоной копится дробно и стреливает позже.
        let mut acc = (0.0, 0.0);
        assert!(stick_to_delta(0.3, 0.0, 0.15, 14, &mut acc).is_none());
        assert!(stick_to_delta(0.3, 0.0, 0.15, 14, &mut acc).is_none());
        assert_eq!(stick_to_delta(0.3, 0.0, 0.15, 14, &mut acc), Some((1, 0)));
    }

    #[test]
    fn direction_keys_modes() {
        let off = direction_keys(StickMode::Off, 1.0, 1.0, 0.15);
        assert!(off.is_empty());
        let arrows = direction_keys(StickMode::Arrows, 0.9, -0.9, 0.15);
        assert_eq!(
            arrows,
            vec![(0x26, false), (0x28, true), (0x25, false), (0x27, true)]
        );
        let wasd = direction_keys(StickMode::Wasd, 0.0, 0.9, 0.15);
        assert_eq!(wasd[0], (0x57, true)); // W
        assert!(!wasd[1].1 && !wasd[2].1 && !wasd[3].1);
    }

    #[test]
    fn button_down_handles_triggers() {
        use crate::xinput::XINPUT_GAMEPAD_A;
        assert!(button_down(GameButton::A, XINPUT_GAMEPAD_A, false, false));
        assert!(!button_down(GameButton::A, 0, false, false));
        assert!(button_down(GameButton::LT, 0, true, false));
        assert!(!button_down(GameButton::LT, 0, false, false));
        assert!(button_down(GameButton::RT, 0, false, true));
    }
}
