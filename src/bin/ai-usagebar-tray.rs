//! Windows system-tray popover. On other OSes this binary exists so
//! `cargo build --all-targets` stays uniform, and exits with a short message.

#![cfg_attr(windows, windows_subsystem = "windows")]

fn main() {
    // Support --log-requests for the tray binary
    for arg in std::env::args_os().skip(1) {
        if arg == "--log-requests" {
            ai_usagebar::request_log::enable_via_cli_flag();
            break;
        }
    }
    ai_usagebar::request_log::begin_run();
    std::process::exit(ai_usagebar::tray::run());
}
