use std::path::PathBuf;

use gpui_kit::{
    AnyElement, ClickEvent, Context, InteractiveElement as _, IntoElement, ObjectFit,
    ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _,
    StyledImage as _,
    assets::IconName,
    component::{
        ActiveTheme as _, Icon, Sizable as _, StyledExt as _,
        button::{Button, ButtonVariants as _},
        h_flex, v_flex,
    },
    div, img,
    prelude::FluentBuilder as _,
    px, rgb, rgba,
};

use super::{DisplayEntry, EntryId, QuickPasteApp};
use crate::{
    config::PasteTrigger,
    content::{self, ContentKind},
    memo,
};

const MAX_TEXT_LINES: usize = 3;
const MAX_LINK_LINES: usize = 2;
const META_HEIGHT: f32 = 18.0;
const TITLE_HEIGHT: f32 = 20.0;
const ROW_LINE_HEIGHT: f32 = 21.0;
const ROW_TAG_HEIGHT: f32 = 18.0;
const IMAGE_PREVIEW_HEIGHT: f32 = 82.0;
const COLOR_ROW_HEIGHT: f32 = 34.0;
const ROW_PADDING_Y: f32 = 10.0;
const ROW_PADDING_X: f32 = 12.0;
const ROW_MARGIN_X: f32 = 8.0;
const ROW_GAP: f32 = 8.0;
const CONTENT_GAP: f32 = 5.0;

pub fn row_height(entry: &DisplayEntry) -> f32 {
    let mut inner = META_HEIGHT + CONTENT_GAP;

    if entry.title.is_some() {
        inner += TITLE_HEIGHT + 3.0;
    }

    inner += match entry.kind {
        ContentKind::Image => IMAGE_PREVIEW_HEIGHT,
        ContentKind::Color => COLOR_ROW_HEIGHT,
        ContentKind::Link => {
            memo::display_line_count(&entry.text, MAX_LINK_LINES) as f32 * ROW_LINE_HEIGHT
        }
        ContentKind::Text | ContentKind::Code => {
            memo::display_line_count(&entry.text, MAX_TEXT_LINES) as f32 * ROW_LINE_HEIGHT
        }
    };

    if !entry.tags.is_empty() {
        inner += CONTENT_GAP + ROW_TAG_HEIGHT;
    }

    ROW_PADDING_Y * 2.0 + inner + ROW_GAP
}

pub fn render_card(
    app: &QuickPasteApp,
    row_idx: usize,
    entry: &DisplayEntry,
    now_secs: u64,
    cx: &mut Context<QuickPasteApp>,
) -> AnyElement {
    let total_height = row_height(entry);
    let card_height = total_height - ROW_GAP;
    let scrolling = app.smooth_scroll.is_active();
    let is_selected = app.selected_index == row_idx;
    let is_hovered = !scrolling && app.hovered_index == Some(row_idx);

    let theme = cx.theme();
    let card_bg = if is_selected {
        theme.list_active
    } else {
        theme.background
    };
    let border_color = if is_selected {
        theme.primary
    } else if is_hovered {
        theme.ring
    } else {
        theme.border
    };
    let primary = theme.primary;
    let muted_fg = theme.muted_foreground;
    let secondary_bg = theme.secondary;
    let secondary_fg = theme.secondary_foreground;
    let danger = theme.danger;

    let entry_id = entry.id.clone();
    let entry_for_click = entry.clone();
    let entry_for_pin = entry.clone();
    let entry_for_preview = entry.clone();
    let entry_for_copy = entry.clone();
    let entry_for_save = entry.clone();
    let entry_for_delete = entry.clone();
    let paste_trigger = app.config.paste_trigger;

    let domain_badge = (entry.kind == ContentKind::Link)
        .then(|| content::extract_domain(&entry.text))
        .flatten();

    let source_label: SharedString = if entry.is_snippet {
        "常用片段".into()
    } else if let Some(ref src) = entry.source_app {
        format!("剪贴板 · {src}").into()
    } else {
        "剪贴板".into()
    };

    let meta_right: SharedString = if let Some(ref domain) = domain_badge {
        if entry.is_snippet {
            domain.clone().into()
        } else {
            format!(
                "{domain} · {}",
                content::format_relative_time(entry.copied_at, now_secs)
            )
            .into()
        }
    } else if entry.is_snippet {
        if entry.usage_count > 0 {
            format!("{} 次", entry.usage_count).into()
        } else {
            "".into()
        }
    } else {
        content::format_relative_time(entry.copied_at, now_secs).into()
    };

    let shortcut_badge = (row_idx < 9).then(|| {
        if cfg!(target_os = "macos") {
            SharedString::from(format!("⌘{}", row_idx + 1))
        } else {
            SharedString::from(format!("^{}", row_idx + 1))
        }
    });

    let thumb_path: Option<PathBuf> = entry
        .thumb_file
        .as_deref()
        .or(entry.image_file.as_deref())
        .map(|file| app.history.image_path(file));

    let content_el = render_card_body(entry, thumb_path, secondary_bg, muted_fg, border_color);

    let tags_el = (!entry.tags.is_empty()).then(|| {
        let tags_str = entry
            .tags
            .iter()
            .map(|t| format!("#{t}"))
            .collect::<Vec<_>>()
            .join("  ");
        div()
            .w_full()
            .h(px(ROW_TAG_HEIGHT))
            .truncate()
            .text_xs()
            .line_height(px(ROW_TAG_HEIGHT))
            .text_color(muted_fg)
            .child(tags_str)
    });

    let (kind_icon, kind_color) = kind_badge_style(entry.kind, theme);

    v_flex()
        .id(("memo-row", row_idx))
        .w_full()
        .h(px(total_height))
        .px(px(ROW_MARGIN_X))
        .pb(px(ROW_GAP))
        .child(
            v_flex()
                .id(("memo-card", row_idx))
                .relative()
                .w_full()
                .h(px(card_height))
                .gap(px(CONTENT_GAP))
                .overflow_hidden()
                .rounded_md()
                .border_1()
                .border_color(border_color)
                .bg(card_bg)
                .px(px(ROW_PADDING_X))
                .py(px(ROW_PADDING_Y))
                .when(is_hovered || is_selected, |this| this.shadow_sm())
                .on_hover(cx.listener(move |this, hovered: &bool, _window, cx| {
                    let next = if *hovered {
                        Some(row_idx)
                    } else if this.hovered_index == Some(row_idx) {
                        None
                    } else {
                        return;
                    };
                    if this.hovered_index != next {
                        this.hovered_index = next;
                        if !this.smooth_scroll.is_active() {
                            cx.notify();
                        }
                    }
                }))
                .cursor_pointer()
                .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                    this.selected_index = row_idx;
                    let should_paste = match paste_trigger {
                        PasteTrigger::SingleClick => true,
                        PasteTrigger::DoubleClick => event.click_count() >= 2,
                    };
                    if should_paste {
                        this.paste_entry(&entry_for_click, event.modifiers().shift, window, cx);
                    } else {
                        cx.notify();
                    }
                }))
                // 顶部元信息栏：来源 + 类型 + 时间/次数 + 快捷键序号 + 悬停操作
                .child(
                    h_flex()
                        .w_full()
                        .h(px(META_HEIGHT))
                        .gap_1p5()
                        .items_center()
                        .child(Icon::new(kind_icon).xsmall().text_color(kind_color))
                        .child(
                            div()
                                .text_xs()
                                .font_medium()
                                .text_color(if entry.is_snippet { primary } else { muted_fg })
                                .truncate()
                                .child(source_label),
                        )
                        .when(!meta_right.is_empty(), |this| {
                            this.child(
                                div()
                                    .text_xs()
                                    .text_color(muted_fg)
                                    .child(format!("· {meta_right}")),
                            )
                        })
                        .child(div().flex_1())
                        .when(!is_hovered, |this| {
                            this.children(shortcut_badge.map(|badge| {
                                div()
                                    .px_1()
                                    .rounded_sm()
                                    .bg(secondary_bg)
                                    .text_xs()
                                    .text_color(secondary_fg)
                                    .child(badge)
                            }))
                        })
                        // 悬停动作条（滚动时隐藏）或常驻置顶星标
                        .when(is_hovered || entry.is_pinned, |this| {
                            this.child(
                                h_flex()
                                    .gap_0p5()
                                    .items_center()
                                    .when(is_hovered, |bar| {
                                        let id_str = match &entry_id {
                                            EntryId::History(id) => format!("h-{id}"),
                                            EntryId::Snippet(ix) => format!("s-{ix}"),
                                        };
                                        let snippet_idx = match entry_id {
                                            EntryId::Snippet(ix) => Some(ix),
                                            EntryId::History(_) => None,
                                        };
                                        bar.child(
                                            Button::new(SharedString::from(format!(
                                                "preview-{id_str}"
                                            )))
                                            .ghost()
                                            .xsmall()
                                            .icon(IconName::Eye)
                                            .tooltip("预览 (Space)")
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                cx.stop_propagation();
                                                this.open_preview(entry_for_preview.clone(), cx);
                                            })),
                                        )
                                        .child(
                                            Button::new(SharedString::from(format!(
                                                "copy-{id_str}"
                                            )))
                                            .ghost()
                                            .xsmall()
                                            .icon(IconName::Copy)
                                            .tooltip("仅复制到剪贴板")
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                cx.stop_propagation();
                                                this.copy_entry_only(&entry_for_copy, cx);
                                            })),
                                        )
                                        .when_some(snippet_idx, |bar, ix| {
                                            bar.child(
                                                Button::new(SharedString::from(format!(
                                                    "edit-{id_str}"
                                                )))
                                                .ghost()
                                                .xsmall()
                                                .icon(IconName::Pencil)
                                                .tooltip("编辑片段")
                                                .on_click(cx.listener(
                                                    move |this, _, window, cx| {
                                                        cx.stop_propagation();
                                                        this.open_snippet_editor(
                                                            Some(ix),
                                                            None,
                                                            window,
                                                            cx,
                                                        );
                                                    },
                                                )),
                                            )
                                        })
                                        .when(
                                            snippet_idx.is_none()
                                                && entry.kind != ContentKind::Image,
                                            |bar| {
                                                bar.child(
                                                    Button::new(SharedString::from(format!(
                                                        "save-{id_str}"
                                                    )))
                                                    .ghost()
                                                    .xsmall()
                                                    .icon(IconName::Bookmark)
                                                    .tooltip("存为常用片段")
                                                    .on_click(cx.listener(
                                                        move |this, _, window, cx| {
                                                            cx.stop_propagation();
                                                            this.save_as_snippet(
                                                                &entry_for_save,
                                                                window,
                                                                cx,
                                                            );
                                                        },
                                                    )),
                                                )
                                            },
                                        )
                                        .child(
                                            Button::new(SharedString::from(format!(
                                                "del-{id_str}"
                                            )))
                                            .ghost()
                                            .xsmall()
                                            .icon(Icon::new(IconName::Trash).text_color(danger))
                                            .tooltip("删除")
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                cx.stop_propagation();
                                                this.delete_entry(&entry_for_delete, cx);
                                            })),
                                        )
                                    })
                                    .child(
                                        Button::new(("pin-btn", row_idx))
                                            .ghost()
                                            .xsmall()
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
                                                cx.stop_propagation();
                                                this.toggle_pin_entry(&entry_for_pin, cx);
                                            })),
                                    ),
                            )
                        }),
                )
                // 可选片段标题
                .children(entry.title.clone().map(|title| {
                    div()
                        .w_full()
                        .h(px(TITLE_HEIGHT))
                        .truncate()
                        .text_sm()
                        .font_semibold()
                        .line_height(px(TITLE_HEIGHT))
                        .child(title)
                }))
                // 内容区
                .child(content_el)
                // 可选标签行
                .children(tags_el),
        )
        .into_any_element()
}

fn render_card_body(
    entry: &DisplayEntry,
    thumb_path: Option<PathBuf>,
    secondary_bg: gpui_kit::Hsla,
    muted_fg: gpui_kit::Hsla,
    border_color: gpui_kit::Hsla,
) -> AnyElement {
    match entry.kind {
        ContentKind::Image => {
            let dims = entry
                .image_dims
                .map(|(w, h)| format!("{w} × {h} px"))
                .unwrap_or_else(|| "图片".to_owned());
            let size_label = content::format_byte_size(entry.byte_size);

            h_flex()
                .w_full()
                .h(px(IMAGE_PREVIEW_HEIGHT))
                .gap_3()
                .items_center()
                .children(thumb_path.map(|path| {
                    div()
                        .w(px(124.0))
                        .h(px(IMAGE_PREVIEW_HEIGHT))
                        .rounded_sm()
                        .border_1()
                        .border_color(border_color)
                        .bg(secondary_bg)
                        .overflow_hidden()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            img(path)
                                .w(px(124.0))
                                .h(px(IMAGE_PREVIEW_HEIGHT))
                                .object_fit(ObjectFit::Contain),
                        )
                }))
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap_1()
                        .child(div().text_sm().font_medium().child(dims))
                        .child(
                            div()
                                .text_xs()
                                .text_color(muted_fg)
                                .child(format!("PNG · {size_label}")),
                        ),
                )
                .into_any_element()
        }
        ContentKind::Color => {
            let parsed = content::parse_color(&entry.text);
            let swatch = parsed
                .map(|c| rgba(c.to_rgba_u32()))
                .unwrap_or_else(|| rgba(0x888888ff));
            let extra = parsed
                .map(|c| format!("{} · {}", c.to_hex(), c.to_rgb_string()))
                .unwrap_or_default();

            h_flex()
                .w_full()
                .h(px(COLOR_ROW_HEIGHT))
                .gap_2p5()
                .items_center()
                .child(
                    div()
                        .w(px(30.0))
                        .h(px(30.0))
                        .flex_shrink_0()
                        .rounded_md()
                        .border_1()
                        .border_color(border_color)
                        .bg(swatch),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .child(
                            div()
                                .text_sm()
                                .font_medium()
                                .truncate()
                                .child(entry.text.clone()),
                        )
                        .when(!extra.is_empty(), |this| {
                            this.child(div().text_xs().text_color(muted_fg).truncate().child(extra))
                        }),
                )
                .into_any_element()
        }
        ContentKind::Link => {
            let lines = memo::display_line_count(&entry.text, MAX_LINK_LINES);
            div()
                .w_full()
                .h(px(lines as f32 * ROW_LINE_HEIGHT))
                .overflow_hidden()
                .text_sm()
                .line_height(px(ROW_LINE_HEIGHT))
                .text_color(rgb(0x2563eb))
                .line_clamp(lines)
                .child(entry.text.clone())
                .into_any_element()
        }
        ContentKind::Code => {
            let lines = memo::display_line_count(&entry.text, MAX_TEXT_LINES);
            div()
                .w_full()
                .h(px(lines as f32 * ROW_LINE_HEIGHT))
                .overflow_hidden()
                .px_2()
                .rounded_sm()
                .bg(secondary_bg)
                .text_xs()
                .line_height(px(ROW_LINE_HEIGHT))
                .line_clamp(lines)
                .child(entry.text.clone())
                .into_any_element()
        }
        ContentKind::Text => {
            let lines = memo::display_line_count(&entry.text, MAX_TEXT_LINES);
            div()
                .w_full()
                .h(px(lines as f32 * ROW_LINE_HEIGHT))
                .overflow_hidden()
                .text_sm()
                .line_height(px(ROW_LINE_HEIGHT))
                .line_clamp(lines)
                .child(entry.text.clone())
                .into_any_element()
        }
    }
}

fn kind_badge_style(
    kind: ContentKind,
    theme: &gpui_kit::component::Theme,
) -> (IconName, gpui_kit::Hsla) {
    match kind {
        ContentKind::Text => (IconName::FileText, theme.muted_foreground),
        ContentKind::Link => (IconName::Link, theme.link),
        ContentKind::Color => (IconName::Palette, theme.warning),
        ContentKind::Code => (IconName::Code, theme.info),
        ContentKind::Image => (IconName::Image, theme.success),
    }
}
