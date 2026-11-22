use std::{
    fs::{self, OpenOptions},
    io::Write as _,
    path::{Path, PathBuf},
    sync::{Arc, RwLock, atomic::AtomicBool},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use async_channel::Receiver;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState, hotkey::HotKey};
use gpui_kit::{
    App, AppContext as _, Entity, Global, SharedString, TitlebarOptions, WindowAppearance,
    WindowBounds, WindowOptions,
    component::{Theme, ThemeMode},
    px, size,
};

use crate::{
    autolaunch, clipboard,
    config::{self, AppConfig, ThemePreference},
    memo, memory_trim, platform,
    single_instance::SingleInstance,
    tray::TrayController,
    ui::QuickPasteApp,
    watcher,
};

#[derive(Clone)]
pub struct HotkeyStatus {
    pub label: Option<SharedString>,
    pub error: Option<SharedString>,
}

impl HotkeyStatus {
    pub fn ready(label: &str) -> Self {
        Self {
            label: Some(label.into()),
            error: None,
        }
    }

    pub fn failed(message: impl Into<SharedString>) -> Self {
        Self {
            label: None,
            error: Some(message.into()),
        }
    }
}

#[derive(Clone)]
pub struct SharedAppState {
    pub data_dir: PathBuf,
    pub config_path: PathBuf,
    pub source_path: PathBuf,
    pub memos_path: PathBuf,
    pub shared_config: Arc<RwLock<AppConfig>>,
    pub monitor_paused: Arc<AtomicBool>,
}

#[derive(Clone)]
struct AppPaths {
    data_dir: PathBuf,
    config: PathBuf,
    source: PathBuf,
    memos: PathBuf,
    crash_log: PathBuf,
    runtime_dir: PathBuf,
}

impl AppPaths {
    fn discover() -> Self {
        let data_dir = memo::data_directory();
        let memos = data_dir.join("memos.yaml");
        let source = memo::external_source_path()
            .filter(|path| path.as_path() != memos.as_path())
            .unwrap_or_else(|| memos.clone());
        Self {
            config: data_dir.join("config.yaml"),
            source,
            memos,
            crash_log: data_dir.join("crash.log"),
            runtime_dir: data_dir.join("runtime"),
            data_dir,
        }
    }
}

struct InstanceGuard(#[allow(dead_code)] SingleInstance);

impl Global for InstanceGuard {}

pub struct HotkeyManagerHolder {
    manager: Option<GlobalHotKeyManager>,
    active: Option<HotKey>,
    pub status: HotkeyStatus,
}

impl Global for HotkeyManagerHolder {}

impl HotkeyManagerHolder {
    pub fn rebind(&mut self, spec: &str) -> Result<HotkeyStatus, String> {
        let canonical = config::canonicalize_hotkey(spec)?;
        let hotkey = config::parse_hotkey(&canonical)?;
        let manager = self
            .manager
            .as_ref()
            .ok_or_else(|| "全局快捷键管理器未初始化".to_owned())?;

        let prev = self.active.take();
        if let Some(old) = prev {
            let _ = manager.unregister(old);
        }

        match manager.register(hotkey) {
            Ok(()) => {
                self.active = Some(hotkey);
                self.status = HotkeyStatus::ready(&config::format_hotkey_display(&canonical));
                Ok(self.status.clone())
            }
            Err(error) => {
                if let Some(old) = prev
                    && manager.register(old).is_ok()
                {
                    self.active = Some(old);
                }
                Err(format!(
                    "快捷键 {} 注册失败（可能已被占用）：{error}",
                    config::format_hotkey_display(&canonical)
                ))
            }
        }
    }
}

pub struct TrayHolder(pub TrayController);

impl Global for TrayHolder {}

pub struct MainWindowHolder {
    pub view: Entity<QuickPasteApp>,
}

impl Global for MainWindowHolder {}

pub fn apply_theme(pref: ThemePreference, appearance: WindowAppearance, cx: &mut App) {
    let mode = match pref {
        ThemePreference::System => match appearance {
            WindowAppearance::Dark | WindowAppearance::VibrantDark => ThemeMode::Dark,
            _ => ThemeMode::Light,
        },
        ThemePreference::Light => ThemeMode::Light,
        ThemePreference::Dark => ThemeMode::Dark,
    };
    Theme::change(mode, None, cx);
    cx.refresh_windows();
}

pub fn run() {
    let paths = AppPaths::discover();

    if let Err(error) = fs::create_dir_all(&paths.data_dir) {
        append_log(
            &std::env::temp_dir().join("quick-paste-startup.log"),
            &format!("无法创建数据目录 {:?}：{error}", paths.data_dir),
        );
        return;
    }
    install_panic_hook(paths.crash_log.clone());

    let instance = match SingleInstance::acquire(&paths.runtime_dir) {
        Ok(Some(instance)) => instance,
        Ok(None) => return,
        Err(error) => {
            append_log(&paths.crash_log, &format!("单实例检查失败：{error}"));
            return;
        }
    };
    let activation_receiver = instance.activation_receiver();

    let cfg = config::load_or_create(&paths.config).unwrap_or_default();
    if cfg.launch_at_login {
        let _ = autolaunch::sync_launch_at_login(true);
    }

    let shared_config = Arc::new(RwLock::new(cfg.clone()));
    let monitor_paused = Arc::new(AtomicBool::new(false));

    platform::start_foreground_tracker();
    let clipboard_receiver =
        clipboard::start_clipboard_monitor(shared_config.clone(), monitor_paused.clone());
    let watcher_receiver = watcher::start_memos_watcher(paths.source.clone());

    let state = SharedAppState {
        data_dir: paths.data_dir.clone(),
        config_path: paths.config.clone(),
        source_path: paths.source.clone(),
        memos_path: paths.memos.clone(),
        shared_config,
        monitor_paused,
    };

    let application = gpui_kit::application().with_assets(gpui_kit::assets::AllAssets);
    application.on_reopen(|cx| {
        if let Some(view) = cx
            .try_global::<MainWindowHolder>()
            .map(|holder| holder.view.clone())
        {
            view.update(cx, |view, cx| view.on_show_request(cx));
        }
    });

    application.run(move |cx| {
        gpui_kit::init(cx);
        cx.set_global(InstanceGuard(instance));

        let (hotkey_holder, hotkey_receiver) = install_hotkey(&cfg.hotkey);
        let hotkey_status = hotkey_holder.status.clone();
        cx.set_global(hotkey_holder);

        let tray_receiver = match TrayController::create() {
            Ok((tray, receiver)) => {
                cx.set_global(TrayHolder(tray));
                platform::set_accessory_activation_policy(true);
                Some(receiver)
            }
            Err(error) => {
                append_log(&paths.crash_log, &format!("托盘初始化失败：{error}"));
                None
            }
        };

        let options = window_options(cx);
        let (window_handle, view) = gpui_kit::open_window(options, cx, |window, cx| {
            apply_theme(cfg.theme, window.appearance(), cx);
            cx.new(|cx| QuickPasteApp::new(window, cx, state.clone(), hotkey_status.clone()))
        })
        .unwrap_or_else(|error| panic!("无法创建主窗口：{error}"));

        cx.set_global(MainWindowHolder { view: view.clone() });

        memory_trim::schedule_startup_cleanup();
        spawn_hotkey_task(cx, view.clone(), hotkey_receiver);
        spawn_activation_task(cx, view.clone(), activation_receiver);
        spawn_clipboard_task(cx, view.clone(), clipboard_receiver);
        if let Some(tray_rx) = tray_receiver {
            spawn_tray_task(cx, view.clone(), tray_rx);
        }
        if let Some(watcher_rx) = watcher_receiver {
            spawn_watcher_task(cx, view, watcher_rx);
        }

        platform::show_main_window(cfg.position_on_cursor_screen);
        cx.activate(true);
        let _ = window_handle.update(cx, |_, window, _| window.activate_window());
    });
}

fn window_options(cx: &mut App) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::centered(size(px(540.0), px(680.0)), cx)),
        titlebar: Some(TitlebarOptions {
            title: Some("QuickPaste · 快速粘贴".into()),
            ..Default::default()
        }),
        window_min_size: Some(size(px(460.0), px(520.0))),
        app_id: Some("com.g-ai-002.quick-paste".to_owned()),
        ..Default::default()
    }
}

fn install_hotkey(preferred: &str) -> (HotkeyManagerHolder, Receiver<()>) {
    let (sender, receiver) = async_channel::unbounded();
    GlobalHotKeyEvent::set_event_handler(Some(move |event: GlobalHotKeyEvent| {
        if event.state == HotKeyState::Pressed {
            let _ = sender.try_send(());
        }
    }));

    let manager = match GlobalHotKeyManager::new() {
        Ok(manager) => manager,
        Err(error) => {
            return (
                HotkeyManagerHolder {
                    manager: None,
                    active: None,
                    status: HotkeyStatus::failed(format!("全局快捷键初始化失败：{error}")),
                },
                receiver,
            );
        }
    };

    let mut candidates: Vec<String> = Vec::new();
    if !preferred.trim().is_empty() {
        candidates.push(preferred.trim().to_owned());
    }
    for &fallback in config::fallback_hotkeys() {
        if !candidates.iter().any(|c| c.eq_ignore_ascii_case(fallback)) {
            candidates.push(fallback.to_owned());
        }
    }

    for spec in candidates {
        if let Ok(hotkey) = config::parse_hotkey(&spec)
            && manager.register(hotkey).is_ok()
        {
            let label = config::format_hotkey_display(&spec);
            return (
                HotkeyManagerHolder {
                    manager: Some(manager),
                    active: Some(hotkey),
                    status: HotkeyStatus::ready(&label),
                },
                receiver,
            );
        }
    }

    (
        HotkeyManagerHolder {
            manager: Some(manager),
            active: None,
            status: HotkeyStatus::failed("全局快捷键注册失败，请在设置中更换组合键"),
        },
        receiver,
    )
}

fn spawn_hotkey_task(cx: &mut App, view: Entity<QuickPasteApp>, receiver: Receiver<()>) {
    cx.spawn(async move |cx| {
        let mut last = Instant::now() - Duration::from_secs(5);
        while receiver.recv().await.is_ok() {
            if last.elapsed() < Duration::from_millis(250) {
                continue;
            }
            last = Instant::now();
            view.update(cx, |view, cx| view.on_hotkey(cx));
        }
    })
    .detach();
}

fn spawn_activation_task(cx: &mut App, view: Entity<QuickPasteApp>, receiver: Receiver<()>) {
    cx.spawn(async move |cx| {
        while receiver.recv().await.is_ok() {
            view.update(cx, |view, cx| view.on_show_request(cx));
            while receiver.try_recv().is_ok() {}
        }
    })
    .detach();
}

fn spawn_clipboard_task(
    cx: &mut App,
    view: Entity<QuickPasteApp>,
    receiver: Receiver<clipboard::ClipboardEvent>,
) {
    cx.spawn(async move |cx| {
        while let Ok(event) = receiver.recv().await {
            view.update(cx, |view, cx| view.on_clipboard_event(event, cx));
        }
    })
    .detach();
}

fn spawn_tray_task(
    cx: &mut App,
    view: Entity<QuickPasteApp>,
    receiver: Receiver<crate::tray::TrayAction>,
) {
    cx.spawn(async move |cx| {
        while let Ok(action) = receiver.recv().await {
            view.update(cx, |view, cx| view.on_tray_action(action, cx));
        }
    })
    .detach();
}

fn spawn_watcher_task(cx: &mut App, view: Entity<QuickPasteApp>, receiver: Receiver<()>) {
    cx.spawn(async move |cx| {
        while receiver.recv().await.is_ok() {
            view.update(cx, |view, cx| view.on_external_memos_changed(cx));
            while receiver.try_recv().is_ok() {}
        }
    })
    .detach();
}

fn install_panic_hook(path: PathBuf) {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        append_log(&path, &format!("panic: {info}"));
        default_hook(info);
    }));
}

fn append_log(path: &Path, message: &str) {
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "[{}] {message}", unix_seconds());
    }
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}
