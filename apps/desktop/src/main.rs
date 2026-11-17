// No console window on Windows.
#![cfg_attr(windows, windows_subsystem = "windows")]
fn main() -> Result<(), slint::PlatformError> {
    atomic_ui::run()
}
