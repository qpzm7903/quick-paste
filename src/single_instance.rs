//! 文件锁实现的单实例，以及再次启动时唤回已有窗口的通知。

use std::{
    fs::{self, File, OpenOptions, TryLockError},
    io,
    path::{Path, PathBuf},
    thread,
    time::Duration,
};

pub struct SingleInstance {
    _lock: File,
    activation_path: PathBuf,
}

impl SingleInstance {
    /// 取得单实例锁；已有实例时写入唤回标记并返回 `Ok(None)`。
    pub fn acquire(runtime_directory: &Path) -> io::Result<Option<Self>> {
        fs::create_dir_all(runtime_directory)?;

        // 锁文件不能删除，否则两个进程可能分别锁住不同的文件。
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(runtime_directory.join("quick-paste.lock"))?;

        match lock.try_lock() {
            Ok(()) => Ok(Some(Self {
                _lock: lock,
                activation_path: runtime_directory.join("activate"),
            })),
            Err(TryLockError::WouldBlock) => {
                OpenOptions::new()
                    .write(true)
                    .create(true)
                    .truncate(true)
                    .open(runtime_directory.join("activate"))?;
                Ok(None)
            }
            Err(TryLockError::Error(error)) => Err(error),
        }
    }

    /// 监听其他实例写入的唤回标记。
    pub fn activation_receiver(&self) -> async_channel::Receiver<()> {
        let (sender, receiver) = async_channel::unbounded();
        let path = self.activation_path.clone();

        thread::Builder::new()
            .name("quick-paste-activation".to_owned())
            .spawn(move || {
                loop {
                    if path.exists() {
                        let _ = fs::remove_file(&path);
                        if sender.try_send(()).is_err() {
                            return;
                        }
                    }
                    thread::sleep(Duration::from_millis(250));
                }
            })
            .ok();

        receiver
    }
}
