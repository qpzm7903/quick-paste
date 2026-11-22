//! 监听外部对 `memos.yaml` 的修改，防抖后通知界面自动热重载。

use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use async_channel::Receiver;
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};

static IGNORE_UNTIL_MS: AtomicU64 = AtomicU64::new(0);

/// 内部写回 `memos.yaml` 前后调用，避免监听线程把自身写入误判为外部改动。
pub fn suppress_reload_for(duration: Duration) {
    let until = now_millis().saturating_add(duration.as_millis() as u64);
    IGNORE_UNTIL_MS.store(until, Ordering::Relaxed);
}

pub fn start_memos_watcher(source_path: PathBuf) -> Option<Receiver<()>> {
    let watch_dir = source_path.parent()?.to_path_buf();
    let target_name = source_path.file_name()?.to_os_string();
    let (raw_tx, raw_rx) = std::sync::mpsc::channel::<()>();
    let (ui_tx, ui_rx) = async_channel::unbounded();

    std::thread::Builder::new()
        .name("quick-paste-watcher".to_owned())
        .spawn(move || {
            let mut watcher: RecommendedWatcher =
                match notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
                    let Ok(event) = res else {
                        return;
                    };
                    if !matches!(
                        event.kind,
                        EventKind::Modify(_) | EventKind::Create(_) | EventKind::Any
                    ) {
                        return;
                    }
                    let matches_file = event
                        .paths
                        .iter()
                        .any(|path| path.file_name() == Some(target_name.as_os_str()));
                    if matches_file {
                        let _ = raw_tx.send(());
                    }
                }) {
                    Ok(watcher) => watcher,
                    Err(_) => return,
                };

            if watcher
                .watch(&watch_dir, RecursiveMode::NonRecursive)
                .is_err()
            {
                return;
            }

            while raw_rx.recv().is_ok() {
                // 合并连续的保存/原子替换事件。
                std::thread::sleep(Duration::from_millis(300));
                while raw_rx.try_recv().is_ok() {}
                if now_millis() < IGNORE_UNTIL_MS.load(Ordering::Relaxed) {
                    continue;
                }
                if ui_tx.try_send(()).is_err() && ui_tx.is_closed() {
                    break;
                }
            }
        })
        .ok()?;

    Some(ui_rx)
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
