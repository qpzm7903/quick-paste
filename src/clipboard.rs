//! 跨平台剪贴板读写与后台变更监听（串行访问、防自粘贴回环、隐私排除）。

use std::{
    borrow::Cow,
    path::Path,
    sync::{
        Arc, Mutex, RwLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use arboard::{Clipboard, ImageData};
use async_channel::Receiver;

use crate::{config::AppConfig, history::sha256_tagged, platform};

static CLIPBOARD_LOCK: Mutex<()> = Mutex::new(());
static SUPPRESS_UNTIL_MS: AtomicU64 = AtomicU64::new(0);
static LAST_INTERNAL_HASH: Mutex<Option<String>> = Mutex::new(None);

#[derive(Debug, Clone)]
pub enum ClipboardSnapshot {
    Text(String),
    Image {
        width: usize,
        height: usize,
        rgba: Vec<u8>,
    },
}

#[derive(Debug, Clone)]
pub enum ClipboardEvent {
    Text {
        text: String,
        source_app: Option<String>,
        source_app_id: Option<String>,
    },
    Image {
        width: u32,
        height: u32,
        rgba: Vec<u8>,
        source_app: Option<String>,
        source_app_id: Option<String>,
    },
}

pub fn suppress_monitor_for(duration: Duration) {
    let until = now_millis().saturating_add(duration.as_millis() as u64);
    SUPPRESS_UNTIL_MS.store(until, Ordering::Relaxed);
}

fn mark_internal_hash(hash: String) {
    suppress_monitor_for(Duration::from_millis(1200));
    if let Ok(mut guard) = LAST_INTERNAL_HASH.lock() {
        *guard = Some(hash);
    }
}

fn with_clipboard<T>(
    mut action: impl FnMut(&mut Clipboard) -> Result<T, arboard::Error>,
) -> Result<T, String> {
    let _guard = CLIPBOARD_LOCK
        .lock()
        .map_err(|_| "剪贴板互斥锁已损坏".to_owned())?;

    let mut last_error = None;
    for attempt in 0..3 {
        if attempt > 0 {
            thread::sleep(Duration::from_millis(15));
        }
        match Clipboard::new().and_then(|mut cb| action(&mut cb)) {
            Ok(value) => return Ok(value),
            Err(error) => last_error = Some(error),
        }
    }

    Err(last_error
        .map(|err| err.to_string())
        .unwrap_or_else(|| "访问系统剪贴板失败".to_owned()))
}

pub fn read_snapshot() -> Option<ClipboardSnapshot> {
    with_clipboard(|cb| {
        if let Ok(text) = cb.get_text()
            && !text.is_empty()
        {
            return Ok(Some(ClipboardSnapshot::Text(text)));
        }
        if let Ok(image) = cb.get_image()
            && image.width > 0
            && image.height > 0
            && !image.bytes.is_empty()
        {
            return Ok(Some(ClipboardSnapshot::Image {
                width: image.width,
                height: image.height,
                rgba: image.bytes.into_owned(),
            }));
        }
        Ok(None)
    })
    .ok()
    .flatten()
}

pub fn write_text(text: &str) -> Result<(), String> {
    let hash = sha256_tagged(b"text:", text.as_bytes());
    mark_internal_hash(hash);
    with_clipboard(|cb| cb.set_text(text.to_owned()))
}

pub fn write_image_png_file(path: &Path) -> Result<(), String> {
    let decoded = image::open(path).map_err(|e| format!("读取图片文件失败：{e}"))?;
    let rgba_img = decoded.to_rgba8();
    let width = rgba_img.width() as usize;
    let height = rgba_img.height() as usize;
    let bytes = rgba_img.into_raw();
    let tag = format!("rgba:{width}x{height}:");
    mark_internal_hash(sha256_tagged(tag.as_bytes(), &bytes));

    with_clipboard(move |cb| {
        cb.set_image(ImageData {
            width,
            height,
            bytes: Cow::Borrowed(&bytes),
        })
    })
}

pub fn restore_snapshot(snapshot: ClipboardSnapshot) -> Result<(), String> {
    match snapshot {
        ClipboardSnapshot::Text(text) => write_text(&text),
        ClipboardSnapshot::Image {
            width,
            height,
            rgba,
        } => {
            let tag = format!("rgba:{width}x{height}:");
            mark_internal_hash(sha256_tagged(tag.as_bytes(), &rgba));
            with_clipboard(move |cb| {
                cb.set_image(ImageData {
                    width,
                    height,
                    bytes: Cow::Borrowed(&rgba),
                })
            })
        }
    }
}

pub fn start_clipboard_monitor(
    config: Arc<RwLock<AppConfig>>,
    paused: Arc<AtomicBool>,
) -> Receiver<ClipboardEvent> {
    let (sender, receiver) = async_channel::unbounded();

    thread::Builder::new()
        .name("quick-paste-clipboard".to_owned())
        .spawn(move || {
            let mut last_token = platform::clipboard_change_token();
            let mut last_captured_hash: Option<String> = None;

            loop {
                thread::sleep(Duration::from_millis(250));

                let current_token = platform::clipboard_change_token();
                if current_token == last_token && current_token != 0 {
                    continue;
                }
                if current_token != 0 {
                    last_token = current_token;
                }

                if paused.load(Ordering::Relaxed) {
                    continue;
                }
                if now_millis() < SUPPRESS_UNTIL_MS.load(Ordering::Relaxed) {
                    continue;
                }

                let cfg = config.read().map(|guard| guard.clone()).unwrap_or_default();
                if !cfg.record_clipboard {
                    continue;
                }

                if platform::clipboard_has_concealed_marker() {
                    continue;
                }

                let foreground = platform::current_target_app();
                if let Some(ref app) = foreground
                    && cfg.is_app_excluded(app.name.as_deref(), app.id.as_deref())
                {
                    continue;
                }

                let Some(snapshot) = read_snapshot() else {
                    continue;
                };

                let event = match snapshot {
                    ClipboardSnapshot::Text(text) => {
                        if text.trim().is_empty() {
                            continue;
                        }
                        let hash = sha256_tagged(b"text:", text.as_bytes());
                        if is_suppressed_or_same(&hash, last_captured_hash.as_deref()) {
                            continue;
                        }
                        last_captured_hash = Some(hash);
                        ClipboardEvent::Text {
                            text,
                            source_app: foreground.as_ref().and_then(|a| a.name.clone()),
                            source_app_id: foreground.as_ref().and_then(|a| a.id.clone()),
                        }
                    }
                    ClipboardSnapshot::Image {
                        width,
                        height,
                        rgba,
                    } => {
                        if !cfg.record_images {
                            continue;
                        }
                        let max_raw = cfg.max_image_mb.saturating_mul(8 * 1024 * 1024);
                        if rgba.len() > max_raw {
                            continue;
                        }
                        let tag = format!("rgba:{width}x{height}:");
                        let hash = sha256_tagged(tag.as_bytes(), &rgba);
                        if is_suppressed_or_same(&hash, last_captured_hash.as_deref()) {
                            continue;
                        }
                        last_captured_hash = Some(hash);
                        ClipboardEvent::Image {
                            width: width as u32,
                            height: height as u32,
                            rgba,
                            source_app: foreground.as_ref().and_then(|a| a.name.clone()),
                            source_app_id: foreground.as_ref().and_then(|a| a.id.clone()),
                        }
                    }
                };

                if sender.try_send(event).is_err() && sender.is_closed() {
                    return;
                }
            }
        })
        .ok();

    receiver
}

fn is_suppressed_or_same(hash: &str, last_captured: Option<&str>) -> bool {
    if last_captured == Some(hash) {
        return true;
    }
    if let Ok(mut guard) = LAST_INTERNAL_HASH.lock()
        && guard.as_deref() == Some(hash)
    {
        *guard = None;
        return true;
    }
    false
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
