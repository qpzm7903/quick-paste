use gpui_kit::{
    AnyElement, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, SharedString, Styled as _, Window,
    assets::IconName,
    component::{
        ActiveTheme as _, Disableable as _, Icon, Sizable as _, StyledExt as _,
        button::{Button, ButtonVariants as _},
        h_flex,
        input::{Input, InputState},
        scroll::ScrollableElement as _,
        switch::Switch,
        v_flex,
    },
    div,
    prelude::FluentBuilder as _,
};

use super::QuickPasteApp;
use crate::{
    app::{self, HotkeyManagerHolder},
    autolaunch,
    config::{self, PasteTrigger, ThemePreference},
    platform,
};

pub struct SettingsState {
    pub hotkey_input: Entity<InputState>,
    pub exclude_app_input: Entity<InputState>,
    pub feedback: Option<(bool, SharedString)>,
}

impl SettingsState {
    pub fn new(current_hotkey: &str, window: &mut Window, cx: &mut Context<QuickPasteApp>) -> Self {
        let hotkey_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("例如 Cmd+Shift+V / Ctrl+Alt+V")
                .default_value(current_hotkey.to_owned())
        });
        let exclude_app_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("输入应用名称或 Bundle ID / .exe"));
        Self {
            hotkey_input,
            exclude_app_input,
            feedback: None,
        }
    }
}

pub fn render_settings(
    app_view: &QuickPasteApp,
    settings: &SettingsState,
    cx: &mut Context<QuickPasteApp>,
) -> AnyElement {
    let theme = cx.theme();
    let border = theme.border;
    let background = theme.background;
    let muted_fg = theme.muted_foreground;
    let secondary_bg = theme.secondary;
    let success = theme.success;
    let warning = theme.warning;
    let danger = theme.danger;
    let primary = theme.primary;

    let mut content = v_flex().w_full().gap_3();

    // macOS 辅助功能权限区块
    #[cfg(target_os = "macos")]
    {
        let trusted = platform::has_paste_permission();
        let status_color = if trusted { success } else { warning };
        let status_icon = if trusted {
            IconName::ShieldCheck
        } else {
            IconName::ShieldAlert
        };
        let status_title = if trusted {
            "辅助功能权限已授予"
        } else {
            "需要「辅助功能」权限以自动模拟 ⌘V 粘贴"
        };
        let status_desc = if trusted {
            "QuickPaste 可直接将选中内容自动粘贴至目标输入框。"
        } else {
            "未授权时仍可复制到剪贴板，授权后无需手动按 ⌘V。"
        };

        content = content.child(
            section_box("macOS 辅助功能权限", border, background).child(
                v_flex()
                    .gap_2()
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(Icon::new(status_icon).small().text_color(status_color))
                            .child(div().text_sm().font_medium().child(status_title)),
                    )
                    .child(div().text_xs().text_color(muted_fg).child(status_desc))
                    .when(!trusted, |this| {
                        this.child(
                            h_flex()
                                .gap_2()
                                .child(
                                    Button::new("settings-req-ax")
                                        .primary()
                                        .xsmall()
                                        .label("请求系统授权")
                                        .on_click(cx.listener(|_, _, _, cx| {
                                            platform::request_paste_permission();
                                            cx.notify();
                                        })),
                                )
                                .child(
                                    Button::new("settings-open-ax")
                                        .outline()
                                        .xsmall()
                                        .label("打开系统设置")
                                        .on_click(cx.listener(|_, _, _, _| {
                                            platform::open_accessibility_settings();
                                        })),
                                )
                                .child(
                                    Button::new("settings-refresh-ax")
                                        .ghost()
                                        .xsmall()
                                        .icon(IconName::RefreshCw)
                                        .tooltip("刷新权限状态")
                                        .on_click(cx.listener(|_, _, _, cx| {
                                            cx.notify();
                                        })),
                                ),
                        )
                    }),
            ),
        );
    }

    // 全局快捷键区块
    let presets = config::preset_hotkeys();
    let mut preset_row = h_flex().gap_1p5().flex_wrap();
    for (ix, &preset) in presets.iter().enumerate() {
        let active = app_view.config.hotkey.eq_ignore_ascii_case(preset);
        let label = config::format_hotkey_display(preset);
        let preset_owned = preset.to_owned();
        let mut btn = Button::new(("hotkey-preset", ix))
            .xsmall()
            .label(label)
            .on_click(cx.listener(move |this, _, window, cx| {
                this.apply_hotkey_setting(&preset_owned, window, cx);
            }));
        btn = if active { btn.primary() } else { btn.outline() };
        preset_row = preset_row.child(btn);
    }

    content = content.child(
        section_box("全局唤起热键", border, background).child(
            v_flex()
                .gap_2()
                .child(preset_row)
                .child(
                    h_flex()
                        .gap_2()
                        .child(div().flex_1().child(Input::new(&settings.hotkey_input)))
                        .child(
                            Button::new("apply-custom-hotkey")
                                .outline()
                                .small()
                                .label("应用")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    let raw = match &this.overlay {
                                        Some(super::OverlayView::Settings(s)) => {
                                            s.hotkey_input.read(cx).value().to_string()
                                        }
                                        _ => return,
                                    };
                                    this.apply_hotkey_setting(&raw, window, cx);
                                })),
                        ),
                )
                .when_some(settings.feedback.clone(), |this, (ok, msg)| {
                    this.child(
                        div()
                            .text_xs()
                            .text_color(if ok { success } else { danger })
                            .child(msg),
                    )
                }),
        ),
    );

    // 常规与粘贴行为
    let mut trigger_row = h_flex().gap_1p5();
    for (ix, trigger) in PasteTrigger::ALL.into_iter().enumerate() {
        let active = app_view.config.paste_trigger == trigger;
        let mut btn = Button::new(("paste-trigger", ix))
            .xsmall()
            .label(trigger.label())
            .on_click(cx.listener(move |this, _, _, cx| {
                this.config.paste_trigger = trigger;
                this.save_config(cx);
                cx.notify();
            }));
        btn = if active { btn.primary() } else { btn.outline() };
        trigger_row = trigger_row.child(btn);
    }

    content = content.child(
        section_box("粘贴与窗口行为", border, background).child(
            v_flex()
                .gap_2p5()
                .child(
                    h_flex()
                        .justify_between()
                        .items_center()
                        .child(div().text_sm().child("触发粘贴方式"))
                        .child(trigger_row),
                )
                .child(setting_switch_row(
                    "switch-restore-cb",
                    "粘贴后自动恢复原剪贴板",
                    Some("粘贴完成约 0.35 秒后还原粘贴前剪贴板内容"),
                    app_view.config.restore_clipboard,
                    muted_fg,
                    cx.listener(|this, checked: &bool, _, cx| {
                        this.config.restore_clipboard = *checked;
                        this.save_config(cx);
                        cx.notify();
                    }),
                ))
                .child(setting_switch_row(
                    "switch-cursor-screen",
                    "在鼠标所在屏幕居中弹出",
                    Some("多显示器环境下跟随光标屏幕"),
                    app_view.config.position_on_cursor_screen,
                    muted_fg,
                    cx.listener(|this, checked: &bool, _, cx| {
                        this.config.position_on_cursor_screen = *checked;
                        this.save_config(cx);
                        cx.notify();
                    }),
                ))
                .child(setting_switch_row(
                    "switch-hide-blur",
                    "窗口失去焦点时自动隐藏",
                    None,
                    app_view.config.hide_on_blur,
                    muted_fg,
                    cx.listener(|this, checked: &bool, _, cx| {
                        this.config.hide_on_blur = *checked;
                        this.save_config(cx);
                        cx.notify();
                    }),
                ))
                .child(setting_switch_row(
                    "switch-autolaunch",
                    "登录系统时自动启动",
                    None,
                    app_view.config.launch_at_login,
                    muted_fg,
                    cx.listener(|this, checked: &bool, _, cx| {
                        let target = *checked;
                        match autolaunch::sync_launch_at_login(target) {
                            Ok(()) => {
                                this.config.launch_at_login = target;
                                this.save_config(cx);
                                if let Some(super::OverlayView::Settings(ref mut s)) = this.overlay
                                {
                                    s.feedback = None;
                                }
                            }
                            Err(error) => {
                                if let Some(super::OverlayView::Settings(ref mut s)) = this.overlay
                                {
                                    s.feedback = Some((false, error.into()));
                                }
                            }
                        }
                        cx.notify();
                    }),
                )),
        ),
    );

    // 外观主题
    let mut theme_row = h_flex().gap_1p5();
    for (ix, pref) in ThemePreference::ALL.into_iter().enumerate() {
        let active = app_view.config.theme == pref;
        let mut btn = Button::new(("theme-pref", ix))
            .xsmall()
            .label(pref.label())
            .on_click(cx.listener(move |this, _, window, cx| {
                this.config.theme = pref;
                app::apply_theme(pref, window.appearance(), cx);
                this.save_config(cx);
                cx.notify();
            }));
        btn = if active { btn.primary() } else { btn.outline() };
        theme_row = theme_row.child(btn);
    }

    content = content.child(
        section_box("外观主题", border, background).child(
            h_flex()
                .justify_between()
                .items_center()
                .child(div().text_sm().child("界面配色"))
                .child(theme_row),
        ),
    );

    // 剪贴板历史
    let limits = [100usize, 300, 500, 1000, 2000];
    let mut limit_row = h_flex().gap_1().flex_wrap();
    for (ix, limit) in limits.into_iter().enumerate() {
        let active = app_view.config.max_history_items == limit;
        let mut btn = Button::new(("history-limit", ix))
            .xsmall()
            .label(limit.to_string())
            .on_click(cx.listener(move |this, _, _, cx| {
                this.config.max_history_items = limit;
                this.save_config(cx);
                cx.notify();
            }));
        btn = if active { btn.primary() } else { btn.outline() };
        limit_row = limit_row.child(btn);
    }

    let unpinned_count = app_view
        .history
        .items()
        .iter()
        .filter(|i| !i.is_pinned)
        .count();

    content = content.child(
        section_box("剪贴板历史", border, background).child(
            v_flex()
                .gap_2p5()
                .child(setting_switch_row(
                    "switch-record-cb",
                    "自动记录系统剪贴板",
                    None,
                    app_view.config.record_clipboard,
                    muted_fg,
                    cx.listener(|this, checked: &bool, _, cx| {
                        this.config.record_clipboard = *checked;
                        this.save_config(cx);
                        cx.notify();
                    }),
                ))
                .child(setting_switch_row(
                    "switch-record-img",
                    "记录剪贴板图片（PNG 缩略图与原图）",
                    None,
                    app_view.config.record_images,
                    muted_fg,
                    cx.listener(|this, checked: &bool, _, cx| {
                        this.config.record_images = *checked;
                        this.save_config(cx);
                        cx.notify();
                    }),
                ))
                .child(
                    h_flex()
                        .justify_between()
                        .items_center()
                        .gap_2()
                        .child(div().text_sm().child("历史保留上限"))
                        .child(limit_row),
                )
                .child(
                    h_flex()
                        .justify_between()
                        .items_center()
                        .child(
                            div()
                                .text_xs()
                                .text_color(muted_fg)
                                .child(format!("当前未置顶历史：{unpinned_count} 条")),
                        )
                        .child(
                            Button::new("clear-unpinned-history")
                                .danger()
                                .xsmall()
                                .label("清空未置顶历史")
                                .disabled(unpinned_count == 0)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    match this.history.clear_unpinned() {
                                        Ok(removed) => {
                                            this.rebuild_history_search_index();
                                            this.rebuild_entries();
                                            if let Some(super::OverlayView::Settings(ref mut s)) =
                                                this.overlay
                                            {
                                                s.feedback = Some((
                                                    true,
                                                    format!("已清理 {removed} 条未置顶记录").into(),
                                                ));
                                            }
                                        }
                                        Err(error) => {
                                            if let Some(super::OverlayView::Settings(ref mut s)) =
                                                this.overlay
                                            {
                                                s.feedback = Some((false, error.into()));
                                            }
                                        }
                                    }
                                    cx.notify();
                                })),
                        ),
                ),
        ),
    );

    // 隐私排除应用
    let mut chips = h_flex().gap_1p5().flex_wrap();
    for (ix, app_name) in app_view.config.excluded_apps.iter().enumerate() {
        let name_display = app_name.clone();
        chips = chips.child(
            h_flex()
                .px_2()
                .py_0p5()
                .gap_1()
                .items_center()
                .rounded_sm()
                .bg(secondary_bg)
                .child(div().text_xs().child(name_display))
                .child(
                    Button::new(("remove-excluded", ix))
                        .ghost()
                        .xsmall()
                        .icon(IconName::X)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if ix < this.config.excluded_apps.len() {
                                this.config.excluded_apps.remove(ix);
                                this.save_config(cx);
                                cx.notify();
                            }
                        })),
                ),
        );
    }

    content = content.child(
        section_box("隐私保护 · 排除应用", border, background).child(
            v_flex()
                .gap_2()
                .child(div().text_xs().text_color(muted_fg).child(
                    "来自密码管理器或以下应用复制的内容将被自动忽略，同时支持系统隐私剪贴板标记。",
                ))
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            div()
                                .flex_1()
                                .child(Input::new(&settings.exclude_app_input)),
                        )
                        .child(
                            Button::new("add-excluded-app")
                                .outline()
                                .small()
                                .icon(IconName::Plus)
                                .label("添加")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    let val = match &this.overlay {
                                        Some(super::OverlayView::Settings(s)) => {
                                            s.exclude_app_input.read(cx).value().trim().to_owned()
                                        }
                                        _ => return,
                                    };
                                    if val.is_empty() {
                                        return;
                                    }
                                    if !this
                                        .config
                                        .excluded_apps
                                        .iter()
                                        .any(|a| a.eq_ignore_ascii_case(&val))
                                    {
                                        this.config.excluded_apps.push(val);
                                        this.save_config(cx);
                                    }
                                    if let Some(super::OverlayView::Settings(ref s)) = this.overlay
                                    {
                                        s.exclude_app_input
                                            .update(cx, |st, cx| st.set_value("", window, cx));
                                    }
                                    cx.notify();
                                })),
                        ),
                )
                .child(chips),
        ),
    );

    v_flex()
        .size_full()
        .p_3()
        .gap_3()
        .child(
            h_flex()
                .w_full()
                .gap_2()
                .items_center()
                .child(
                    Button::new("settings-back")
                        .ghost()
                        .small()
                        .icon(IconName::ArrowLeft)
                        .label("返回 (Esc)")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.overlay = None;
                            this.focus_search(window, cx);
                            cx.notify();
                        })),
                )
                .child(div().text_sm().font_semibold().child("偏好设置"))
                .child(div().flex_1())
                .child(
                    div()
                        .text_xs()
                        .text_color(primary)
                        .child(format!("v{}", env!("CARGO_PKG_VERSION"))),
                ),
        )
        .child(
            div()
                .id("settings-scroll")
                .flex_1()
                .min_h_0()
                .w_full()
                .overflow_y_scrollbar()
                .pr_1()
                .child(content),
        )
        .into_any_element()
}

fn section_box(
    title: &'static str,
    border: gpui_kit::Hsla,
    background: gpui_kit::Hsla,
) -> gpui_kit::Div {
    v_flex()
        .w_full()
        .gap_2()
        .p_3()
        .rounded_md()
        .border_1()
        .border_color(border)
        .bg(background)
        .child(div().text_xs().font_semibold().child(title))
}

fn setting_switch_row(
    id: &'static str,
    title: &'static str,
    subtitle: Option<&'static str>,
    checked: bool,
    muted_fg: gpui_kit::Hsla,
    on_change: impl Fn(&bool, &mut Window, &mut gpui_kit::App) + 'static,
) -> AnyElement {
    h_flex()
        .w_full()
        .justify_between()
        .items_center()
        .gap_3()
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .child(div().text_sm().child(title))
                .when_some(subtitle, |this, sub| {
                    this.child(div().text_xs().text_color(muted_fg).child(sub))
                }),
        )
        .child(Switch::new(id).checked(checked).on_change(on_change))
        .into_any_element()
}

impl QuickPasteApp {
    pub(crate) fn apply_hotkey_setting(
        &mut self,
        spec: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let canonical = match config::canonicalize_hotkey(spec) {
            Ok(c) => c,
            Err(err) => {
                if let Some(super::OverlayView::Settings(ref mut s)) = self.overlay {
                    s.feedback = Some((false, err.into()));
                }
                cx.notify();
                return;
            }
        };

        let result = cx.global_mut::<HotkeyManagerHolder>().rebind(&canonical);
        match result {
            Ok(status) => {
                self.hotkey = status;
                self.config.hotkey = canonical.clone();
                self.save_config(cx);
                if let Some(super::OverlayView::Settings(ref mut s)) = self.overlay {
                    s.hotkey_input
                        .update(cx, |st, cx| st.set_value(&canonical, window, cx));
                    s.feedback = Some((
                        true,
                        format!(
                            "快捷键已切换为 {}",
                            config::format_hotkey_display(&canonical)
                        )
                        .into(),
                    ));
                }
            }
            Err(err) => {
                if let Some(super::OverlayView::Settings(ref mut s)) = self.overlay {
                    s.feedback = Some((false, err.into()));
                }
            }
        }
        cx.notify();
    }
}
