//! XboxDesk — геймпад Xbox как клавиатура и мышь.
//!
//! Графическое приложение без консоли (терминал не показывается): запуск
//! двойным кликом по exe, закрытие главного окна полностью останавливает
//! программу — движок предварительно отпускает все зажатые клавиши и мышь.
//! Рукописный Win32-интерфейс в стиле alibi-quic, без зависимостей.

#![windows_subsystem = "windows"]

mod binding;
mod config;
mod dialog;
mod engine;
mod gui;
mod send;
mod ui;
mod vk;
mod xinput;

fn main() {
    let config_path = config::default_path();
    let (cfg, warnings) = config::load(config_path.as_deref());
    if let Some(path) = &config_path {
        if !path.exists() {
            // Первый запуск: создаем INI со стандартной игровой раскладкой,
            // чтобы файл можно было найти и править руками.
            let _ = config::save(Some(path), &cfg);
        }
    }

    let shared = engine::new_shared(cfg);
    for warning in &warnings {
        engine::log(&shared, &format!("Предупреждение настроек: {warning}"));
    }

    let worker = engine::spawn(shared.clone());
    let code = gui::run(shared.clone());

    // Окно закрыли — останавливаем движок и ждем, пока он отпустит клавиши.
    if let Ok(mut guard) = shared.lock() {
        guard.stop = true;
        guard.running = false;
        guard.capture = None;
    }
    let _ = worker.join();
    std::process::exit(code);
}
