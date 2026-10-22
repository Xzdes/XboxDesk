# Contributing to XboxDesk

Thank you for your interest! Contributions are welcome.

## Getting Started

1. **Requirements**
   - Rust MSVC toolchain (`rustup default stable-msvc`)
   - Windows 10/11 SDK (for `rc.exe` to embed icon/manifest)
   - PowerShell or cmd

2. **Clone and Build**
   ```powershell
   git clone https://github.com/<your-org>/XboxDesk.git
   cd XboxDesk
   cargo build --release
   ```

3. **Run Tests**
   ```powershell
   cargo test
   ```

## Code Style

- Rust: `cargo fmt --check` + `cargo clippy -- -D warnings`
- Commit messages: Conventional Commits (e.g. `feat:`, `fix:`, `docs:`, `refactor:`)
- Keep PRs focused and small

## Architecture Notes

- **No external UI toolkits** — pure Win32/GDI via `windows-sys`
- **Zero runtime deps** — only Windows system DLLs
- **Double-buffered GDI** drawing in `src/gui.rs`
- **125 Hz XInput thread** in `src/engine.rs` via `Arc<Mutex<Shared>>`
- **Owner-draw controls** for custom buttons and list rows
- Per-monitor DPI V2, logical 96-DPI coordinate scaling

## Areas Open for Contribution

- Gamepad-driven UI navigation (focus graph, D-pad/stick nav)
- Hover states on child controls (subclassing or WM_MOUSEMOVE hit-test)
- Accessibility: high-contrast mode, screen-reader labels
- Localization (currently Russian-only UI)
- Theming / user colour preferences
- Portable settings backup/restore
- Automated Windows packaging (MSIX, winget, Scoop)

## Submitting

1. Fork → feature branch → PR against `main`
2. Ensure `cargo test`, `cargo fmt --check`, `cargo clippy` pass
3. Update README/CHANGELOG if user-facing
4. Squash-merge on approval

## Code of Conduct

Be respectful, inclusive, and constructive. This project follows the [Contributor Covenant](https://www.contributor-covenant.org/version/2/1/code_of_conduct/).