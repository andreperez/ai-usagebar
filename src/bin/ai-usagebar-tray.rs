//! Windows system-tray popover. On other OSes this binary exists so
//! `cargo build --all-targets` stays uniform, and exits with a short message.

#![cfg_attr(windows, windows_subsystem = "windows")]

fn main() {
    ai_usagebar::request_log::begin_run();
    std::process::exit(ai_usagebar::tray::run());
}
