//! 隐藏窗口后清理工作集，降低任务管理器里的常驻内存数字。

#[cfg(target_os = "windows")]
mod imp {
    use std::{
        mem::size_of,
        sync::{Mutex, OnceLock, mpsc},
        thread,
        time::{Duration, Instant, SystemTime, UNIX_EPOCH},
    };

    use windows::Win32::System::{
        ProcessStatus::{
            EmptyWorkingSet, GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
            PROCESS_MEMORY_COUNTERS_EX2,
        },
        Threading::GetCurrentProcess,
    };

    const STARTUP_TRIM_DELAY_SECS: u64 = 1;
    const MIN_HIDDEN_TRIM_DELAY_SECS: u64 = 10;
    const MAX_HIDDEN_TRIM_DELAY_SECS: u64 = 30;
    const IDLE_TIMEOUT_SECS: u64 = 3600;
    const PRIVATE_WORKING_SET_LIMIT: usize = 30 * 1024 * 1024;

    enum Command {
        Schedule,
        Cancel,
    }

    static SENDER: OnceLock<Mutex<mpsc::Sender<Command>>> = OnceLock::new();

    pub(crate) fn schedule_startup_cleanup() {
        let _ = thread::Builder::new()
            .name("quick-paste-memory-startup".to_owned())
            .spawn(|| {
                thread::sleep(Duration::from_secs(STARTUP_TRIM_DELAY_SECS));
                let _ = empty_working_set();
            });
    }

    pub(crate) fn schedule_hidden_trim() {
        send(Command::Schedule);
    }

    pub(crate) fn cancel_hidden_trim() {
        let Some(sender) = SENDER.get() else {
            return;
        };
        if let Ok(sender) = sender.lock() {
            let _ = sender.send(Command::Cancel);
        }
    }

    fn send(command: Command) {
        let sender = SENDER.get_or_init(|| {
            let (sender, receiver) = mpsc::channel();
            let _ = thread::Builder::new()
                .name("quick-paste-memory-trim".to_owned())
                .spawn(move || worker(receiver));
            Mutex::new(sender)
        });
        if let Ok(sender) = sender.lock() {
            let _ = sender.send(command);
        }
    }

    fn worker(receiver: mpsc::Receiver<Command>) {
        let mut deadline = None;
        loop {
            let timeout = deadline
                .map(|deadline: Instant| deadline.saturating_duration_since(Instant::now()))
                .unwrap_or(Duration::from_secs(IDLE_TIMEOUT_SECS));
            match receiver.recv_timeout(timeout) {
                Ok(Command::Schedule) => {
                    deadline = Some(Instant::now() + hidden_trim_delay());
                }
                Ok(Command::Cancel) => {
                    deadline = None;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if deadline.take().is_some()
                        && !crate::platform::is_main_window_visible()
                        && private_working_set_size()
                            .is_some_and(|size| size > PRIVATE_WORKING_SET_LIMIT)
                    {
                        let _ = empty_working_set();
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
    }

    fn hidden_trim_delay() -> Duration {
        let span = MAX_HIDDEN_TRIM_DELAY_SECS - MIN_HIDDEN_TRIM_DELAY_SECS + 1;
        let offset = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| u64::from(elapsed.subsec_nanos()) % span)
            .unwrap_or(0);
        Duration::from_secs(MIN_HIDDEN_TRIM_DELAY_SECS + offset)
    }

    fn private_working_set_size() -> Option<usize> {
        let size = size_of::<PROCESS_MEMORY_COUNTERS_EX2>() as u32;
        let mut counters = PROCESS_MEMORY_COUNTERS_EX2 {
            cb: size,
            ..Default::default()
        };
        // SAFETY: 伪句柄表示当前进程，缓冲区在调用期间有效，并通过 cb 传入准确大小。
        let result = unsafe {
            GetProcessMemoryInfo(
                GetCurrentProcess(),
                std::ptr::from_mut(&mut counters).cast::<PROCESS_MEMORY_COUNTERS>(),
                size,
            )
        };
        result.ok().map(|_| counters.PrivateWorkingSetSize)
    }

    fn empty_working_set() -> bool {
        // SAFETY: 伪句柄表示当前进程，EmptyWorkingSet 只会移除可回收的工作集页面。
        unsafe { EmptyWorkingSet(GetCurrentProcess()).is_ok() }
    }
}

#[cfg(not(target_os = "windows"))]
mod imp {
    pub(crate) fn schedule_startup_cleanup() {}
    #[allow(dead_code)]
    pub(crate) fn schedule_hidden_trim() {}
    #[allow(dead_code)]
    pub(crate) fn cancel_hidden_trim() {}
}

#[allow(unused_imports)]
pub(crate) use imp::{cancel_hidden_trim, schedule_hidden_trim, schedule_startup_cleanup};
