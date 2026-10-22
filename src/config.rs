//! Настройки и привязки: загрузка/сохранение `settings.ini` в `%APPDATA%\XboxDesk`.
//!
//! Формат — простой INI (как gui.ini в alibi-quic): секции `[settings]` и
//! `[bindings]`, по одной привязке на строку. Битые строки не роняют загрузку —
//! они попадают в список предупреждений и показываются в журнале GUI.

use std::fs;
use std::path::{Path, PathBuf};

use crate::binding::{default_bindings, Binding};

/// Режим работы стика.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StickMode {
    Off,
    Mouse,
    Arrows,
    Wasd,
}

impl StickMode {
    pub fn parse(token: &str) -> Option<Self> {
        Some(match token.trim().to_ascii_lowercase().as_str() {
            "off" | "none" => Self::Off,
            "mouse" => Self::Mouse,
            "arrows" | "arrow" => Self::Arrows,
            "wasd" => Self::Wasd,
            _ => return None,
        })
    }

    pub fn token(&self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Mouse => "mouse",
            Self::Arrows => "arrows",
            Self::Wasd => "wasd",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Off => "выключен",
            Self::Mouse => "мышь",
            Self::Arrows => "стрелки",
            Self::Wasd => "WASD",
        }
    }

    pub const ALL: [StickMode; 4] = [Self::Mouse, Self::Arrows, Self::Wasd, Self::Off];
}

/// Глобальные параметры движения/скролла/вибрации.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    pub left_stick: StickMode,
    /// 1..=30, пикселей за опрос при полном отклонении.
    pub left_sensitivity: u32,
    pub right_stick: StickMode,
    pub right_sensitivity: u32,
    /// LT/RT крутят колесо прокрутки.
    pub trigger_scroll: bool,
    /// 1..=10, скорость накопления делений колеса.
    pub scroll_speed: u32,
    /// Мертвая зона стиков, % от радиуса (0..=40).
    pub deadzone_pct: u32,
    /// Глобальная вибрация (мастер-выключатель).
    pub rumble: bool,
    /// Запускать движок сразу при старте программы.
    pub auto_start: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            left_stick: StickMode::Mouse,
            left_sensitivity: 14,
            right_stick: StickMode::Arrows,
            right_sensitivity: 10,
            trigger_scroll: true,
            scroll_speed: 4,
            deadzone_pct: 15,
            rumble: true,
            auto_start: true,
        }
    }
}

impl Settings {
    /// Ограничение значений разумными пределами (защита от ручных правок ini).
    pub fn clamped(mut self) -> Self {
        self.left_sensitivity = self.left_sensitivity.clamp(1, 30);
        self.right_sensitivity = self.right_sensitivity.clamp(1, 30);
        self.scroll_speed = self.scroll_speed.clamp(1, 10);
        self.deadzone_pct = self.deadzone_pct.min(40);
        self
    }
}

/// Всё содержимое settings.ini.
#[derive(Clone, Debug)]
pub struct Config {
    pub settings: Settings,
    pub bindings: Vec<Binding>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            settings: Settings::default(),
            bindings: default_bindings(),
        }
    }
}

/// Путь `%APPDATA%\XboxDesk\settings.ini`.
pub fn default_path() -> Option<PathBuf> {
    std::env::var_os("APPDATA")
        .map(|appdata| PathBuf::from(appdata).join("XboxDesk").join("settings.ini"))
}

/// Загрузка конфига. Возвращает конфиг и предупреждения о битых строках.
/// Отсутствующий файл — не ошибка: возвращаются значения по умолчанию.
pub fn load(path: Option<&Path>) -> (Config, Vec<String>) {
    let mut warnings = Vec::new();
    let Some(path) = path else {
        warnings.push("Папка %APPDATA% недоступна — настройки не сохраняются.".into());
        return (Config::default(), warnings);
    };
    let Ok(text) = fs::read_to_string(path) else {
        return (Config::default(), warnings);
    };
    let mut config = Config {
        settings: Settings::default(),
        bindings: Vec::new(),
    };
    let mut section = String::new();
    for (number, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            section = name.trim().to_ascii_lowercase();
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            warnings.push(format!("строка {}: нет `=` — пропущено", number + 1));
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        match section.as_str() {
            "settings" => {
                if let Err(problem) = apply_setting(&mut config.settings, key, value) {
                    warnings.push(format!("строка {}: {problem}", number + 1));
                }
            }
            "bindings" => match Binding::parse_line(line) {
                Ok(binding) => config.bindings.push(binding),
                Err(problem) => warnings.push(format!("строка {}: {problem}", number + 1)),
            },
            other => warnings.push(format!(
                "строка {}: неизвестная секция [{other}]",
                number + 1
            )),
        }
    }
    config.settings = config.settings.clamped();
    if config.bindings.is_empty() && !text.trim().is_empty() {
        warnings.push("Ни одной корректной привязки в файле — взяты стандартные.".into());
        config.bindings = default_bindings();
    }
    (config, warnings)
}

fn apply_setting(settings: &mut Settings, key: &str, value: &str) -> Result<(), String> {
    let number = || -> Result<u32, String> {
        value
            .parse::<u32>()
            .map_err(|_| format!("`{key}` должно быть целым числом, а не `{value}`"))
    };
    let boolean = || -> Result<bool, String> {
        match value.to_ascii_lowercase().as_str() {
            "1" | "true" | "on" | "yes" => Ok(true),
            "0" | "false" | "off" | "no" => Ok(false),
            _ => Err(format!("`{key}` должно быть 0 или 1, а не `{value}`")),
        }
    };
    match key.to_ascii_lowercase().as_str() {
        "left_stick" => {
            settings.left_stick =
                StickMode::parse(value).ok_or_else(|| format!("не понял режим `{value}`"))?
        }
        "right_stick" => {
            settings.right_stick =
                StickMode::parse(value).ok_or_else(|| format!("не понял режим `{value}`"))?
        }
        "left_sensitivity" => settings.left_sensitivity = number()?,
        "right_sensitivity" => settings.right_sensitivity = number()?,
        "trigger_scroll" => settings.trigger_scroll = boolean()?,
        "scroll_speed" => settings.scroll_speed = number()?,
        "deadzone" | "deadzone_pct" => settings.deadzone_pct = number()?,
        "rumble" => settings.rumble = boolean()?,
        "auto_start" => settings.auto_start = boolean()?,
        other => return Err(format!("неизвестный параметр `{other}`")),
    }
    Ok(())
}

/// Сериализация в INI-текст.
pub fn to_ini(config: &Config) -> String {
    let s = &config.settings;
    let mut out = String::new();
    out.push_str(
        "# XboxDesk — настройки геймпада. Файл можно править вручную при закрытой программе.\n",
    );
    out.push_str("[settings]\n");
    out.push_str(&format!("left_stick={}\n", s.left_stick.token()));
    out.push_str(&format!("left_sensitivity={}\n", s.left_sensitivity));
    out.push_str(&format!("right_stick={}\n", s.right_stick.token()));
    out.push_str(&format!("right_sensitivity={}\n", s.right_sensitivity));
    out.push_str(&format!("trigger_scroll={}\n", s.trigger_scroll as u8));
    out.push_str(&format!("scroll_speed={}\n", s.scroll_speed));
    out.push_str(&format!("deadzone={}\n", s.deadzone_pct));
    out.push_str(&format!("rumble={}\n", s.rumble as u8));
    out.push_str(&format!("auto_start={}\n", s.auto_start as u8));
    out.push_str("\n[bindings]\n");
    out.push_str("# строка = КНОПКА=событие|действие[|rumble]; события: press, release, hold:мс, double:мс, repeat:мс\n");
    out.push_str("# действия: keys:CTRL+C, text:привет, mouse:left, wheel:3, wheelh:-2, macro:keys:A;delay:200;text:ок\n");
    for binding in &config.bindings {
        out.push_str(&binding.to_line());
        out.push('\n');
    }
    out
}

/// Сохранение (атомарно: сначала временный файл, затем rename).
pub fn save(path: Option<&Path>, config: &Config) -> Result<(), String> {
    let Some(path) = path else {
        return Err("Папка %APPDATA% недоступна — сохранить некуда.".into());
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("не удалось создать папку настроек: {e}"))?;
    }
    let tmp = path.with_extension("ini.tmp");
    fs::write(&tmp, to_ini(config)).map_err(|e| format!("не удалось записать настройки: {e}"))?;
    // На Windows rename поверх существующего файла может не сработать — сначала удаляем.
    let _ = fs::remove_file(path);
    fs::rename(&tmp, path).map_err(|e| format!("не удалось сохранить настройки: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ini_roundtrip() {
        let config = Config::default();
        let text = to_ini(&config);
        let dir = std::env::temp_dir().join(format!("xboxdesk-test-{}", std::process::id()));
        let path = dir.join("settings.ini");
        fs::create_dir_all(&dir).unwrap();
        save(Some(&path), &config).unwrap();
        let (loaded, warnings) = load(Some(&path));
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(loaded.settings, config.settings);
        assert_eq!(loaded.bindings, config.bindings);
        assert_eq!(to_ini(&loaded), text);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn broken_lines_warn_but_load() {
        let dir = std::env::temp_dir().join(format!("xboxdesk-test-broken-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.ini");
        fs::write(
            &path,
            "[settings]\nrumble=yes\nleft_sensitivity=abc\n[bindings]\nA=press|mouse:left\nZZZ=press|keys:A\n",
        )
        .unwrap();
        let (config, warnings) = load(Some(&path));
        assert_eq!(warnings.len(), 2, "{warnings:?}");
        assert!(config.settings.rumble);
        assert_eq!(config.bindings.len(), 1);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_file_gives_defaults() {
        let path = Path::new("S:\\точно-нет-такого-файла-xboxdesk\\settings.ini");
        let (config, warnings) = load(Some(path));
        assert!(warnings.is_empty());
        assert_eq!(config.settings, Settings::default());
        assert!(!config.bindings.is_empty());
    }

    #[test]
    fn settings_clamped() {
        let wild = Settings {
            left_sensitivity: 999,
            deadzone_pct: 200,
            scroll_speed: 0,
            ..Settings::default()
        }
        .clamped();
        assert_eq!(wild.left_sensitivity, 30);
        assert_eq!(wild.deadzone_pct, 40);
        assert_eq!(wild.scroll_speed, 1);
    }
}
