use std::{
    ffi::c_void,
    process::Command,
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicI32, AtomicUsize, Ordering},
    },
    thread,
    time::Duration,
};

use gpui_kit::Window;
use objc2::{
    ClassType as _, msg_send,
    rc::{Retained, autoreleasepool},
};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationOptions, NSApplicationActivationPolicy, NSEvent,
    NSFloatingWindowLevel, NSPasteboard, NSRunningApplication, NSScreen, NSView, NSWindow,
    NSWindowCollectionBehavior, NSWorkspace,
};
use objc2_core_graphics::{
    CGEvent, CGEventFlags, CGEventSource, CGEventSourceStateID, CGEventTapLocation, CGKeyCode,
};
use objc2_foundation::{MainThreadMarker, NSPoint};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use super::ForegroundApp;

const KEY_V: CGKeyCode = 9;

static MAIN_WINDOW: AtomicUsize = AtomicUsize::new(0);
static LAST_TARGET_PID: AtomicI32 = AtomicI32::new(0);
static LAST_TARGET_APP: OnceLock<Mutex<Option<ForegroundApp>>> = OnceLock::new();

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    static kAXTrustedCheckOptionPrompt: *const c_void;
    fn AXIsProcessTrusted() -> u8;
    fn AXIsProcessTrustedWithOptions(options: *const c_void) -> u8;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    static kCFBooleanTrue: *const c_void;
    static kCFTypeDictionaryKeyCallBacks: c_void;
    static kCFTypeDictionaryValueCallBacks: c_void;
    fn CFDictionaryCreateMutable(
        allocator: *const c_void,
        capacity: isize,
        key_callbacks: *const c_void,
        value_callbacks: *const c_void,
    ) -> *mut c_void;
    fn CFDictionaryAddValue(dict: *mut c_void, key: *const c_void, value: *const c_void);
    fn CFRelease(cf: *const c_void);
}

pub fn start_foreground_tracker() {
    thread::Builder::new()
        .name("quick-paste-foreground".to_owned())
        .spawn(|| {
            loop {
                poll_frontmost_app();
                thread::sleep(Duration::from_millis(150));
            }
        })
        .ok();
}

fn poll_frontmost_app() -> Option<ForegroundApp> {
    autoreleasepool(|_| {
        let workspace = NSWorkspace::sharedWorkspace();
        let front = workspace.frontmostApplication()?;
        let pid = front.processIdentifier();
        let self_pid = std::process::id() as i32;
        if pid == 0 || pid == self_pid {
            return None;
        }

        let name = front
            .localizedName()
            .map(|value| value.to_string())
            .filter(|value| !value.is_empty());
        let id = front
            .bundleIdentifier()
            .map(|value| value.to_string())
            .filter(|value| !value.is_empty());
        let info = ForegroundApp { name, id };

        LAST_TARGET_PID.store(pid, Ordering::Relaxed);
        let lock = LAST_TARGET_APP.get_or_init(|| Mutex::new(None));
        if let Ok(mut guard) = lock.lock() {
            *guard = Some(info.clone());
        }
        Some(info)
    })
}

pub fn current_target_app() -> Option<ForegroundApp> {
    if let Some(info) = poll_frontmost_app() {
        return Some(info);
    }
    LAST_TARGET_APP
        .get()
        .and_then(|lock| lock.lock().ok())
        .and_then(|guard| guard.clone())
}

pub fn register_main_window(window: &Window) {
    let Some(_mtm) = MainThreadMarker::new() else {
        return;
    };
    let Some(ns_window) = extract_ns_window(window) else {
        return;
    };
    let ptr = (&*ns_window as *const NSWindow) as usize;
    let prev = MAIN_WINDOW.swap(ptr, Ordering::Relaxed);
    if prev != ptr {
        ns_window.setLevel(NSFloatingWindowLevel);
        ns_window.setCollectionBehavior(
            NSWindowCollectionBehavior::CanJoinAllSpaces
                | NSWindowCollectionBehavior::MoveToActiveSpace
                | NSWindowCollectionBehavior::FullScreenAuxiliary,
        );
    }
}

fn extract_ns_window(window: &Window) -> Option<objc2::rc::Retained<NSWindow>> {
    let handle = HasWindowHandle::window_handle(window).ok()?;
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        return None;
    };
    let view_ptr = handle.ns_view.as_ptr().cast::<NSView>();
    if view_ptr.is_null() {
        return None;
    }
    // SAFETY: 该回调仅在主线程调用，ns_view 来自当前有效 GPUI 窗口。
    let view = unsafe { &*view_ptr };
    view.window()
}

fn main_ns_window<'a>(_mtm: MainThreadMarker) -> Option<&'a NSWindow> {
    let ptr = MAIN_WINDOW.load(Ordering::Relaxed) as *const NSWindow;
    if ptr.is_null() {
        return None;
    }
    // SAFETY: 仅在主线程解引用由 register_main_window 缓存的主窗口指针，窗口关闭被拦截不会释放。
    Some(unsafe { &*ptr })
}

pub fn hide_window(window: &Window) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    if let Some(ns_window) = extract_ns_window(window) {
        ns_window.orderOut(None);
    }
    NSApplication::sharedApplication(mtm).hide(None);
}

pub fn hide_main_window() {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    if let Some(ns_window) = main_ns_window(mtm) {
        ns_window.orderOut(None);
    }
    NSApplication::sharedApplication(mtm).hide(None);
}

pub fn show_main_window(position_on_cursor_screen: bool) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let _ = poll_frontmost_app();

    let Some(ns_window) = main_ns_window(mtm) else {
        return;
    };

    if position_on_cursor_screen {
        center_window_on_cursor_screen(mtm, ns_window);
    }

    let app = NSApplication::sharedApplication(mtm);
    app.activate();
    #[allow(deprecated)]
    app.activateIgnoringOtherApps(true);
    ns_window.makeKeyAndOrderFront(None);
}

fn center_window_on_cursor_screen(mtm: MainThreadMarker, ns_window: &NSWindow) {
    let mouse = NSEvent::mouseLocation();
    let screens = NSScreen::screens(mtm);
    let mut target_screen = None;
    for screen in screens.iter() {
        let frame = screen.frame();
        let in_x = mouse.x >= frame.origin.x && mouse.x <= frame.origin.x + frame.size.width;
        let in_y = mouse.y >= frame.origin.y && mouse.y <= frame.origin.y + frame.size.height;
        if in_x && in_y {
            target_screen = Some(screen);
            break;
        }
    }
    let Some(screen) = target_screen.or_else(|| NSScreen::mainScreen(mtm)) else {
        return;
    };

    let visible = screen.visibleFrame();
    let win_frame = ns_window.frame();
    let x = visible.origin.x + (visible.size.width - win_frame.size.width) * 0.5;
    let y = visible.origin.y + (visible.size.height - win_frame.size.height) * 0.5;
    ns_window.setFrameOrigin(NSPoint::new(x, y));
}

pub fn is_main_window_visible() -> bool {
    let Some(mtm) = MainThreadMarker::new() else {
        return false;
    };
    main_ns_window(mtm).is_some_and(|window| window.isVisible())
}

pub fn is_main_window_foreground() -> bool {
    let Some(mtm) = MainThreadMarker::new() else {
        return false;
    };
    main_ns_window(mtm).is_some_and(|window| window.isVisible() && window.isKeyWindow())
}

pub fn hide_then_paste(window: &Window, on_after_paste: Option<Box<dyn FnOnce() + Send>>) -> bool {
    if !has_paste_permission() {
        return false;
    }

    let target_pid = LAST_TARGET_PID.load(Ordering::Relaxed);
    hide_window(window);

    if target_pid > 0
        && let Some(target_app) =
            NSRunningApplication::runningApplicationWithProcessIdentifier(target_pid)
    {
        #[allow(deprecated)]
        let _ = target_app.activateWithOptions(
            NSApplicationActivationOptions::ActivateAllWindows
                | NSApplicationActivationOptions::ActivateIgnoringOtherApps,
        );
    }

    let spawned = thread::Builder::new()
        .name("quick-paste-paste".to_owned())
        .spawn(move || {
            wait_for_modifiers_released();
            thread::sleep(Duration::from_millis(65));

            let _ = send_cmd_v();

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

fn wait_for_modifiers_released() {
    let watched = CGEventFlags::MaskShift
        | CGEventFlags::MaskControl
        | CGEventFlags::MaskAlternate
        | CGEventFlags::MaskCommand;
    for _ in 0..25 {
        let flags = CGEventSource::flags_state(CGEventSourceStateID::CombinedSessionState);
        if (flags & watched).is_empty() {
            return;
        }
        thread::sleep(Duration::from_millis(20));
    }
}

fn send_cmd_v() -> Result<(), ()> {
    let source = CGEventSource::new(CGEventSourceStateID::Private);
    let down = CGEvent::new_keyboard_event(source.as_deref(), KEY_V, true).ok_or(())?;
    let up = CGEvent::new_keyboard_event(source.as_deref(), KEY_V, false).ok_or(())?;

    CGEvent::set_flags(Some(&down), CGEventFlags::MaskCommand);
    CGEvent::set_flags(Some(&up), CGEventFlags::MaskCommand);

    CGEvent::post(CGEventTapLocation::HIDEventTap, Some(&down));
    thread::sleep(Duration::from_millis(12));
    CGEvent::post(CGEventTapLocation::HIDEventTap, Some(&up));
    Ok(())
}

fn try_general_pasteboard() -> Option<Retained<NSPasteboard>> {
    // SAFETY: +[NSPasteboard generalPasteboard] 可能在沙箱或 pboard 不可用时返回 nil，使用 Option 避免 panic。
    unsafe { msg_send![NSPasteboard::class(), generalPasteboard] }
}

pub fn clipboard_change_token() -> u64 {
    autoreleasepool(|_| {
        let Some(pasteboard) = try_general_pasteboard() else {
            return 0;
        };
        pasteboard.changeCount().max(0) as u64
    })
}

pub fn clipboard_has_concealed_marker() -> bool {
    autoreleasepool(|_| {
        let Some(pasteboard) = try_general_pasteboard() else {
            return false;
        };
        let Some(types) = pasteboard.types() else {
            return false;
        };
        for item in types.iter() {
            let name = item.to_string();
            if matches!(
                name.as_str(),
                "org.nspasteboard.ConcealedType"
                    | "org.nspasteboard.TransientType"
                    | "org.nspasteboard.AutoGeneratedType"
                    | "com.agilebits.onepassword"
            ) {
                return true;
            }
        }
        false
    })
}

pub fn has_paste_permission() -> bool {
    // SAFETY: AXIsProcessTrusted 无参数，仅查询当前进程的辅助功能信任状态。
    unsafe { AXIsProcessTrusted() != 0 }
}

pub fn request_paste_permission() -> bool {
    // SAFETY: 使用 CoreFoundation 标准字典回调构建单键值选项字典并及时 CFRelease。
    unsafe {
        let dict = CFDictionaryCreateMutable(
            std::ptr::null(),
            1,
            &kCFTypeDictionaryKeyCallBacks as *const _,
            &kCFTypeDictionaryValueCallBacks as *const _,
        );
        if dict.is_null() {
            return has_paste_permission();
        }
        CFDictionaryAddValue(dict, kAXTrustedCheckOptionPrompt, kCFBooleanTrue);
        let trusted = AXIsProcessTrustedWithOptions(dict) != 0;
        CFRelease(dict);
        trusted
    }
}

pub fn open_accessibility_settings() {
    let _ = Command::new("open")
        .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")
        .spawn();
}

pub fn set_accessory_activation_policy(accessory: bool) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let policy = if accessory {
        NSApplicationActivationPolicy::Accessory
    } else {
        NSApplicationActivationPolicy::Regular
    };
    NSApplication::sharedApplication(mtm).setActivationPolicy(policy);
}
