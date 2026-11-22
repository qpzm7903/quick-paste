//! 跨平台窗口控制、前台应用跟踪、粘贴模拟、隐私剪贴板检测与权限检查。

use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use gpui_kit::Window;

static LAST_SHOW_MS: AtomicU64 = AtomicU64::new(0);
static LAST_HIDE_MS: AtomicU64 = AtomicU64::new(0);

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

pub(crate) fn mark_window_shown() {
    LAST_SHOW_MS.store(now_millis(), Ordering::Relaxed);
}

pub(crate) fn mark_window_hidden() {
    LAST_HIDE_MS.store(now_millis(), Ordering::Relaxed);
}

pub fn recently_shown(within: Duration) -> bool {
    let last = LAST_SHOW_MS.load(Ordering::Relaxed);
    last > 0 && now_millis().saturating_sub(last) <= within.as_millis() as u64
}

pub fn recently_hidden(within: Duration) -> bool {
    let last = LAST_HIDE_MS.load(Ordering::Relaxed);
    last > 0 && now_millis().saturating_sub(last) <= within.as_millis() as u64
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForegroundApp {
    pub name: Option<String>,
    pub id: Option<String>,
}

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
use macos as imp;

#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
use windows as imp;

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod stub {
    use super::ForegroundApp;
    use gpui_kit::Window;

    pub fn start_foreground_tracker() {}
    pub fn current_target_app() -> Option<ForegroundApp> {
        None
    }
    pub fn register_main_window(_window: &Window) {}
    pub fn hide_window(_window: &Window) {}
    pub fn hide_main_window() {}
    pub fn show_main_window(_position_on_cursor_screen: bool) {}
    pub fn is_main_window_visible() -> bool {
        false
    }
    pub fn is_main_window_foreground() -> bool {
        false
    }
    pub fn hide_then_paste(
        _window: &Window,
        _on_after_paste: Option<Box<dyn FnOnce() + Send>>,
    ) -> bool {
        false
    }
    pub fn clipboard_change_token() -> u64 {
        0
    }
    pub fn clipboard_has_concealed_marker() -> bool {
        false
    }
    pub fn has_paste_permission() -> bool {
        true
    }
    pub fn request_paste_permission() -> bool {
        true
    }
    pub fn open_accessibility_settings() {}
    pub fn set_accessory_activation_policy(_accessory: bool) {}
}
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
use stub as imp;

pub fn start_foreground_tracker() {
    imp::start_foreground_tracker();
}

pub fn current_target_app() -> Option<ForegroundApp> {
    imp::current_target_app()
}

pub fn register_main_window(window: &Window) {
    imp::register_main_window(window);
}

pub fn hide_window(window: &Window) {
    imp::hide_window(window);
}

pub fn hide_main_window() {
    imp::hide_main_window();
}

pub fn show_main_window(position_on_cursor_screen: bool) {
    imp::show_main_window(position_on_cursor_screen);
}

#[allow(dead_code)]
pub fn is_main_window_visible() -> bool {
    imp::is_main_window_visible()
}

pub fn is_main_window_foreground() -> bool {
    imp::is_main_window_foreground()
}

pub fn hide_then_paste(window: &Window, on_after_paste: Option<Box<dyn FnOnce() + Send>>) -> bool {
    imp::hide_then_paste(window, on_after_paste)
}

pub fn clipboard_change_token() -> u64 {
    imp::clipboard_change_token()
}

pub fn clipboard_has_concealed_marker() -> bool {
    imp::clipboard_has_concealed_marker()
}

pub fn has_paste_permission() -> bool {
    imp::has_paste_permission()
}

pub fn request_paste_permission() -> bool {
    imp::request_paste_permission()
}

pub fn open_accessibility_settings() {
    imp::open_accessibility_settings();
}

pub fn set_accessory_activation_policy(accessory: bool) {
    imp::set_accessory_activation_policy(accessory);
}
