#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod autolaunch;
mod clipboard;
mod config;
mod content;
mod history;
mod memo;
mod memory_trim;
mod platform;
mod search;
mod single_instance;
mod tray;
mod ui;
mod watcher;

fn main() {
    app::run();
}
