use std::{
    ffi::c_void,
    path::Path,
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicIsize, Ordering},
    },
    thread,
    time::Duration,
};

use gpui_kit::Window;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::{
    Win32::{
        Foundation::{CloseHandle, HWND, POINT, RECT},
        Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint},
        System::{
            DataExchange::{
                CloseClipboard, GetClipboardData, GetClipboardSequenceNumber,
                IsClipboardFormatAvailable, OpenClipboard, RegisterClipboardFormatW,
            },
            Memory::{GlobalLock, GlobalUnlock},
            Threading::{
                GetCurrentProcessId, OpenProcess, PROCESS_NAME_WIN32,
                PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
            },
        },
        UI::{
            Input::KeyboardAndMouse::{
                GetAsyncKeyState, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT,
                KEYEVENTF_KEYUP, SendInput, VIRTUAL_KEY, VK_CONTROL, VK_MENU, VK_SHIFT, VK_V,
            },
            WindowsAndMessaging::{
                GetCursorPos, GetForegroundWindow, GetWindowRect, GetWindowThreadProcessId,
                HWND_TOPMOST, IsIconic, IsWindow, IsWindowVisible, SW_HIDE, SW_RESTORE, SW_SHOW,
                SWP_NOACTIVATE, SWP_NOSIZE, SetForegroundWindow, SetWindowPos, ShowWindowAsync,
            },
        },
    },
    core::{PCWSTR, PWSTR},
};

use super::ForegroundApp;

static MAIN_WINDOW: AtomicIsize = AtomicIsize::new(0);
static LAST_TARGET: AtomicIsize = AtomicIsize::new(0);
static LAST_TARGET_APP: OnceLock<Mutex<Option<ForegroundApp>>> = OnceLock::new();

pub fn start_foreground_tracker() {
    thread::Builder::new()
        .name("quick-paste-foreground".to_owned())
        .spawn(|| {
            loop {
                // SAFETY: GetForegroundWindow 只读取当前前台窗口句柄。
                let foreground = unsafe { GetForegroundWindow() };
                if !foreground.0.is_null() && !is_current_process(foreground) {
                    LAST_TARGET.store(foreground.0 as isize, Ordering::Relaxed);
                    let app = process_info_for_window(foreground);
                    let lock = LAST_TARGET_APP.get_or_init(|| Mutex::new(None));
                    if let Ok(mut guard) = lock.lock() {
                        *guard = app;
                    }
                }
                thread::sleep(Duration::from_millis(120));
            }
        })
        .ok();
}

pub fn current_target_app() -> Option<ForegroundApp> {
    // SAFETY: GetForegroundWindow 只读取当前前台窗口句柄。
    let foreground = unsafe { GetForegroundWindow() };
    if !foreground.0.is_null()
        && !is_current_process(foreground)
        && let Some(info) = process_info_for_window(foreground)
    {
        return Some(info);
    }
    LAST_TARGET_APP
        .get()
        .and_then(|lock| lock.lock().ok())
        .and_then(|guard| guard.clone())
}

pub fn register_main_window(window: &Window) {
    if let Some(hwnd) = window_hwnd(window) {
        let prev = MAIN_WINDOW.swap(hwnd, Ordering::Relaxed);
        if prev != hwnd {
            super::mark_window_shown();
        }
    }
}

fn window_hwnd(window: &Window) -> Option<isize> {
    let handle = HasWindowHandle::window_handle(window).ok()?;
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        return None;
    };
    Some(handle.hwnd.get())
}

pub fn hide_window(window: &Window) {
    let Some(hwnd) = window_hwnd(window) else {
        return;
    };
    super::mark_window_hidden();
    // SAFETY: hwnd 来自当前 GPUI 窗口，ShowWindowAsync 只修改该窗口的显示状态。
    unsafe {
        let _ = ShowWindowAsync(HWND(hwnd as *mut c_void), SW_HIDE);
    }
    crate::memory_trim::schedule_hidden_trim();
}

pub fn hide_main_window() {
    let hwnd = MAIN_WINDOW.load(Ordering::Relaxed);
    if hwnd == 0 {
        return;
    }
    super::mark_window_hidden();
    // SAFETY: hwnd 是本进程记录的窗口句柄。
    unsafe {
        let _ = ShowWindowAsync(HWND(hwnd as *mut c_void), SW_HIDE);
    }
    crate::memory_trim::schedule_hidden_trim();
}

pub fn show_main_window(position_on_cursor_screen: bool) {
    let hwnd = MAIN_WINDOW.load(Ordering::Relaxed);
    if hwnd == 0 {
        return;
    }
    super::mark_window_shown();
    let handle = HWND(hwnd as *mut c_void);
    if position_on_cursor_screen {
        center_window_on_cursor_monitor(handle);
    }
    // SAFETY: hwnd 是本进程记录的窗口句柄。
    unsafe {
        let command = if IsIconic(handle).as_bool() {
            SW_RESTORE
        } else {
            SW_SHOW
        };
        let _ = ShowWindowAsync(handle, command);
        let _ = SetForegroundWindow(handle);
    }
    crate::memory_trim::cancel_hidden_trim();
}

fn center_window_on_cursor_monitor(handle: HWND) {
    let mut cursor = POINT::default();
    // SAFETY: GetCursorPos 读取当前鼠标屏幕坐标。
    if unsafe { GetCursorPos(&mut cursor) }.is_err() {
        return;
    }
    // SAFETY: MonitorFromPoint 与 GetMonitorInfoW 读取鼠标所在显示器工作区矩形。
    let monitor = unsafe { MonitorFromPoint(cursor, MONITOR_DEFAULTTONEAREST) };
    if monitor.0.is_null() {
        return;
    }
    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    if !unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
        return;
    }

    let mut rect = RECT::default();
    if unsafe { GetWindowRect(handle, &mut rect) }.is_err() {
        return;
    }
    let win_w = (rect.right - rect.left).max(400);
    let win_h = (rect.bottom - rect.top).max(400);
    let work = info.rcWork;
    let x = work.left + ((work.right - work.left - win_w) / 2);
    let y = work.top + ((work.bottom - work.top - win_h) / 2);

    // SAFETY: SetWindowPos 仅调整窗口位置与置顶层级，保持宽高不变。
    unsafe {
        let _ = SetWindowPos(
            handle,
            Some(HWND_TOPMOST),
            x,
            y,
            0,
            0,
            SWP_NOSIZE | SWP_NOACTIVATE,
        );
    }
}

pub fn is_main_window_foreground() -> bool {
    let main = MAIN_WINDOW.load(Ordering::Relaxed);
    if main == 0 {
        return false;
    }
    // SAFETY: GetForegroundWindow 只读取当前前台窗口句柄。
    let foreground = unsafe { GetForegroundWindow() };
    foreground.0 as isize == main
}

pub fn is_main_window_visible() -> bool {
    let main = MAIN_WINDOW.load(Ordering::Relaxed);
    if main == 0 {
        return false;
    }
    // SAFETY: hwnd 是本进程记录的窗口句柄，IsWindowVisible 只查询显示状态。
    unsafe { IsWindowVisible(HWND(main as *mut c_void)).as_bool() }
}

/// 隐藏本窗口并把 Ctrl+V 发到刚才的前台窗口。没有可用目标时返回 `false`。
pub fn hide_then_paste(window: &Window, on_after_paste: Option<Box<dyn FnOnce() + Send>>) -> bool {
    let Some(target) = valid_target() else {
        return false;
    };

    // 先把前台交还给目标窗口，再隐藏自己：本进程此刻仍持有前台权限，
    // 比隐藏之后再调用 SetForegroundWindow 更可靠。
    let handle = HWND(target as *mut c_void);
    // SAFETY: target 已通过 IsWindow 校验，SetForegroundWindow 只切换前台窗口。
    unsafe {
        let _ = SetForegroundWindow(handle);
    }
    hide_window(window);

    let spawned = thread::Builder::new()
        .name("quick-paste-paste".to_owned())
        .spawn(move || {
            // 热键可能还按着 Ctrl/Shift，等物理按键松开再模拟，避免再次触发本程序热键。
            if !wait_for_modifiers_released() {
                return;
            }
            thread::sleep(Duration::from_millis(60));

            if !wait_until_foreground(target) {
                return;
            }

            // SAFETY: 模拟的组合键只发给当前前台窗口。
            unsafe {
                let _ = send_ctrl_v();
            }

            if let Some(after_paste) = on_after_paste {
                thread::sleep(Duration::from_millis(350));
                after_paste();
            }
        })
        .is_ok();

    if !spawned {
        show_main_window(false);
        return false;
    }
    true
}

pub fn clipboard_change_token() -> u64 {
    // SAFETY: GetClipboardSequenceNumber 无副作用，仅返回系统全局序列号。
    u64::from(unsafe { GetClipboardSequenceNumber() })
}

pub fn clipboard_has_concealed_marker() -> bool {
    let exclude_fmt = register_format("ExcludeClipboardContentFromMonitorProcessing");
    let ignore_fmt = register_format("Clipboard Viewer Ignore");
    let history_fmt = register_format("CanIncludeInClipboardHistory");

    // SAFETY: IsClipboardFormatAvailable 只查询剪贴板是否包含敏感排除标记。
    unsafe {
        if exclude_fmt != 0 && IsClipboardFormatAvailable(exclude_fmt).is_ok() {
            return true;
        }
        if ignore_fmt != 0 && IsClipboardFormatAvailable(ignore_fmt).is_ok() {
            return true;
        }
        if history_fmt != 0
            && IsClipboardFormatAvailable(history_fmt).is_ok()
            && OpenClipboard(None).is_ok()
        {
            let mut disallowed = false;
            if let Ok(handle) = GetClipboardData(history_fmt) {
                let hglobal = windows::Win32::Foundation::HGLOBAL(handle.0);
                let ptr = GlobalLock(hglobal);
                if !ptr.is_null() {
                    disallowed = *(ptr as *const u32) == 0;
                    let _ = GlobalUnlock(hglobal);
                }
            }
            let _ = CloseClipboard();
            if disallowed {
                return true;
            }
        }
    }
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

fn register_format(name: &str) -> u32 {
    let mut wide: Vec<u16> = name.encode_utf16().collect();
    wide.push(0);
    // SAFETY: wide 为以 0 结尾的有效 UTF-16 字符串。
    unsafe { RegisterClipboardFormatW(PCWSTR(wide.as_ptr())) }
}

fn process_info_for_window(hwnd: HWND) -> Option<ForegroundApp> {
    let mut process_id = 0u32;
    // SAFETY: hwnd 已非空，process_id 写入栈变量。
    unsafe {
        GetWindowThreadProcessId(hwnd, Some(&mut process_id));
    }
    if process_id == 0 {
        return None;
    }

    // SAFETY: OpenProcess 仅申请受限查询权限以读取可执行路径。
    let Ok(process) =
        (unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process_id) })
    else {
        return None;
    };

    let mut buf = [0u16; 512];
    let mut len = buf.len() as u32;
    let ok = unsafe {
        QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            PWSTR(buf.as_mut_ptr()),
            &mut len,
        )
    }
    .is_ok();
    // SAFETY: process 为有效进程句柄。
    unsafe {
        let _ = CloseHandle(process);
    }

    if !ok || len == 0 {
        return None;
    }

    let full_path = String::from_utf16_lossy(&buf[..len as usize]);
    let file_name = Path::new(&full_path)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())?;
    let display_name = Path::new(&full_path)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| file_name.clone());

    Some(ForegroundApp {
        name: Some(display_name),
        id: Some(file_name),
    })
}

fn valid_target() -> Option<isize> {
    let target = LAST_TARGET.load(Ordering::Relaxed);
    if target == 0 {
        return None;
    }
    // SAFETY: target 来自 GetForegroundWindow，IsWindow 只查询它是否仍然有效。
    let valid = unsafe { IsWindow(Some(HWND(target as *mut c_void))).as_bool() };
    valid.then_some(target)
}

fn is_current_process(hwnd: HWND) -> bool {
    let mut process_id = 0u32;
    // SAFETY: hwnd 来自 GetForegroundWindow，进程 ID 写入栈上变量。
    unsafe {
        GetWindowThreadProcessId(hwnd, Some(&mut process_id));
    }
    // SAFETY: GetCurrentProcessId 无参数，仅返回本进程 ID。
    process_id == unsafe { GetCurrentProcessId() }
}

fn wait_for_modifiers_released() -> bool {
    for _ in 0..25 {
        // SAFETY: GetAsyncKeyState 只读取按键状态。
        let control = unsafe { GetAsyncKeyState(VK_CONTROL.0 as i32) } < 0;
        let shift = unsafe { GetAsyncKeyState(VK_SHIFT.0 as i32) } < 0;
        let alt = unsafe { GetAsyncKeyState(VK_MENU.0 as i32) } < 0;
        if !control && !shift && !alt {
            return true;
        }
        thread::sleep(Duration::from_millis(20));
    }
    false
}

fn wait_until_foreground(target: isize) -> bool {
    for _ in 0..10 {
        // SAFETY: GetForegroundWindow 只读取当前前台窗口句柄。
        let foreground = unsafe { GetForegroundWindow() };
        if foreground.0 as isize == target {
            return true;
        }
        thread::sleep(Duration::from_millis(30));
    }
    false
}

unsafe fn send_ctrl_v() -> Result<(), ()> {
    let down = [
        keyboard_input(VK_CONTROL, false),
        keyboard_input(VK_V, false),
    ];
    let up = [keyboard_input(VK_V, true), keyboard_input(VK_CONTROL, true)];
    let size = std::mem::size_of::<INPUT>() as i32;

    // SAFETY: down 与 up 都是完整的 INPUT 数组，size 与结构体大小一致。
    let sent_down = unsafe { SendInput(&down, size) };
    thread::sleep(Duration::from_millis(8));
    let sent_up = unsafe { SendInput(&up, size) };

    if sent_down == down.len() as u32 && sent_up == up.len() as u32 {
        Ok(())
    } else {
        Err(())
    }
}

fn keyboard_input(key: VIRTUAL_KEY, key_up: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: key,
                wScan: 0,
                dwFlags: if key_up {
                    KEYEVENTF_KEYUP
                } else {
                    KEYBD_EVENT_FLAGS::default()
                },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}
