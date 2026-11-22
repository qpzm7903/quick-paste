use gpui_kit::{
    AnyElement, AppContext as _, Context, Entity, Focusable as _, IntoElement, ParentElement as _,
    SharedString, Styled as _, Window,
    assets::IconName,
    component::{
        ActiveTheme as _, Sizable as _, StyledExt as _,
        button::{Button, ButtonVariants as _},
        h_flex,
        input::{Input, InputState, Textarea, TextareaState},
        switch::Switch,
        v_flex,
    },
    div,
    prelude::FluentBuilder as _,
    px,
};

use super::QuickPasteApp;
use crate::memo;

pub struct SnippetEditorState {
    pub editing_index: Option<usize>,
    pub title_input: Entity<InputState>,
    pub body_input: Entity<TextareaState>,
    pub tags_input: Entity<InputState>,
    pub is_pinned: bool,
    pub error: Option<SharedString>,
}

impl SnippetEditorState {
    pub fn new(
        editing_index: Option<usize>,
        initial: Option<&memo::Memo>,
        prefill_text: Option<String>,
        window: &mut Window,
        cx: &mut Context<QuickPasteApp>,
    ) -> Self {
        let initial_title = initial.and_then(|m| m.title.clone()).unwrap_or_default();
        let initial_body = initial
            .map(|m| m.text.clone())
            .or(prefill_text)
            .unwrap_or_default();
        let initial_tags = initial.map(|m| m.tags.join(", ")).unwrap_or_default();
        let is_pinned = initial.is_some_and(|m| m.is_pinned);

        let title_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("可选标题（如：工作邮箱 / SSH 命令）")
                .default_value(initial_title)
        });
        let body_input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("输入常用片段正文…")
                .default_value(initial_body)
        });
        let tags_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("标签（用逗号或空格分隔，如：工作, 常用）")
                .default_value(initial_tags)
        });

        body_input.focus_handle(cx).focus(window, cx);

        Self {
            editing_index,
            title_input,
            body_input,
            tags_input,
            is_pinned,
            error: None,
        }
    }
}

pub fn render_editor(editor: &SnippetEditorState, cx: &mut Context<QuickPasteApp>) -> AnyElement {
    let theme = cx.theme();
    let muted_fg = theme.muted_foreground;
    let danger = theme.danger;
    let heading = if editor.editing_index.is_some() {
        "编辑常用片段"
    } else {
        "新建常用片段"
    };
    let save_shortcut = if cfg!(target_os = "macos") {
        "保存 (⌘S)"
    } else {
        "保存 (Ctrl+S)"
    };

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
                    Button::new("editor-back")
                        .ghost()
                        .small()
                        .icon(IconName::ArrowLeft)
                        .label("取消 (Esc)")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.overlay = None;
                            this.focus_search(window, cx);
                            cx.notify();
                        })),
                )
                .child(div().text_sm().font_semibold().child(heading))
                .child(div().flex_1()),
        )
        .child(
            v_flex()
                .flex_1()
                .min_h_0()
                .gap_3()
                .child(
                    v_flex()
                        .gap_1()
                        .child(div().text_xs().text_color(muted_fg).child("标题（可选）"))
                        .child(Input::new(&editor.title_input).cleanable(true)),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .min_h_0()
                        .gap_1()
                        .child(div().text_xs().text_color(muted_fg).child("片段内容 *"))
                        .child(Textarea::new(&editor.body_input).h(px(220.0))),
                )
                .child(
                    v_flex()
                        .gap_1()
                        .child(div().text_xs().text_color(muted_fg).child("标签"))
                        .child(Input::new(&editor.tags_input).cleanable(true)),
                )
                .child(
                    h_flex()
                        .w_full()
                        .items_center()
                        .justify_between()
                        .child(div().text_sm().child("置顶显示在最前"))
                        .child(
                            Switch::new("editor-pin-switch")
                                .checked(editor.is_pinned)
                                .on_change(cx.listener(|this, checked: &bool, _, cx| {
                                    if let Some(super::OverlayView::SnippetEditor(ref mut ed)) =
                                        this.overlay
                                    {
                                        ed.is_pinned = *checked;
                                        cx.notify();
                                    }
                                })),
                        ),
                )
                .when_some(editor.error.clone(), |this, err| {
                    this.child(div().text_xs().text_color(danger).child(err))
                }),
        )
        .child(
            h_flex()
                .w_full()
                .gap_2()
                .justify_end()
                .child(
                    Button::new("editor-cancel")
                        .outline()
                        .small()
                        .label("取消")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.overlay = None;
                            this.focus_search(window, cx);
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("editor-save")
                        .primary()
                        .small()
                        .icon(IconName::Check)
                        .label(save_shortcut)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.commit_snippet_editor(window, cx);
                        })),
                ),
        )
        .into_any_element()
}
