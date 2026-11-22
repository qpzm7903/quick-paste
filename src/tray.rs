//! 跨平台菜单栏 / 系统托盘集成（macOS 状态栏图标 / Windows 托盘图标）。

use async_channel::Receiver;
use tray_icon::{
    Icon, TrayIcon, TrayIconBuilder,
    menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem},
};
#[cfg(not(target_os = "macos"))]
use tray_icon::{MouseButton, MouseButtonState, TrayIconEvent};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayAction {
    ToggleWindow,
    TogglePause,
    NewSnippet,
    OpenSettings,
    OpenDataDir,
    Quit,
}

pub struct TrayController {
    _tray: TrayIcon,
    toggle_item: MenuItem,
    pause_item: MenuItem,
}

impl TrayController {
    pub fn create() -> Result<(Self, Receiver<TrayAction>), String> {
        let (sender, receiver) = async_channel::unbounded();

        let toggle_item = MenuItem::with_id("toggle", "显示 / 隐藏窗口", true, None);
        let pause_item = MenuItem::with_id("pause", "暂停剪贴板监听", true, None);
        let new_snippet_item = MenuItem::with_id("new_snippet", "新建常用片段…", true, None);
        let settings_item = MenuItem::with_id("settings", "偏好设置…", true, None);
        let open_data_item = MenuItem::with_id("open_data", "打开数据目录", true, None);
        let quit_item = MenuItem::with_id("quit", "退出 QuickPaste", true, None);

        let menu = Menu::new();
        let _ = menu.append(&toggle_item);
        let _ = menu.append(&pause_item);
        let _ = menu.append(&PredefinedMenuItem::separator());
        let _ = menu.append(&new_snippet_item);
        let _ = menu.append(&settings_item);
        let _ = menu.append(&open_data_item);
        let _ = menu.append(&PredefinedMenuItem::separator());
        let _ = menu.append(&quit_item);

        let icon = build_clipboard_icon()?;
        let mut builder = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip("QuickPaste · 快速粘贴");

        #[cfg(target_os = "macos")]
        {
            builder = builder.with_icon_templated(icon);
        }
        #[cfg(not(target_os = "macos"))]
        {
            builder = builder.with_icon(icon);
        }

        let tray = builder
            .build()
            .map_err(|e| format!("创建系统托盘失败：{e}"))?;

        let menu_sender = sender.clone();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            let action = match event.id.as_ref() {
                "toggle" => Some(TrayAction::ToggleWindow),
                "pause" => Some(TrayAction::TogglePause),
                "new_snippet" => Some(TrayAction::NewSnippet),
                "settings" => Some(TrayAction::OpenSettings),
                "open_data" => Some(TrayAction::OpenDataDir),
                "quit" => Some(TrayAction::Quit),
                _ => None,
            };
            if let Some(action) = action {
                let _ = menu_sender.try_send(action);
            }
        }));

        #[cfg(not(target_os = "macos"))]
        {
            let click_sender = sender;
            TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
                if let TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } = event
                {
                    let _ = click_sender.try_send(TrayAction::ToggleWindow);
                }
            }));
        }
        #[cfg(target_os = "macos")]
        {
            let _ = tray_icon::TrayIconEvent::receiver();
        }

        Ok((
            Self {
                _tray: tray,
                toggle_item,
                pause_item,
            },
            receiver,
        ))
    }

    pub fn set_paused(&self, paused: bool) {
        let label = if paused {
            "恢复剪贴板监听"
        } else {
            "暂停剪贴板监听"
        };
        self.pause_item.set_text(label);
    }

    #[allow(dead_code)]
    pub fn set_visible(&self, visible: bool) {
        let label = if visible {
            "隐藏主窗口"
        } else {
            "显示主窗口"
        };
        self.toggle_item.set_text(label);
    }
}

/// 纯 Rust 生成高清 32×32 RGBA 剪贴板图标。
/// macOS 使用纯黑模板像素以自适应深浅菜单栏，其他平台使用蓝色渐变主色。
fn build_clipboard_icon() -> Result<Icon, String> {
    const SIZE: u32 = 32;
    let mut rgba = vec![0u8; (SIZE * SIZE * 4) as usize];
    let monochrome = cfg!(target_os = "macos");

    for y in 0..SIZE {
        for x in 0..SIZE {
            let fx = x as f32 + 0.5;
            let fy = y as f32 + 0.5;

            // 外框：剪贴板主体 (x: 6..26, y: 6..29, radius 3.2)
            let body_outer = rounded_rect_sdf(fx, fy, 6.0, 6.5, 26.0, 29.0, 3.2);
            let body_inner = rounded_rect_sdf(fx, fy, 8.6, 9.1, 23.4, 26.4, 1.5);
            // 顶部夹子 (x: 11.5..20.5, y: 3.5..8.2, radius 1.8)
            let clip = rounded_rect_sdf(fx, fy, 11.2, 3.5, 20.8, 8.2, 1.8);
            // 两条内容横线
            let line1 = rounded_rect_sdf(fx, fy, 11.2, 13.8, 20.8, 16.0, 1.0);
            let line2 = rounded_rect_sdf(fx, fy, 11.2, 18.8, 18.4, 21.0, 1.0);

            let stroke_alpha =
                (sdf_alpha(body_outer) * (1.0 - sdf_alpha(body_inner))).max(sdf_alpha(clip));
            let content_alpha = sdf_alpha(line1).max(sdf_alpha(line2));
            let alpha = stroke_alpha.max(content_alpha).clamp(0.0, 1.0);

            if alpha > 0.01 {
                let idx = ((y * SIZE + x) * 4) as usize;
                if monochrome {
                    rgba[idx] = 0;
                    rgba[idx + 1] = 0;
                    rgba[idx + 2] = 0;
                    rgba[idx + 3] = (alpha * 255.0).round() as u8;
                } else {
                    // Windows 托盘：主体与夹子用明亮蓝色，内容线用白色/浅蓝。
                    let is_line = content_alpha > stroke_alpha;
                    if is_line {
                        rgba[idx] = 248;
                        rgba[idx + 1] = 250;
                        rgba[idx + 2] = 252;
                    } else {
                        rgba[idx] = 59;
                        rgba[idx + 1] = 130;
                        rgba[idx + 2] = 246;
                    }
                    rgba[idx + 3] = (alpha * 255.0).round() as u8;
                }
            }
        }
    }

    Icon::from_rgba(rgba, SIZE, SIZE).map_err(|e| format!("构造托盘图标失败：{e}"))
}

fn rounded_rect_sdf(px: f32, py: f32, x0: f32, y0: f32, x1: f32, y1: f32, r: f32) -> f32 {
    let cx = (x0 + x1) * 0.5;
    let cy = (y0 + y1) * 0.5;
    let hx = (x1 - x0) * 0.5 - r;
    let hy = (y1 - y0) * 0.5 - r;
    let dx = (px - cx).abs() - hx;
    let dy = (py - cy).abs() - hy;
    let outside = dx.max(0.0).hypot(dy.max(0.0));
    let inside = dx.max(dy).min(0.0);
    outside + inside - r
}

fn sdf_alpha(dist: f32) -> f32 {
    (0.5 - dist).clamp(0.0, 1.0)
}
