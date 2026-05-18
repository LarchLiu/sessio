// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if std::env::args_os().len() > 1 {
        app_lib::cli::run_from_env();
    } else {
        app_lib::run();
    }
}
