//! Модель привязок: кнопка геймпада + тип события + действие.
//!
//! Строковый формат (для settings.ini и GUI):
//! `A=press|keys:CTRL+C`, `DPadUp=repeat:150|keys:VOLUME_UP|rumble`,
//! `LT=hold:600|macro:keys:WIN+D;delay:300;text:готово`.

use crate::send::MouseButton;
use crate::vk::{format_combo, parse_combo};
use crate::xinput::{
    XINPUT_GAMEPAD_A, XINPUT_GAMEPAD_B, XINPUT_GAMEPAD_BACK, XINPUT_GAMEPAD_DPAD_DOWN,
    XINPUT_GAMEPAD_DPAD_LEFT, XINPUT_GAMEPAD_DPAD_RIGHT, XINPUT_GAMEPAD_DPAD_UP,
    XINPUT_GAMEPAD_LEFT_SHOULDER, XINPUT_GAMEPAD_LEFT_THUMB, XINPUT_GAMEPAD_RIGHT_SHOULDER,
    XINPUT_GAMEPAD_RIGHT_THUMB, XINPUT_GAMEPAD_START, XINPUT_GAMEPAD_X, XINPUT_GAMEPAD_Y,
};

/// Все «кнопки», которые можно назначить. LT/RT — аналоговые триггеры,
/// движок считает их нажатыми при значении выше порога.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GameButton {
    A,
    B,
    X,
    Y,
    LB,
    RB,
    LT,
    RT,
    Back,
    Start,
    LThumb,
    RThumb,
    DPadUp,
    DPadDown,
    DPadLeft,
    DPadRight,
}

impl GameButton {
    pub const ALL: [GameButton; 16] = [
        GameButton::A,
        GameButton::B,
        GameButton::X,
        GameButton::Y,
        GameButton::LB,
        GameButton::RB,
        GameButton::LT,
        GameButton::RT,
        GameButton::Back,
        GameButton::Start,
        GameButton::LThumb,
        GameButton::RThumb,
        GameButton::DPadUp,
        GameButton::DPadDown,
        GameButton::DPadLeft,
        GameButton::DPadRight,
    ];

    pub fn parse(token: &str) -> Option<Self> {
        Some(
            match token.trim().to_ascii_uppercase().replace('-', "_").as_str() {
                "A" => Self::A,
                "B" => Self::B,
                "X" => Self::X,
                "Y" => Self::Y,
                "LB" | "LEFT_SHOULDER" => Self::LB,
                "RB" | "RIGHT_SHOULDER" => Self::RB,
                "LT" | "LEFT_TRIGGER" => Self::LT,
                "RT" | "RIGHT_TRIGGER" => Self::RT,
                "BACK" | "SELECT" => Self::Back,
                "START" => Self::Start,
                "LTHUMB" | "LS" | "LEFT_THUMB" => Self::LThumb,
                "RTHUMB" | "RS" | "RIGHT_THUMB" => Self::RThumb,
                "DPAD_UP" | "UP" => Self::DPadUp,
                "DPAD_DOWN" | "DOWN" => Self::DPadDown,
                "DPAD_LEFT" | "LEFT" => Self::DPadLeft,
                "DPAD_RIGHT" | "RIGHT" => Self::DPadRight,
                _ => return None,
            },
        )
    }

    pub fn token(&self) -> &'static str {
        match self {
            Self::A => "A",
            Self::B => "B",
            Self::X => "X",
            Self::Y => "Y",
            Self::LB => "LB",
            Self::RB => "RB",
            Self::LT => "LT",
            Self::RT => "RT",
            Self::Back => "BACK",
            Self::Start => "START",
            Self::LThumb => "LTHUMB",
            Self::RThumb => "RTHUMB",
            Self::DPadUp => "DPAD_UP",
            Self::DPadDown => "DPAD_DOWN",
            Self::DPadLeft => "DPAD_LEFT",
            Self::DPadRight => "DPAD_RIGHT",
        }
    }

    /// Маска XInput для цифровых кнопок; None для аналоговых LT/RT.
    pub fn xinput_mask(&self) -> Option<u16> {
        Some(match self {
            Self::A => XINPUT_GAMEPAD_A,
            Self::B => XINPUT_GAMEPAD_B,
            Self::X => XINPUT_GAMEPAD_X,
            Self::Y => XINPUT_GAMEPAD_Y,
            Self::LB => XINPUT_GAMEPAD_LEFT_SHOULDER,
            Self::RB => XINPUT_GAMEPAD_RIGHT_SHOULDER,
            Self::Back => XINPUT_GAMEPAD_BACK,
            Self::Start => XINPUT_GAMEPAD_START,
            Self::LThumb => XINPUT_GAMEPAD_LEFT_THUMB,
            Self::RThumb => XINPUT_GAMEPAD_RIGHT_THUMB,
            Self::DPadUp => XINPUT_GAMEPAD_DPAD_UP,
            Self::DPadDown => XINPUT_GAMEPAD_DPAD_DOWN,
            Self::DPadLeft => XINPUT_GAMEPAD_DPAD_LEFT,
            Self::DPadRight => XINPUT_GAMEPAD_DPAD_RIGHT,
            Self::LT | Self::RT => return None,
        })
    }
}

/// Тип события на кнопке.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TriggerKind {
    /// Мгновенно при нажатии.
    Press,
    /// При отпускании.
    Release,
    /// После удержания N мс (один раз).
    Hold(u32),
    /// Второе нажатие в пределах N мс после отпускания.
    Double(u32),
    /// Сразу при нажатии, затем каждые N мс, пока держат.
    Repeat(u32),
}

impl TriggerKind {
    pub fn parse(token: &str) -> Result<Self, String> {
        let (name, arg) = match token.split_once(':') {
            Some((n, a)) => (n, Some(a)),
            None => (token, None),
        };
        let millis = || -> Result<u32, String> {
            let value: u32 = arg
                .ok_or_else(|| format!("событию `{name}` нужно время в мс"))?
                .trim()
                .parse()
                .map_err(|_| "время в мс должно быть числом".to_string())?;
            if value == 0 {
                return Err("время должно быть больше 0 мс".into());
            }
            Ok(value)
        };
        Ok(match name.trim().to_ascii_lowercase().as_str() {
            "press" => Self::Press,
            "release" => Self::Release,
            "hold" => Self::Hold(millis()?),
            "double" => Self::Double(millis()?),
            "repeat" => Self::Repeat(millis()?),
            other => return Err(format!("неизвестное событие `{other}`")),
        })
    }

    pub fn token(&self) -> String {
        match self {
            Self::Press => "press".into(),
            Self::Release => "release".into(),
            Self::Hold(ms) => format!("hold:{ms}"),
            Self::Double(ms) => format!("double:{ms}"),
            Self::Repeat(ms) => format!("repeat:{ms}"),
        }
    }

    /// Русское имя для списка привязок.
    pub fn label(&self) -> String {
        match self {
            Self::Press => "нажатие".into(),
            Self::Release => "отпускание".into(),
            Self::Hold(ms) => format!("удержание {ms} мс"),
            Self::Double(ms) => format!("двойное ≤{ms} мс"),
            Self::Repeat(ms) => format!("повтор {ms} мс"),
        }
    }
}

/// Шаг макропоследовательности.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MacroStep {
    Keys(Vec<u16>),
    Text(String),
    Delay(u32),
}

impl MacroStep {
    fn parse(token: &str) -> Result<Self, String> {
        let (kind, value) = token
            .split_once(':')
            .ok_or_else(|| format!("шаг макроса без `:` — `{token}`"))?;
        Ok(match kind.trim().to_ascii_lowercase().as_str() {
            "keys" => Self::Keys(
                parse_combo(value).ok_or_else(|| format!("не понял комбинацию `{value}`"))?,
            ),
            "text" => Self::Text(value.to_string()),
            "delay" => Self::Delay(
                value
                    .trim()
                    .parse()
                    .map_err(|_| "задержка должна быть числом мс".to_string())?,
            ),
            other => return Err(format!("неизвестный шаг макроса `{other}`")),
        })
    }

    fn token(&self) -> String {
        match self {
            Self::Keys(keys) => format!("keys:{}", format_combo(keys)),
            Self::Text(text) => format!("text:{text}"),
            Self::Delay(ms) => format!("delay:{ms}"),
        }
    }
}

/// Действие, которое выполняется при срабатывании события.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Keys(Vec<u16>),
    Text(String),
    Mouse(MouseButton),
    Wheel { notches: i32, horizontal: bool },
    Macro(Vec<MacroStep>),
}

impl Action {
    pub fn parse(token: &str) -> Result<Self, String> {
        let (kind, value) = token
            .split_once(':')
            .ok_or_else(|| format!("действию нужно значение через `:` — `{token}`"))?;
        Ok(match kind.trim().to_ascii_lowercase().as_str() {
            "keys" => Self::Keys(
                parse_combo(value).ok_or_else(|| format!("не понял комбинацию `{value}`"))?,
            ),
            "text" => {
                if value.is_empty() {
                    return Err("пустой текст".into());
                }
                Self::Text(value.to_string())
            }
            "mouse" => Self::Mouse(
                MouseButton::parse(value)
                    .ok_or_else(|| format!("неизвестная кнопка мыши `{value}`"))?,
            ),
            "wheel" | "wheelh" => {
                let notches: i32 = value
                    .trim()
                    .parse()
                    .map_err(|_| "число делений колеса должно быть числом".to_string())?;
                if notches == 0 {
                    return Err("колесо: 0 делений ничего не делает".into());
                }
                Self::Wheel {
                    notches,
                    horizontal: kind.trim().eq_ignore_ascii_case("wheelh"),
                }
            }
            "macro" => {
                let steps = value
                    .split(';')
                    .filter(|s| !s.trim().is_empty())
                    .map(MacroStep::parse)
                    .collect::<Result<Vec<_>, _>>()?;
                if steps.is_empty() {
                    return Err("макро без шагов".into());
                }
                Self::Macro(steps)
            }
            other => return Err(format!("неизвестное действие `{other}`")),
        })
    }

    pub fn token(&self) -> String {
        match self {
            Self::Keys(keys) => format!("keys:{}", format_combo(keys)),
            Self::Text(text) => format!("text:{text}"),
            Self::Mouse(button) => format!("mouse:{}", button.token()),
            Self::Wheel {
                notches,
                horizontal,
            } => format!(
                "{}:{}",
                if *horizontal { "wheelh" } else { "wheel" },
                notches
            ),
            Self::Macro(steps) => format!(
                "macro:{}",
                steps
                    .iter()
                    .map(MacroStep::token)
                    .collect::<Vec<_>>()
                    .join(";")
            ),
        }
    }

    /// Русское имя для списка привязок.
    pub fn label(&self) -> String {
        match self {
            Self::Keys(keys) => format_combo(keys),
            Self::Text(text) => {
                let short: String = text.chars().take(18).collect();
                if short.len() < text.len() {
                    format!("текст «{short}…»")
                } else {
                    format!("текст «{short}»")
                }
            }
            Self::Mouse(button) => button.label().to_string(),
            Self::Wheel {
                notches,
                horizontal,
            } => {
                let dir = if *horizontal {
                    if *notches > 0 {
                        "вправо"
                    } else {
                        "влево"
                    }
                } else if *notches > 0 {
                    "вверх"
                } else {
                    "вниз"
                };
                format!("колесо {dir} ×{}", notches.abs())
            }
            Self::Macro(steps) => format!("макро ({} шаг.)", steps.len()),
        }
    }
}

/// Полная привязка: кнопка + событие + действие + вибрация.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Binding {
    pub button: GameButton,
    pub trigger: TriggerKind,
    pub action: Action,
    pub rumble: bool,
}

impl Binding {
    /// Разбор строки `A=press|keys:CTRL+C|rumble`.
    pub fn parse_line(line: &str) -> Result<Self, String> {
        let (button, rest) = line
            .split_once('=')
            .ok_or_else(|| format!("нет `=` в строке `{line}`"))?;
        let button = GameButton::parse(button)
            .ok_or_else(|| format!("неизвестная кнопка геймпада `{button}`"))?;
        let mut segments = rest.split('|');
        let trigger_token = segments
            .next()
            .ok_or_else(|| format!("нет события в `{line}`"))?;
        let action_token = segments
            .next()
            .ok_or_else(|| format!("нет действия в `{line}`"))?;
        let trigger = TriggerKind::parse(trigger_token)?;
        let action = Action::parse(action_token)?;
        let mut rumble = false;
        for extra in segments {
            match extra.trim().to_ascii_lowercase().as_str() {
                "rumble" => rumble = true,
                "" => {}
                other => return Err(format!("непонятный хвост `{other}` в `{line}`")),
            }
        }
        Ok(Self {
            button,
            trigger,
            action,
            rumble,
        })
    }

    pub fn to_line(&self) -> String {
        let mut line = format!(
            "{}={}|{}",
            self.button.token(),
            self.trigger.token(),
            self.action.token()
        );
        if self.rumble {
            line.push_str("|rumble");
        }
        line
    }

    /// Короткое русское описание для списка и журнала:
    /// «A · нажатие → ЛКМ», с хвостом « · вибро» при включенной вибрации.
    pub fn display_plain(&self) -> String {
        let mut text = format!(
            "{} · {} → {}",
            self.button.token(),
            self.trigger.label(),
            self.action.label()
        );
        if self.rumble {
            text.push_str(" · вибро");
        }
        text
    }
}

/// Стартовая «игровая» раскладка, переложенная на действия Windows:
/// A — подтвердить (клик), B — контекстное меню, Y — отмена, бамперы —
/// переключение окон как смена снаряжения, RB — представление задач как
/// «карта», Back — «выход в меню» (показать рабочий стол), Start — ввод,
/// крестовина — громкость и медиа, стики-клики — пауза и TAB, долгое
/// нажатие триггеров — снимок экрана и блокировка (короткое — колесо).
pub fn default_bindings() -> Vec<Binding> {
    let b = |button: GameButton, trigger: TriggerKind, action: Action| Binding {
        button,
        trigger,
        action,
        rumble: false,
    };
    let keys = |s: &str| Action::Keys(parse_combo(s).expect("дефолтная комбинация корректна"));
    vec![
        // ABXY — классика: A подтвердить, B назад/меню.
        b(
            GameButton::A,
            TriggerKind::Press,
            Action::Mouse(MouseButton::Left),
        ),
        b(
            GameButton::B,
            TriggerKind::Press,
            Action::Mouse(MouseButton::Right),
        ),
        b(
            GameButton::X,
            TriggerKind::Press,
            Action::Mouse(MouseButton::Middle),
        ),
        b(GameButton::Y, TriggerKind::Press, keys("ESC")),
        // Бамперы — переключение, как смена оружия/снаряжения.
        b(GameButton::LB, TriggerKind::Press, keys("ALT+TAB")),
        b(GameButton::RB, TriggerKind::Press, keys("WIN+TAB")),
        // Кнопки меню: Back — «в меню» (рабочий стол), Start — ввод.
        b(GameButton::Back, TriggerKind::Press, keys("WIN+D")),
        b(GameButton::Start, TriggerKind::Press, keys("ENTER")),
        // Крестовина — медиа и громкость (как радио).
        b(
            GameButton::DPadUp,
            TriggerKind::Repeat(150),
            keys("VOLUME_UP"),
        ),
        b(
            GameButton::DPadDown,
            TriggerKind::Repeat(150),
            keys("VOLUME_DOWN"),
        ),
        b(GameButton::DPadLeft, TriggerKind::Press, keys("MEDIA_PREV")),
        b(
            GameButton::DPadRight,
            TriggerKind::Press,
            keys("MEDIA_NEXT"),
        ),
        // Клики стиков: L3 — пауза, R3 — переход по элементам.
        b(
            GameButton::LThumb,
            TriggerKind::Press,
            keys("MEDIA_PLAY_PAUSE"),
        ),
        b(GameButton::RThumb, TriggerKind::Press, keys("TAB")),
        // Долгое нажатие триггеров — системные действия (короткое — колесо).
        b(GameButton::LT, TriggerKind::Hold(800), keys("WIN+L")),
        b(GameButton::RT, TriggerKind::Hold(800), keys("PRINTSCREEN")),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_roundtrip() {
        let lines = [
            "A=press|mouse:left",
            "DPAD_UP=repeat:150|keys:VOLUME_UP|rumble",
            "LT=hold:600|keys:WIN+L",
            "Y=double:300|text:привет",
            "START=release|wheel:-2",
            "X=press|macro:keys:CTRL+A;delay:200;text:ок;keys:ENTER",
            "B=press|wheelh:1",
        ];
        for line in lines {
            let binding = Binding::parse_line(line).unwrap_or_else(|e| panic!("{line}: {e}"));
            assert_eq!(
                binding.to_line().to_ascii_uppercase(),
                line.to_ascii_uppercase(),
                "{line}"
            );
        }
    }

    #[test]
    fn parse_errors() {
        assert!(Binding::parse_line("A=press").is_err());
        assert!(Binding::parse_line("NOPE=press|keys:A").is_err());
        assert!(Binding::parse_line("A=hold|keys:A").is_err());
        assert!(Binding::parse_line("A=press|keys:").is_err());
        assert!(Binding::parse_line("A=press|mouse:tail").is_err());
    }

    #[test]
    fn display_readable() {
        let binding = Binding::parse_line("DPAD_UP=repeat:150|keys:VOLUME_UP").unwrap();
        assert_eq!(
            binding.display_plain(),
            "DPAD_UP · повтор 150 мс → VOLUME_UP"
        );
        let rumble = Binding::parse_line("A=press|mouse:left|rumble").unwrap();
        assert_eq!(rumble.display_plain(), "A · нажатие → ЛКМ · вибро");
    }

    #[test]
    fn defaults_are_valid() {
        for binding in default_bindings() {
            let line = binding.to_line();
            assert_eq!(Binding::parse_line(&line), Ok(binding), "{line}");
        }
    }
}
