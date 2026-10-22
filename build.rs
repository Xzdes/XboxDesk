//! Сборка ресурсов Windows (иконка + манифест) штатным `rc.exe` из SDK.
//!
//! Никаких сторонних build-зависимостей: ищем rc.exe в Windows Kits,
//! компилируем assets\xboxdesk.rc в OUT_DIR и передаем .res линкеру.
//! Если rc.exe не найден — exe просто соберется без иконки (не фатально).

use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=assets/xboxdesk.rc");
    println!("cargo:rerun-if-changed=assets/xboxdesk.ico");
    println!("cargo:rerun-if-changed=assets/xboxdesk.manifest");
    println!("cargo:rerun-if-changed=build.rs");

    if env::var("CARGO_CFG_TARGET_OS").ok().as_deref() != Some("windows") {
        return;
    }
    let manifest_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let rc_file = manifest_dir.join("assets").join("xboxdesk.rc");
    if !rc_file.exists() {
        return;
    }
    let Some(rc_exe) = find_rc() else {
        println!("cargo:warning=rc.exe (Windows SDK) не найден — exe будет без иконки");
        return;
    };
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let res = out_dir.join("xboxdesk.res");
    let status = Command::new(&rc_exe)
        .args(["/nologo", "/fo"])
        .arg(&res)
        .arg(&rc_file)
        .current_dir(manifest_dir.join("assets"))
        .status();
    match status {
        Ok(s) if s.success() && res.exists() => {
            // rustc передаст .res линкеру как входной файл (так делает embed-resource).
            println!("cargo:rustc-link-arg-bins={}", res.display());
        }
        Ok(s) => println!("cargo:warning=rc.exe завершился с кодом {s} — exe без иконки"),
        Err(problem) => {
            println!("cargo:warning=не удалось запустить rc.exe: {problem} — exe без иконки")
        }
    }
}

/// Последний rc.exe из Windows Kits 10 (x64).
fn find_rc() -> Option<PathBuf> {
    let root = PathBuf::from(r"C:\Program Files (x86)\Windows Kits\10\bin");
    let mut candidates: Vec<PathBuf> = std::fs::read_dir(&root)
        .ok()?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| path.is_dir())
        .map(|path| path.join("x64").join("rc.exe"))
        .filter(|exe| exe.exists())
        .collect();
    if candidates.is_empty() {
        return None;
    }
    candidates.sort();
    candidates.pop()
}
