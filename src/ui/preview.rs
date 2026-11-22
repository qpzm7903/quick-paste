use gpui_kit::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ObjectFit, ParentElement as _,
    SharedString, Styled as _, StyledImage as _,
    assets::IconName,
    component::{
        ActiveTheme as _, Icon, Sizable as _, StyledExt as _,
        button::{Button, ButtonVariants as _},
        h_flex,
        scroll::ScrollableElement as _,
        v_flex,
    },
    div, img,
    prelude::FluentBuilder as _,
    px, rgba,
};

use super::{DisplayEntry, EntryId, QuickPasteApp};
use crate::{
    content::{self, ContentKind},
    memo,
};

pub fn render_preview(
    app: &QuickPasteApp,
    entry: &DisplayEntry,
    now_secs: u64,
    cx: &mut Context<QuickPasteApp>,
) -> AnyElement {
    let theme = cx.theme();
    let border = theme.border;
    let background = theme.background;
    let muted_fg = theme.muted_foreground;
    let secondary_bg = theme.secondary;
    let primary = theme.primary;
    let danger = theme.danger;

    let entry_paste = entry.clone();
    let entry_plain = entry.clone();
    let entry_copy = entry.clone();
    let entry_pin = entry.clone();
    let entry_save = entry.clone();
    let entry_del = entry.clone();
    let url_to_open = (entry.kind == ContentKind::Link).then(|| entry.text.trim().to_owned());

    let source_text = if entry.is_snippet {
        "常用片段".to_owned()
    } else if let Some(ref app_name) = entry.source_app {
        format!("来自 {app_name}")
    } else {
        "剪贴板记录".to_owned()
    };

    let time_or_usage = if entry.is_snippet {
        format!("已使用 {} 次", entry.usage_count)
    } else {
        format!(
            "复制于 {} · 已使用 {} 次",
            content::format_relative_time(entry.copied_at, now_secs),
            entry.usage_count
        )
    };

    let body: AnyElement = match entry.kind {
        ContentKind::Image => {
            let path = entry
                .image_file
                .as_deref()
                .or(entry.thumb_file.as_deref())
                .map(|f| app.history.image_path(f));
            let dims = entry
                .image_dims
                .map(|(w, h)| {
                    format!(
                        "{w} × {h} px · {}",
                        content::format_byte_size(entry.byte_size)
                    )
                })
                .unwrap_or_else(|| content::format_byte_size(entry.byte_size));

            v_flex()
                .flex_1()
                .min_h_0()
                .gap_3()
                .items_center()
                .justify_center()
                .children(path.map(|p| {
                    div()
                        .w_full()
                        .flex_1()
                        .min_h(px(180.0))
                        .max_h(px(360.0))
                        .rounded_md()
                        .border_1()
                        .border_color(border)
                        .bg(secondary_bg)
                        .overflow_hidden()
                        .flex()
                        .items_center()
                        .justify_center()
                        .p_2()
                        .child(img(p).size_full().object_fit(ObjectFit::Contain))
                }))
                .child(div().text_xs().text_color(muted_fg).child(dims))
                .into_any_element()
        }
        ContentKind::Color => {
            let parsed = content::parse_color(&entry.text);
            let swatch = parsed
                .map(|c| rgba(c.to_rgba_u32()))
                .unwrap_or_else(|| rgba(0x888888ff));
            let hex = parsed.map(|c| c.to_hex()).unwrap_or_default();
            let rgb_str = parsed.map(|c| c.to_rgb_string()).unwrap_or_default();

            v_flex()
                .flex_1()
                .gap_3()
                .child(
                    div()
                        .w_full()
                        .h(px(110.0))
                        .rounded_md()
                        .border_1()
                        .border_color(border)
                        .bg(swatch),
                )
                .child(
                    v_flex()
                        .gap_1()
                        .p_3()
                        .rounded_md()
                        .border_1()
                        .border_color(border)
                        .bg(background)
                        .child(div().text_sm().font_semibold().child(entry.text.clone()))
                        .when(!hex.is_empty(), |this| {
                            this.child(
                                div()
                                    .text_xs()
                                    .text_color(muted_fg)
                                    .child(format!("HEX: {hex}")),
                            )
                        })
                        .when(!rgb_str.is_empty(), |this| {
                            this.child(
                                div()
                                    .text_xs()
                                    .text_color(muted_fg)
                                    .child(format!("RGB: {rgb_str}")),
                            )
                        }),
                )
                .into_any_element()
        }
        _ => {
            let summary = content::char_and_line_summary(&entry.text);
            v_flex()
                .flex_1()
                .min_h_0()
                .gap_2()
                .child(
                    div()
                        .id("preview-text-scroll")
                        .flex_1()
                        .min_h_0()
                        .w_full()
                        .p_3()
                        .rounded_md()
                        .border_1()
                        .border_color(border)
                        .bg(background)
                        .overflow_y_scrollbar()
                        .text_sm()
                        .line_height(px(22.0))
                        .child(entry.text.clone()),
                )
                .child(div().text_xs().text_color(muted_fg).child(summary))
                .into_any_element()
        }
    };

    v_flex()
        .size_full()
        .p_3()
        .gap_3()
        // 顶部返回栏
        .child(
            h_flex()
                .w_full()
                .gap_2()
                .items_center()
                .child(
                    Button::new("preview-back")
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
                .child(
                    div()
                        .px_2()
                        .py_0p5()
                        .rounded_sm()
                        .bg(secondary_bg)
                        .text_xs()
                        .child(entry.kind.label()),
                )
                .child(div().flex_1())
                .child(
                    Button::new("preview-pin")
                        .ghost()
                        .small()
                        .icon(
                            Icon::new(if entry.is_pinned {
                                IconName::StarFill
                            } else {
                                IconName::Star
                            })
                            .text_color(primary),
                        )
                        .tooltip(if entry.is_pinned {
                            "取消置顶"
                        } else {
                            "置顶"
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.toggle_pin_entry(&entry_pin, cx);
                        })),
                )
                .child(
                    Button::new("preview-delete")
                        .ghost()
                        .small()
                        .icon(Icon::new(IconName::Trash).text_color(danger))
                        .tooltip("删除")
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.delete_entry(&entry_del, cx);
                            this.overlay = None;
                            this.focus_search(window, cx);
                            cx.notify();
                        })),
                ),
        )
        // 可选标题
        .children(
            entry
                .title
                .clone()
                .map(|title| div().text_base().font_semibold().child(title)),
        )
        // 主体内容
        .child(body)
        // 标签与来源信息
        .child(
            v_flex()
                .gap_1()
                .when(!entry.tags.is_empty(), |this| {
                    let tags = entry
                        .tags
                        .iter()
                        .map(|t| format!("#{t}"))
                        .collect::<Vec<_>>()
                        .join("  ");
                    this.child(div().text_xs().text_color(primary).child(tags))
                })
                .child(
                    div()
                        .text_xs()
                        .text_color(muted_fg)
                        .child(format!("{source_text} · {time_or_usage}")),
                ),
        )
        // 底部动作按钮
        .child(
            h_flex()
                .w_full()
                .gap_2()
                .flex_wrap()
                .child(
                    Button::new("preview-paste")
                        .primary()
                        .small()
                        .icon(IconName::ClipboardPaste)
                        .label("粘贴 (Enter)")
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.overlay = None;
                            this.paste_entry(&entry_paste, false, window, cx);
                        })),
                )
                .when(entry.kind != ContentKind::Image, |this| {
                    this.child(
                        Button::new("preview-paste-plain")
                            .outline()
                            .small()
                            .label("纯文本 (⇧Enter)")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.overlay = None;
                                this.paste_entry(&entry_plain, true, window, cx);
                            })),
                    )
                })
                .child(
                    Button::new("preview-copy")
                        .outline()
                        .small()
                        .icon(IconName::Copy)
                        .label("仅复制")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.copy_entry_only(&entry_copy, cx);
                        })),
                )
                .when_some(url_to_open, |this, url| {
                    this.child(
                        Button::new("preview-open-url")
                            .outline()
                            .small()
                            .icon(IconName::ExternalLink)
                            .label("打开链接")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if let Err(error) = memo::open_url(&url) {
                                    this.notice =
                                        Some(SharedString::from(format!("打开链接失败：{error}")));
                                    cx.notify();
                                }
                            })),
                    )
                })
                .when(matches!(entry.id, EntryId::Snippet(_)), |this| {
                    let idx = match entry.id {
                        EntryId::Snippet(i) => i,
                        EntryId::History(_) => 0,
                    };
                    this.child(
                        Button::new("preview-edit-snippet")
                            .outline()
                            .small()
                            .icon(IconName::Pencil)
                            .label("编辑")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.open_snippet_editor(Some(idx), None, window, cx);
                            })),
                    )
                })
                .when(
                    matches!(entry.id, EntryId::History(_)) && entry.kind != ContentKind::Image,
                    |this| {
                        this.child(
                            Button::new("preview-save-snippet")
                                .outline()
                                .small()
                                .icon(IconName::Bookmark)
                                .label("存为常用片段")
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.save_as_snippet(&entry_save, window, cx);
                                })),
                        )
                    },
                ),
        )
        .into_any_element()
}
