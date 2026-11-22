mod card;
mod preview;
mod settings;
mod smooth_scroll;
mod snippet_editor;

use std::{collections::HashMap, path::Path, rc::Rc, sync::atomic::Ordering, time::Duration};

use gpui_kit::{
    AnyElement, AppContext as _, ClickEvent, Context, Entity, Focusable as _,
    InteractiveElement as _, IntoElement, KeyDownEvent, Modifiers, ParentElement as _, Pixels,
    Render, ScrollEvent, SharedString, Size, Styled as _, Subscription, Window,
    assets::IconName,
    component::{
        ActiveTheme as _, Icon, Sizable as _, StyledExt as _, VirtualListScrollHandle,
        WindowExt as _,
        button::{Button, ButtonVariants as _},
        h_flex,
        input::{Input, InputEvent, InputState},
        scroll::Scrollbar,
        tab::{Tab, TabBar},
        v_flex, v_virtual_list,
    },
    div, point, px, size,
};

use self::{
    preview::render_preview,
    settings::{SettingsState, render_settings},
    smooth_scroll::SmoothScroll,
    snippet_editor::{SnippetEditorState, render_editor},
};
use crate::{
    app::{self, HotkeyStatus, SharedAppState, TrayHolder},
    clipboard::{self, ClipboardEvent},
    config::{self, AppConfig},
    content::ContentKind,
    history::{self, HistoryStore},
    memo, platform, search, watcher,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActiveTab {
    All,
    Clipboard,
    Snippets,
}

impl ActiveTab {
    fn index(self) -> usize {
        match self {
            Self::All => 0,
            Self::Clipboard => 1,
            Self::Snippets => 2,
        }
    }

    fn from_index(idx: usize) -> Self {
        match idx {
            1 => Self::Clipboard,
            2 => Self::Snippets,
            _ => Self::All,
        }
    }

    fn next(self) -> Self {
        match self {
            Self::All => Self::Clipboard,
            Self::Clipboard => Self::Snippets,
            Self::Snippets => Self::All,
        }
    }

    fn prev(self) -> Self {
        match self {
            Self::All => Self::Snippets,
            Self::Clipboard => Self::All,
            Self::Snippets => Self::Clipboard,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentFilter {
    All,
    Pinned,
    Text,
    Image,
    Link,
    Color,
    Code,
}

impl ContentFilter {
    const ALL_FILTERS: [Self; 7] = [
        Self::All,
        Self::Pinned,
        Self::Text,
        Self::Image,
        Self::Link,
        Self::Color,
        Self::Code,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::All => "全部",
            Self::Pinned => "置顶",
            Self::Text => "文本",
            Self::Image => "图片",
            Self::Link => "链接",
            Self::Color => "颜色",
            Self::Code => "代码",
        }
    }

    fn matches(self, kind: ContentKind, is_pinned: bool) -> bool {
        match self {
            Self::All => true,
            Self::Pinned => is_pinned,
            Self::Text => kind == ContentKind::Text,
            Self::Image => kind == ContentKind::Image,
            Self::Link => kind == ContentKind::Link,
            Self::Color => kind == ContentKind::Color,
            Self::Code => kind == ContentKind::Code,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntryId {
    History(String),
    Snippet(usize),
}

#[derive(Debug, Clone)]
pub struct DisplayEntry {
    pub id: EntryId,
    pub is_snippet: bool,
    pub kind: ContentKind,
    pub title: Option<String>,
    pub text: String,
    pub tags: Vec<String>,
    pub source_app: Option<String>,
    pub copied_at: u64,
    pub usage_count: u64,
    pub is_pinned: bool,
    pub image_file: Option<String>,
    pub thumb_file: Option<String>,
    pub image_dims: Option<(u32, u32)>,
    pub byte_size: u64,
}

pub enum PendingOverlay {
    NewSnippet,
    Settings,
}

pub enum OverlayView {
    Preview(DisplayEntry),
    SnippetEditor(SnippetEditorState),
    Settings(SettingsState),
}

pub struct QuickPasteApp {
    pub(crate) state: SharedAppState,
    pub(crate) config: AppConfig,
    pub(crate) hotkey: HotkeyStatus,
    pub(crate) memos: Vec<memo::Memo>,
    pub(crate) memo_search_index: Vec<search::SearchIndexEntry>,
    pub(crate) history: HistoryStore,
    pub(crate) history_search_index: HashMap<String, search::SearchIndexEntry>,
    pub(crate) entries: Vec<DisplayEntry>,
    pub(crate) row_sizes: Rc<Vec<Size<Pixels>>>,
    pub(crate) list_scroll: VirtualListScrollHandle,
    pub(crate) smooth_scroll: SmoothScroll,
    pub(crate) search: Entity<InputState>,
    pub(crate) filter: String,
    pub(crate) active_tab: ActiveTab,
    pub(crate) content_filter: ContentFilter,
    pub(crate) active_tag: Option<String>,
    pub(crate) selected_index: usize,
    pub(crate) hovered_index: Option<usize>,
    pub(crate) overlay: Option<OverlayView>,
    pub(crate) pending_overlay: Option<PendingOverlay>,
    pub(crate) pending_focus_search: bool,
    pub(crate) notice: Option<SharedString>,
    pub(crate) paused: bool,
    _subscriptions: Vec<Subscription>,
}

impl QuickPasteApp {
    pub fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        state: SharedAppState,
        hotkey: HotkeyStatus,
    ) -> Self {
        let search = cx.new(|cx| {
            InputState::new(window, cx).placeholder("搜索剪贴板与片段（拼音、标签、应用名）…")
        });

        let search_sub =
            cx.subscribe_in(
                &search,
                window,
                |this, state, event, window, cx| match event {
                    InputEvent::Change => {
                        this.filter = state.read(cx).value().to_string();
                        this.rebuild_entries();
                        this.scroll_to_top();
                        cx.notify();
                    }
                    InputEvent::PressEnter { shift, .. } => {
                        this.filter = state.read(cx).value().to_string();
                        this.rebuild_entries();
                        if let Some(entry) = this.entries.get(this.selected_index).cloned() {
                            this.paste_entry(&entry, *shift, window, cx);
                        }
                    }
                    _ => {}
                },
            );

        let appearance_sub = cx.observe_window_appearance(window, |this, window, cx| {
            if this.config.theme == config::ThemePreference::System {
                app::apply_theme(this.config.theme, window.appearance(), cx);
            }
        });

        let activation_sub = cx.observe_window_activation(window, |this, window, _cx| {
            if !window.is_window_active() && this.config.hide_on_blur && this.overlay.is_none() {
                platform::hide_window(window);
            }
        });

        let config = state
            .shared_config
            .read()
            .map(|guard| guard.clone())
            .unwrap_or_default();

        let mut notice: Option<SharedString> = None;
        let memos = match Self::load_merged(&state.source_path, &state.memos_path) {
            Ok(memos) => {
                watcher::suppress_reload_for(Duration::from_millis(800));
                if let Err(error) = memo::save(&state.memos_path, &memos) {
                    notice = Some(format!("无法写入本地状态：{error}").into());
                }
                memos
            }
            Err(error) => {
                notice = Some(format!("无法读取备忘录：{error}").into());
                Vec::new()
            }
        };

        let history = HistoryStore::load(&state.data_dir);

        window.on_window_should_close(cx, |window, _cx| {
            platform::hide_window(window);
            false
        });

        let mut app_view = Self {
            state,
            config,
            hotkey,
            memos,
            memo_search_index: Vec::new(),
            history,
            history_search_index: HashMap::new(),
            entries: Vec::new(),
            row_sizes: Rc::new(Vec::new()),
            list_scroll: VirtualListScrollHandle::new(),
            smooth_scroll: SmoothScroll::default(),
            search,
            filter: String::new(),
            active_tab: ActiveTab::All,
            content_filter: ContentFilter::All,
            active_tag: None,
            selected_index: 0,
            hovered_index: None,
            overlay: None,
            pending_overlay: None,
            pending_focus_search: false,
            notice,
            paused: false,
            _subscriptions: vec![search_sub, appearance_sub, activation_sub],
        };
        app_view.rebuild_memo_search_index();
        app_view.rebuild_history_search_index();
        app_view.rebuild_entries();
        platform::register_main_window(window);
        app_view.search.focus_handle(cx).focus(window, cx);
        app_view
    }

    pub fn on_hotkey(&mut self, cx: &mut Context<Self>) {
        if platform::is_main_window_foreground() {
            platform::hide_main_window();
        } else {
            self.on_show_request(cx);
        }
    }

    pub fn on_show_request(&mut self, cx: &mut Context<Self>) {
        platform::show_main_window(self.config.position_on_cursor_screen);
        self.selected_index = 0;
        self.pending_focus_search = true;
        self.reload_memos(cx);
        cx.notify();
    }

    pub fn on_clipboard_event(&mut self, event: ClipboardEvent, cx: &mut Context<Self>) {
        let result = match event {
            ClipboardEvent::Text {
                text,
                source_app,
                source_app_id,
            } => self.history.record_text(
                text,
                source_app,
                source_app_id,
                self.config.max_history_items,
            ),
            ClipboardEvent::Image {
                width,
                height,
                rgba,
                source_app,
                source_app_id,
            } => self.history.record_image(
                &rgba,
                width,
                height,
                source_app,
                source_app_id,
                self.config.max_history_items,
                self.config.max_image_mb,
            ),
        };

        match result {
            Ok(true) => {
                self.rebuild_history_search_index();
                self.rebuild_entries();
                cx.notify();
            }
            Ok(false) => {}
            Err(error) => {
                self.notice = Some(error.into());
                cx.notify();
            }
        }
    }

    pub fn on_external_memos_changed(&mut self, cx: &mut Context<Self>) {
        self.reload_memos(cx);
    }

    pub fn on_tray_action(&mut self, action: crate::tray::TrayAction, cx: &mut Context<Self>) {
        match action {
            crate::tray::TrayAction::ToggleWindow => self.on_hotkey(cx),
            crate::tray::TrayAction::TogglePause => self.toggle_pause(cx),
            crate::tray::TrayAction::NewSnippet => {
                self.pending_overlay = Some(PendingOverlay::NewSnippet);
                self.on_show_request(cx);
            }
            crate::tray::TrayAction::OpenSettings => {
                self.pending_overlay = Some(PendingOverlay::Settings);
                self.on_show_request(cx);
            }
            crate::tray::TrayAction::OpenDataDir => {
                if let Err(error) = memo::reveal(&self.state.data_dir) {
                    self.notice = Some(format!("无法打开数据目录：{error}").into());
                    cx.notify();
                }
            }
            crate::tray::TrayAction::Quit => cx.quit(),
        }
    }

    pub(crate) fn toggle_pause(&mut self, cx: &mut Context<Self>) {
        self.paused = !self.paused;
        self.state
            .monitor_paused
            .store(self.paused, Ordering::Relaxed);
        if let Some(holder) = cx.try_global::<TrayHolder>() {
            holder.0.set_paused(self.paused);
        }
        cx.notify();
    }

    pub(crate) fn save_config(&mut self, cx: &mut Context<Self>) {
        self.config = self.config.clone().normalized();
        if let Ok(mut guard) = self.state.shared_config.write() {
            *guard = self.config.clone();
        }
        if let Err(error) = config::save(&self.state.config_path, &self.config) {
            self.notice = Some(format!("保存配置失败：{error}").into());
            cx.notify();
        }
    }

    fn rebuild_memo_search_index(&mut self) {
        self.memo_search_index = self
            .memos
            .iter()
            .map(|m| {
                let kind = crate::content::classify_text(&m.text);
                search::build_entry(m.title.as_deref(), &m.text, &m.tags, None, kind)
            })
            .collect();
    }

    pub(crate) fn rebuild_history_search_index(&mut self) {
        self.history_search_index = self
            .history
            .items()
            .iter()
            .map(|item| {
                let entry = search::build_entry(
                    None,
                    &item.text,
                    &[],
                    item.source_app.as_deref(),
                    item.kind,
                );
                (item.id.clone(), entry)
            })
            .collect();
    }

    pub(crate) fn rebuild_entries(&mut self) {
        self.smooth_scroll.reset();
        self.hovered_index = None;
        let query = self.filter.trim();
        let has_query = !query.is_empty();

        let mut candidates: Vec<(i64, usize, DisplayEntry)> = Vec::new();
        let mut order = 0usize;

        // 1. 常用片段
        if matches!(self.active_tab, ActiveTab::All | ActiveTab::Snippets) {
            for (ix, m) in self.memos.iter().enumerate() {
                let kind = crate::content::classify_text(&m.text);
                if let Some(ref tag_filter) = self.active_tag
                    && !m.tags.iter().any(|t| t.eq_ignore_ascii_case(tag_filter))
                {
                    continue;
                }
                if self.active_tab == ActiveTab::All
                    && !self.content_filter.matches(kind, m.is_pinned)
                {
                    continue;
                }
                if self.active_tab == ActiveTab::Snippets
                    && self.content_filter == ContentFilter::Pinned
                    && !m.is_pinned
                {
                    continue;
                }

                let score = if has_query {
                    let Some(idx_entry) = self.memo_search_index.get(ix) else {
                        continue;
                    };
                    let Some(score) = search::score_entry(idx_entry, query) else {
                        continue;
                    };
                    score
                } else {
                    0
                };

                candidates.push((
                    score,
                    order,
                    DisplayEntry {
                        id: EntryId::Snippet(ix),
                        is_snippet: true,
                        kind,
                        title: m.title.clone(),
                        text: m.text.clone(),
                        tags: m.tags.clone(),
                        source_app: None,
                        copied_at: 0,
                        usage_count: m.usage_count,
                        is_pinned: m.is_pinned,
                        image_file: None,
                        thumb_file: None,
                        image_dims: None,
                        byte_size: m.text.len() as u64,
                    },
                ));
                order += 1;
            }
        }

        // 2. 系统剪贴板历史
        if matches!(self.active_tab, ActiveTab::All | ActiveTab::Clipboard) {
            for item in self.history.items() {
                if !self.content_filter.matches(item.kind, item.is_pinned) {
                    continue;
                }
                let score = if has_query {
                    let Some(idx_entry) = self.history_search_index.get(&item.id) else {
                        continue;
                    };
                    let Some(score) = search::score_entry(idx_entry, query) else {
                        continue;
                    };
                    score
                } else {
                    0
                };

                candidates.push((
                    score,
                    order,
                    DisplayEntry {
                        id: EntryId::History(item.id.clone()),
                        is_snippet: false,
                        kind: item.kind,
                        title: None,
                        text: item.text.clone(),
                        tags: Vec::new(),
                        source_app: item.source_app.clone(),
                        copied_at: item.copied_at,
                        usage_count: item.usage_count,
                        is_pinned: item.is_pinned,
                        image_file: item.image_file.clone(),
                        thumb_file: item.thumb_file.clone(),
                        image_dims: item.image_width.zip(item.image_height),
                        byte_size: item.byte_size,
                    },
                ));
                order += 1;
            }
        }

        if has_query {
            candidates.sort_by(|(sa, oa, a), (sb, ob, b)| {
                sb.cmp(sa)
                    .then_with(|| b.is_pinned.cmp(&a.is_pinned))
                    .then_with(|| b.usage_count.cmp(&a.usage_count))
                    .then_with(|| b.copied_at.cmp(&a.copied_at))
                    .then_with(|| oa.cmp(ob))
            });
        } else if self.active_tab == ActiveTab::All {
            candidates.sort_by(|(_, oa, a), (_, ob, b)| {
                b.is_pinned
                    .cmp(&a.is_pinned)
                    .then_with(|| a.is_snippet.cmp(&b.is_snippet))
                    .then_with(|| b.copied_at.cmp(&a.copied_at))
                    .then_with(|| b.usage_count.cmp(&a.usage_count))
                    .then_with(|| oa.cmp(ob))
            });
        }

        let entries: Vec<DisplayEntry> = candidates.into_iter().map(|(_, _, e)| e).collect();
        let sizes: Vec<Size<Pixels>> = entries
            .iter()
            .map(|entry| size(px(0.0), px(card::row_height(entry))))
            .collect();

        if entries.is_empty() {
            self.selected_index = 0;
        } else if self.selected_index >= entries.len() {
            self.selected_index = entries.len() - 1;
        }

        self.entries = entries;
        self.row_sizes = Rc::new(sizes);
    }

    pub(crate) fn focus_search(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.search.focus_handle(cx).focus(window, cx);
    }

    fn scroll_to_top(&mut self) {
        self.smooth_scroll.reset();
        self.list_scroll.set_offset(point(px(0.0), px(0.0)));
    }

    pub(crate) fn open_preview(&mut self, entry: DisplayEntry, cx: &mut Context<Self>) {
        self.overlay = Some(OverlayView::Preview(entry));
        cx.notify();
    }

    pub(crate) fn open_snippet_editor(
        &mut self,
        editing_index: Option<usize>,
        prefill_text: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let initial = editing_index.and_then(|ix| self.memos.get(ix));
        let editor = SnippetEditorState::new(editing_index, initial, prefill_text, window, cx);
        self.overlay = Some(OverlayView::SnippetEditor(editor));
        cx.notify();
    }

    pub(crate) fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let settings = SettingsState::new(&self.config.hotkey, window, cx);
        self.overlay = Some(OverlayView::Settings(settings));
        cx.notify();
    }

    pub(crate) fn save_as_snippet(
        &mut self,
        entry: &DisplayEntry,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if entry.kind == ContentKind::Image {
            return;
        }
        self.open_snippet_editor(None, Some(entry.text.clone()), window, cx);
    }

    pub(crate) fn commit_snippet_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(OverlayView::SnippetEditor(ref mut editor)) = self.overlay else {
            return;
        };

        let title = editor.title_input.read(cx).value().trim().to_owned();
        let body = editor.body_input.read(cx).value().to_string();
        let tags = memo::parse_tags_input(&editor.tags_input.read(cx).value());
        let is_pinned = editor.is_pinned;
        let editing_index = editor.editing_index;

        if body.trim().is_empty() {
            editor.error = Some("片段内容不能为空".into());
            cx.notify();
            return;
        }

        let title_opt = (!title.is_empty()).then_some(title);
        let mut next = self.memos.clone();
        if let Some(ix) = editing_index {
            memo::update_memo(&mut next, ix, title_opt, body, tags, is_pinned);
        } else {
            let is_custom = self.state.source_path != self.state.memos_path;
            memo::add_memo(&mut next, title_opt, body, tags, is_pinned, is_custom);
        }

        watcher::suppress_reload_for(Duration::from_millis(1200));
        match memo::save(&self.state.memos_path, &next) {
            Ok(()) => {
                self.memos = next;
                self.overlay = None;
                self.notice = None;
                self.rebuild_memo_search_index();
                self.rebuild_entries();
                self.focus_search(window, cx);
            }
            Err(error) => {
                if let Some(OverlayView::SnippetEditor(ref mut ed)) = self.overlay {
                    ed.error = Some(format!("保存片段失败：{error}").into());
                }
            }
        }
        cx.notify();
    }

    pub(crate) fn copy_entry_only(&mut self, entry: &DisplayEntry, cx: &mut Context<Self>) {
        if let Err(error) = self.write_entry_to_clipboard(entry, false) {
            self.notice = Some(error.into());
        } else {
            self.notice = Some("已复制到系统剪贴板".into());
        }
        cx.notify();
    }

    pub(crate) fn paste_entry(
        &mut self,
        entry: &DisplayEntry,
        plain_text: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let previous = self
            .config
            .restore_clipboard
            .then(clipboard::read_snapshot)
            .flatten();

        if let Err(error) = self.write_entry_to_clipboard(entry, plain_text) {
            self.notice = Some(error.into());
            cx.notify();
            return;
        }

        match &entry.id {
            EntryId::History(id) => {
                if let Err(error) = self.history.mark_used(id) {
                    self.notice = Some(error.into());
                    cx.notify();
                    return;
                }
            }
            EntryId::Snippet(ix) => {
                if let Err(error) = self.record_snippet_usage(*ix) {
                    self.notice = Some(format!("记录使用次数失败：{error}").into());
                    cx.notify();
                    return;
                }
                self.rebuild_memo_search_index();
            }
        }

        self.rebuild_entries();
        self.notice = None;

        let on_after_paste: Option<Box<dyn FnOnce() + Send>> = previous.map(|snapshot| {
            Box::new(move || {
                let _ = clipboard::restore_snapshot(snapshot);
            }) as Box<dyn FnOnce() + Send>
        });

        if !platform::hide_then_paste(window, on_after_paste) {
            if !platform::has_paste_permission() {
                self.notice =
                    Some("已复制到剪贴板；授予 macOS「辅助功能」权限后可一键自动粘贴".into());
            } else {
                self.notice = Some("已复制到剪贴板；未找到可粘贴的目标窗口，请手动粘贴".into());
            }
            cx.notify();
        }
    }

    fn write_entry_to_clipboard(
        &self,
        entry: &DisplayEntry,
        plain_text: bool,
    ) -> Result<(), String> {
        if entry.kind == ContentKind::Image && !plain_text {
            let file_name = entry
                .image_file
                .as_deref()
                .ok_or_else(|| "图片路径丢失".to_owned())?;
            let path = self.history.image_path(file_name);
            return clipboard::write_image_png_file(&path);
        }
        clipboard::write_text(&entry.text)
    }

    pub(crate) fn toggle_pin_entry(&mut self, entry: &DisplayEntry, cx: &mut Context<Self>) {
        match &entry.id {
            EntryId::History(id) => match self.history.toggle_pin(id) {
                Ok(Some(pinned)) => {
                    self.notice = None;
                    self.rebuild_entries();
                    if pinned {
                        self.scroll_to_top();
                    }
                }
                Ok(None) => {}
                Err(error) => self.notice = Some(error.into()),
            },
            EntryId::Snippet(ix) => {
                let Some(existing) = self.memos.get(*ix) else {
                    return;
                };
                let pinned = !existing.is_pinned;
                let mut next = self.memos.clone();
                next[*ix].is_pinned = pinned;
                memo::sort_for_display(&mut next);

                watcher::suppress_reload_for(Duration::from_millis(1200));
                match memo::save(&self.state.memos_path, &next) {
                    Ok(()) => {
                        self.memos = next;
                        self.notice = None;
                        self.rebuild_memo_search_index();
                        self.rebuild_entries();
                        if pinned {
                            self.scroll_to_top();
                        }
                    }
                    Err(error) => self.notice = Some(format!("保存失败：{error}").into()),
                }
            }
        }
        if let Some(OverlayView::Preview(ref mut preview)) = self.overlay {
            preview.is_pinned = !preview.is_pinned;
        }
        cx.notify();
    }

    pub(crate) fn delete_entry(&mut self, entry: &DisplayEntry, cx: &mut Context<Self>) {
        match &entry.id {
            EntryId::History(id) => {
                if let Err(error) = self.history.delete(id) {
                    self.notice = Some(error.into());
                } else {
                    self.notice = None;
                    self.history_search_index.remove(id);
                    self.rebuild_entries();
                }
            }
            EntryId::Snippet(ix) => {
                let mut next = self.memos.clone();
                if memo::delete_memo(&mut next, *ix).is_some() {
                    watcher::suppress_reload_for(Duration::from_millis(1200));
                    match memo::save(&self.state.memos_path, &next) {
                        Ok(()) => {
                            self.memos = next;
                            self.notice = None;
                            self.rebuild_memo_search_index();
                            self.rebuild_entries();
                        }
                        Err(error) => self.notice = Some(format!("删除失败：{error}").into()),
                    }
                }
            }
        }
        cx.notify();
    }

    fn record_snippet_usage(&mut self, index: usize) -> Result<(), memo::MemoError> {
        if self.memos.get(index).is_none() {
            return Ok(());
        }
        let mut next = self.memos.clone();
        next[index].usage_count = next[index].usage_count.saturating_add(1);
        memo::sort_for_display(&mut next);
        watcher::suppress_reload_for(Duration::from_millis(1200));
        memo::save(&self.state.memos_path, &next)?;
        self.memos = next;
        Ok(())
    }

    fn reload_memos(&mut self, cx: &mut Context<Self>) {
        match Self::load_merged(&self.state.source_path, &self.state.memos_path) {
            Ok(memos) => {
                watcher::suppress_reload_for(Duration::from_millis(1200));
                let save_error = memo::save(&self.state.memos_path, &memos).err();
                self.memos = memos;
                self.notice = save_error.map(|error| {
                    SharedString::from(format!("已重新加载，但写入本地状态失败：{error}"))
                });
                self.rebuild_memo_search_index();
                self.rebuild_entries();
            }
            Err(error) => {
                self.notice = Some(format!("重新加载失败：{error}").into());
            }
        }
        cx.notify();
    }

    fn load_merged(
        source_path: &Path,
        data_path: &Path,
    ) -> Result<Vec<memo::Memo>, memo::MemoError> {
        let mut memos = if source_path != data_path && source_path.exists() {
            let source = memo::load(source_path)?;
            let local = memo::load_or_create(data_path)?;
            memo::merge(&source, &local)
        } else {
            memo::load_or_create(data_path)?
        };
        memo::sort_for_display(&mut memos);
        Ok(memos)
    }

    fn on_list_scroll(&mut self, event: &ScrollEvent, window: &mut Window, cx: &mut Context<Self>) {
        let was_scrolling = self.smooth_scroll.is_active();
        if self
            .smooth_scroll
            .handle_wheel(&self.list_scroll, event, window)
        {
            cx.stop_propagation();
            cx.notify();
        } else if was_scrolling && !self.smooth_scroll.is_active() {
            cx.notify();
        }
    }

    fn on_capture_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if window.has_active_dialog(cx) {
            return;
        }

        let key = event.keystroke.key.as_str();
        let modifiers = event.keystroke.modifiers;
        let primary = is_primary_modifier(modifiers);

        // 覆盖层（预览 / 编辑器 / 设置）专属按键
        match &self.overlay {
            Some(OverlayView::Preview(entry)) => {
                let entry = entry.clone();
                if key == "escape" || key == "space" {
                    self.overlay = None;
                    self.focus_search(window, cx);
                    cx.notify();
                    cx.stop_propagation();
                } else if key == "enter" {
                    self.overlay = None;
                    self.paste_entry(&entry, modifiers.shift, window, cx);
                    cx.stop_propagation();
                }
                return;
            }
            Some(OverlayView::SnippetEditor(_)) => {
                if key == "escape" {
                    self.overlay = None;
                    self.focus_search(window, cx);
                    cx.notify();
                    cx.stop_propagation();
                } else if primary && key == "s" {
                    self.commit_snippet_editor(window, cx);
                    cx.stop_propagation();
                }
                return;
            }
            Some(OverlayView::Settings(_)) => {
                if key == "escape" {
                    self.overlay = None;
                    self.focus_search(window, cx);
                    cx.notify();
                    cx.stop_propagation();
                }
                return;
            }
            None => {}
        }

        // 主列表快捷键
        if key == "escape" {
            if !self.filter.is_empty() {
                self.filter.clear();
                self.search
                    .update(cx, |state, cx| state.set_value("", window, cx));
                self.rebuild_entries();
                self.scroll_to_top();
                cx.notify();
            } else if self.active_tag.is_some() || self.content_filter != ContentFilter::All {
                self.active_tag = None;
                self.content_filter = ContentFilter::All;
                self.rebuild_entries();
                self.scroll_to_top();
                cx.notify();
            } else {
                platform::hide_window(window);
            }
            cx.stop_propagation();
        } else if key == "up" {
            if !self.entries.is_empty() {
                self.selected_index = self.selected_index.saturating_sub(1);
                self.smooth_scroll
                    .ensure_item_visible(&self.list_scroll, self.selected_index);
                cx.notify();
            }
            cx.stop_propagation();
        } else if key == "down" {
            if !self.entries.is_empty() {
                self.selected_index = (self.selected_index + 1).min(self.entries.len() - 1);
                self.smooth_scroll
                    .ensure_item_visible(&self.list_scroll, self.selected_index);
                cx.notify();
            }
            cx.stop_propagation();
        } else if key == "tab" {
            self.active_tab = if modifiers.shift {
                self.active_tab.prev()
            } else {
                self.active_tab.next()
            };
            self.active_tag = None;
            self.content_filter = ContentFilter::All;
            self.rebuild_entries();
            self.scroll_to_top();
            cx.notify();
            cx.stop_propagation();
        } else if key == "enter" && !self.search.focus_handle(cx).is_focused(window) {
            if let Some(entry) = self.entries.get(self.selected_index).cloned() {
                self.paste_entry(&entry, modifiers.shift, window, cx);
            }
            cx.stop_propagation();
        } else if ((key == "space" && self.filter.is_empty())
            || (primary && key == "y")
            || (modifiers.alt && key == "space"))
            && let Some(entry) = self.entries.get(self.selected_index).cloned()
        {
            self.open_preview(entry, cx);
            cx.stop_propagation();
        } else if primary && key == "f" {
            self.focus_search(window, cx);
            cx.stop_propagation();
        } else if primary && key == "n" {
            self.open_snippet_editor(None, None, window, cx);
            cx.stop_propagation();
        } else if primary && key == "," {
            self.open_settings(window, cx);
            cx.stop_propagation();
        } else if primary
            && key == "b"
            && let Some(entry) = self.entries.get(self.selected_index).cloned()
        {
            self.toggle_pin_entry(&entry, cx);
            cx.stop_propagation();
        } else if primary
            && key == "s"
            && let Some(entry) = self.entries.get(self.selected_index).cloned()
            && !entry.is_snippet
            && entry.kind != ContentKind::Image
        {
            self.save_as_snippet(&entry, window, cx);
            cx.stop_propagation();
        } else if primary
            && modifiers.shift
            && key == "c"
            && let Some(entry) = self.entries.get(self.selected_index).cloned()
        {
            self.copy_entry_only(&entry, cx);
            cx.stop_propagation();
        } else if primary
            && is_delete_key(key)
            && let Some(entry) = self.entries.get(self.selected_index).cloned()
        {
            self.delete_entry(&entry, cx);
            cx.stop_propagation();
        } else if primary && let Some(num) = parse_digit_1_to_9(key) {
            if let Some(entry) = self.entries.get(num - 1).cloned() {
                self.paste_entry(&entry, modifiers.shift, window, cx);
            }
            cx.stop_propagation();
        }
    }

    fn render_toolbar(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let border = theme.border;
        let secondary_bg = theme.secondary;
        let muted_fg = theme.muted_foreground;
        let count_text = format!("{} 条", self.entries.len());

        v_flex()
            .w_full()
            .border_b_1()
            .border_color(border)
            .bg(theme.background)
            .child(
                h_flex()
                    .w_full()
                    .px_3()
                    .pt_3()
                    .pb_2()
                    .gap_2()
                    .items_center()
                    .child(
                        div().flex_1().min_w_0().child(
                            Input::new(&self.search)
                                .cleanable(true)
                                .prefix(Icon::new(IconName::Search).small())
                                .suffix(
                                    div()
                                        .px_1p5()
                                        .py_0p5()
                                        .rounded_sm()
                                        .bg(secondary_bg)
                                        .text_xs()
                                        .text_color(muted_fg)
                                        .child(count_text),
                                ),
                        ),
                    )
                    .child(
                        Button::new("new-snippet-btn")
                            .outline()
                            .small()
                            .icon(IconName::Plus)
                            .tooltip(if cfg!(target_os = "macos") {
                                "新建常用片段 (⌘N)"
                            } else {
                                "新建常用片段 (Ctrl+N)"
                            })
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_snippet_editor(None, None, window, cx);
                            })),
                    )
                    .child(
                        Button::new("open-settings-btn")
                            .ghost()
                            .small()
                            .icon(IconName::Settings)
                            .tooltip(if cfg!(target_os = "macos") {
                                "偏好设置 (⌘,)"
                            } else {
                                "偏好设置 (Ctrl+,)"
                            })
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_settings(window, cx);
                            })),
                    ),
            )
            .child(self.render_filter_bar(cx))
            .into_any_element()
    }

    fn render_filter_bar(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut container = v_flex().w_full().px_3().pb_2().gap_1p5();

        let tabs = TabBar::new("mode-tabs")
            .segmented()
            .small()
            .selected_index(self.active_tab.index())
            .on_click(cx.listener(|this, index: &usize, _, cx| {
                let next_tab = ActiveTab::from_index(*index);
                if this.active_tab != next_tab {
                    this.active_tab = next_tab;
                    this.active_tag = None;
                    this.content_filter = ContentFilter::All;
                    this.rebuild_entries();
                    this.scroll_to_top();
                    cx.notify();
                }
            }))
            .child(Tab::new().label("全部"))
            .child(Tab::new().label("剪贴板"))
            .child(Tab::new().label("常用片段"));

        container = container.child(tabs);

        let mut pills = h_flex().w_full().gap_1().flex_wrap().items_center();
        if self.active_tab == ActiveTab::Snippets {
            let all_active = self.content_filter == ContentFilter::All && self.active_tag.is_none();
            pills = pills
                .child(filter_pill(
                    "snippet-all",
                    "全部",
                    all_active,
                    cx.listener(|this, _, _, cx| {
                        this.content_filter = ContentFilter::All;
                        this.active_tag = None;
                        this.rebuild_entries();
                        this.scroll_to_top();
                        cx.notify();
                    }),
                ))
                .child(filter_pill(
                    "snippet-pinned",
                    "置顶",
                    self.content_filter == ContentFilter::Pinned,
                    cx.listener(|this, _, _, cx| {
                        this.content_filter = ContentFilter::Pinned;
                        this.active_tag = None;
                        this.rebuild_entries();
                        this.scroll_to_top();
                        cx.notify();
                    }),
                ));

            for (ix, (tag, count)) in memo::collect_all_tags(&self.memos)
                .into_iter()
                .take(6)
                .enumerate()
            {
                let active = self
                    .active_tag
                    .as_ref()
                    .is_some_and(|t| t.eq_ignore_ascii_case(&tag));
                let tag_clone = tag.clone();
                pills = pills.child(filter_pill(
                    ("tag-pill", ix),
                    format!("#{tag} ({count})"),
                    active,
                    cx.listener(move |this, _, _, cx| {
                        this.content_filter = ContentFilter::All;
                        this.active_tag = if this
                            .active_tag
                            .as_ref()
                            .is_some_and(|t| t.eq_ignore_ascii_case(&tag_clone))
                        {
                            None
                        } else {
                            Some(tag_clone.clone())
                        };
                        this.rebuild_entries();
                        this.scroll_to_top();
                        cx.notify();
                    }),
                ));
            }
        } else {
            for (ix, filter) in ContentFilter::ALL_FILTERS.into_iter().enumerate() {
                let active = self.content_filter == filter;
                pills = pills.child(filter_pill(
                    ("kind-pill", ix),
                    filter.label(),
                    active,
                    cx.listener(move |this, _, _, cx| {
                        this.content_filter = filter;
                        this.rebuild_entries();
                        this.scroll_to_top();
                        cx.notify();
                    }),
                ));
            }
        }

        container.child(pills).into_any_element()
    }

    fn render_permission_banner(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        #[cfg(target_os = "macos")]
        {
            if platform::has_paste_permission() {
                return None;
            }
            let theme = cx.theme();
            let warning = theme.warning;
            let border = theme.border;
            let bg = theme.yellow_light;
            Some(
                h_flex()
                    .w_full()
                    .px_3()
                    .py_1p5()
                    .gap_2()
                    .items_center()
                    .border_b_1()
                    .border_color(border)
                    .bg(bg)
                    .child(Icon::new(IconName::ShieldAlert).small().text_color(warning))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_xs()
                            .child("未授予辅助功能权限：点击条目将复制，授权后可自动 ⌘V 粘贴"),
                    )
                    .child(
                        Button::new("banner-req-ax")
                            .primary()
                            .xsmall()
                            .label("立即授权")
                            .on_click(cx.listener(|_, _, _, cx| {
                                platform::request_paste_permission();
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("banner-open-ax")
                            .ghost()
                            .xsmall()
                            .label("设置")
                            .on_click(cx.listener(|_, _, _, _| {
                                platform::open_accessibility_settings();
                            })),
                    )
                    .into_any_element(),
            )
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = cx;
            None
        }
    }

    fn render_list(&self, cx: &mut Context<Self>) -> AnyElement {
        if self.entries.is_empty() {
            let (icon, title, hint) = if !self.filter.is_empty() {
                (
                    IconName::Search,
                    "没有匹配的记录",
                    "支持全文、标签、应用名及中文拼音首字母搜索",
                )
            } else if self.active_tab == ActiveTab::Snippets {
                (
                    IconName::Bookmark,
                    "还没有常用片段",
                    "点击右上角 + 新建常用片段，或在 memos.yaml 中编辑",
                )
            } else {
                (
                    IconName::ClipboardList,
                    "剪贴板历史为空",
                    "复制任意文本、链接、色值、代码或图片后会立即出现在这里",
                )
            };

            return div()
                .flex_1()
                .min_h_0()
                .w_full()
                .child(render_empty_state(icon, title, hint, cx))
                .into_any_element();
        }

        let now_secs = history::now_unix_secs();
        let row_sizes = self.row_sizes.clone();
        let scroll_height: f32 = self.entries.iter().map(card::row_height).sum();
        let scroll_size = size(px(0.0), px(scroll_height));

        let list = v_virtual_list(
            cx.entity(),
            "memo-list",
            row_sizes,
            move |this, range, _window, cx| {
                range
                    .filter_map(|row_idx| {
                        this.entries
                            .get(row_idx)
                            .map(|entry| card::render_card(this, row_idx, entry, now_secs, cx))
                    })
                    .collect::<Vec<_>>()
            },
        )
        .track_scroll(&self.list_scroll)
        .overflow_hidden()
        .flex_1()
        .min_h_0()
        .w_full();

        div()
            .id("memo-list-scroll")
            .relative()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .w_full()
            .overflow_hidden()
            .pt_2()
            .on_scroll(cx.listener(Self::on_list_scroll))
            .child(list)
            .child(Scrollbar::vertical(&self.list_scroll).scroll_size(scroll_size))
            .into_any_element()
    }

    fn render_footer(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let muted_fg = theme.muted_foreground;
        let border = theme.border;
        let danger = theme.danger;
        let success = theme.success;

        let num_hint = if cfg!(target_os = "macos") {
            "⌘1-9 直贴"
        } else {
            "Ctrl+1-9 直贴"
        };
        let hint: SharedString = match (&self.hotkey.label, &self.hotkey.error) {
            (Some(label), _) => {
                format!("{label} 唤起 · Enter 粘贴 · ⇧Enter 纯文本 · Space 预览 · {num_hint}")
                    .into()
            }
            (None, Some(error)) => format!("{error} · Enter 粘贴 · {num_hint}").into(),
            (None, None) => format!("Enter 粘贴 · ⇧Enter 纯文本 · Space 预览 · {num_hint}").into(),
        };

        v_flex()
            .w_full()
            .gap_1p5()
            .px_3()
            .py_2()
            .border_t_1()
            .border_color(border)
            .bg(theme.background)
            .children(self.notice.clone().map(|notice| {
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        Icon::new(IconName::TriangleAlert)
                            .small()
                            .text_color(danger),
                    )
                    .child(div().flex_1().text_xs().text_color(danger).child(notice))
            }))
            .child(
                h_flex()
                    .w_full()
                    .gap_1()
                    .items_center()
                    .child(
                        Button::new("toggle-monitor")
                            .ghost()
                            .xsmall()
                            .icon(
                                Icon::new(if self.paused {
                                    IconName::Play
                                } else {
                                    IconName::Pause
                                })
                                .text_color(if self.paused {
                                    muted_fg
                                } else {
                                    success
                                }),
                            )
                            .label(if self.paused {
                                "已暂停"
                            } else {
                                "监听中"
                            })
                            .tooltip(if self.paused {
                                "点击恢复剪贴板监听"
                            } else {
                                "点击暂停剪贴板监听"
                            })
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.toggle_pause(cx);
                            })),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_xs()
                            .text_color(muted_fg)
                            .child(hint),
                    )
                    .child(
                        Button::new("reload-memos")
                            .ghost()
                            .xsmall()
                            .icon(IconName::RefreshCw)
                            .tooltip("重新加载 memos.yaml")
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.reload_memos(cx);
                            })),
                    )
                    .child(
                        Button::new("open-data")
                            .ghost()
                            .xsmall()
                            .icon(IconName::FolderOpen)
                            .tooltip("打开数据目录")
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                if let Err(error) = memo::reveal(&this.state.memos_path) {
                                    this.notice = Some(format!("无法打开数据目录：{error}").into());
                                    cx.notify();
                                }
                            })),
                    )
                    .child(
                        Button::new("quit-app")
                            .ghost()
                            .xsmall()
                            .label("退出")
                            .on_click(|_, _, cx| cx.quit()),
                    ),
            )
            .into_any_element()
    }
}

impl Render for QuickPasteApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        platform::register_main_window(window);

        if self.pending_focus_search {
            self.pending_focus_search = false;
            if self.overlay.is_none() {
                self.focus_search(window, cx);
            }
        }

        if let Some(pending) = self.pending_overlay.take() {
            match pending {
                PendingOverlay::NewSnippet => self.open_snippet_editor(None, None, window, cx),
                PendingOverlay::Settings => self.open_settings(window, cx),
            }
        }

        // 清除按钮等程序化改值不会发 Change 事件，这里按实际内容兜底同步过滤词。
        let query = self.search.read(cx).value().to_string();
        if query != self.filter {
            self.filter = query;
            self.rebuild_entries();
            self.scroll_to_top();
        }
        self.smooth_scroll.advance(&self.list_scroll, window);

        let now_secs = history::now_unix_secs();

        let body: AnyElement = match &self.overlay {
            Some(OverlayView::Preview(entry)) => render_preview(self, entry, now_secs, cx),
            Some(OverlayView::SnippetEditor(editor)) => render_editor(editor, cx),
            Some(OverlayView::Settings(settings)) => render_settings(self, settings, cx),
            None => v_flex()
                .size_full()
                .child(self.render_toolbar(cx))
                .children(self.render_permission_banner(cx))
                .child(self.render_list(cx))
                .child(self.render_footer(cx))
                .into_any_element(),
        };

        v_flex()
            .id("quick-paste-root")
            .size_full()
            .bg(cx.theme().muted)
            .text_color(cx.theme().foreground)
            .capture_key_down(cx.listener(Self::on_capture_key_down))
            .child(body)
    }
}

fn filter_pill(
    id: impl Into<gpui_kit::ElementId>,
    label: impl Into<SharedString>,
    active: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut gpui_kit::App) + 'static,
) -> AnyElement {
    let mut btn = Button::new(id)
        .xsmall()
        .label(label.into())
        .on_click(on_click);
    btn = if active { btn.primary() } else { btn.ghost() };
    btn.into_any_element()
}

fn is_primary_modifier(modifiers: Modifiers) -> bool {
    if cfg!(target_os = "macos") {
        modifiers.platform && !modifiers.control && !modifiers.alt
    } else {
        modifiers.control && !modifiers.platform && !modifiers.alt
    }
}

fn is_delete_key(key: &str) -> bool {
    if cfg!(target_os = "macos") {
        key == "backspace" || key == "delete"
    } else {
        key == "delete"
    }
}

fn parse_digit_1_to_9(key: &str) -> Option<usize> {
    match key {
        "1" => Some(1),
        "2" => Some(2),
        "3" => Some(3),
        "4" => Some(4),
        "5" => Some(5),
        "6" => Some(6),
        "7" => Some(7),
        "8" => Some(8),
        "9" => Some(9),
        _ => None,
    }
}

fn render_empty_state(
    icon: IconName,
    title: &'static str,
    hint: &'static str,
    cx: &mut Context<QuickPasteApp>,
) -> AnyElement {
    let muted_foreground = cx.theme().muted_foreground;

    v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .gap_2()
        .p_6()
        .child(Icon::new(icon).large().text_color(muted_foreground))
        .child(div().text_sm().font_medium().child(title))
        .child(
            div()
                .text_xs()
                .text_color(muted_foreground)
                .text_center()
                .child(hint),
        )
        .into_any_element()
}
