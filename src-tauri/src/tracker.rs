use crate::store::{now_ms, AppInfo, LiveStatus, Store, WidgetStatus};
use parking_lot::Mutex;
use std::{
    path::PathBuf,
    process,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};
use windows::{
    core::PWSTR,
    Win32::{
        Foundation::{CloseHandle, BOOL, HWND},
        System::Threading::{
            OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_FORMAT,
            PROCESS_QUERY_LIMITED_INFORMATION,
        },
        UI::{
            Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO},
            WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId},
        },
    },
};

#[derive(Default)]
struct Session {
    app: Option<AppInfo>,
    started_at: i64,
    last_checkpoint_at: i64,
    last_seen_at: i64,
    last_tick: Option<Instant>,
    pending_end_at: Option<i64>,
    paused: bool,
    manual_pause: bool,
}

pub struct Tracker {
    store: Arc<Store>,
    session: Mutex<Session>,
    idle_limit_seconds: AtomicU64,
}

impl Tracker {
    pub fn new(store: Arc<Store>) -> Arc<Self> {
        let idle_limit = store
            .settings()
            .map(|settings| settings.idle_minutes as u64 * 60)
            .unwrap_or(300);
        Arc::new(Self {
            store,
            session: Mutex::new(Session::default()),
            idle_limit_seconds: AtomicU64::new(idle_limit),
        })
    }

    pub fn start(self: &Arc<Self>) {
        let tracker = Arc::clone(self);
        thread::spawn(move || loop {
            tracker.tick();
            thread::sleep(Duration::from_secs(1));
        });
    }

    pub fn status(&self) -> LiveStatus {
        let session = self.session.lock();
        LiveStatus {
            tracking: !session.manual_pause,
            paused: session.paused,
            idle_seconds: idle_seconds(),
            current_app: session.app.clone(),
        }
    }

    pub fn widget_status(&self) -> Result<WidgetStatus, String> {
        let session = self.session.lock();
        let current_app = session.app.clone();
        let current_app_seconds = match current_app.as_ref() {
            Some(app) if !session.paused && !app.ignored => Some(
                self.store.today_app_seconds(&app.executable)?
                    + (now_ms() - session.started_at).max(0) / 1000,
            ),
            _ => None,
        };
        Ok(WidgetStatus {
            tracking: !session.manual_pause,
            paused: session.paused,
            idle_seconds: idle_seconds(),
            current_app,
            current_app_seconds,
        })
    }

    pub fn set_idle_minutes(&self, minutes: u32) {
        self.idle_limit_seconds
            .store(minutes.clamp(1, 120) as u64 * 60, Ordering::Relaxed);
    }

    pub fn set_manual_pause(&self, paused: bool) {
        let mut session = self.session.lock();
        if paused {
            self.finish_at(&mut session, now_ms());
        }
        session.manual_pause = paused;
        session.paused = paused;
    }

    pub fn flush(&self) {
        let mut session = self.session.lock();
        self.finish_at(&mut session, now_ms());
    }

    pub fn clear_usage(&self) -> Result<(), String> {
        let mut session = self.session.lock();
        self.store.clear()?;
        session.app = None;
        session.started_at = 0;
        session.last_checkpoint_at = 0;
        session.pending_end_at = None;
        session.paused = session.manual_pause;
        Ok(())
    }

    fn tick(&self) {
        let tick_at = Instant::now();
        let now = now_ms();
        let idle_limit = self.idle_limit_seconds.load(Ordering::Relaxed);
        let idle = idle_seconds();
        let mut session = self.session.lock();
        let interrupted = session
            .last_tick
            .replace(tick_at)
            .is_some_and(|last| tick_at.duration_since(last) > Duration::from_secs(5));
        if session.pending_end_at.is_some() && !self.finish_at(&mut session, now) {
            return;
        }
        if interrupted && session.app.is_some() {
            let end = (session.last_seen_at + 1_000)
                .min(now)
                .max(session.started_at);
            if !self.finish_at(&mut session, end) {
                return;
            }
            session.paused = true;
        }
        session.last_seen_at = now;
        if session.manual_pause || idle >= idle_limit {
            if !session.paused {
                let inactive_ms = idle
                    .saturating_sub(idle_limit)
                    .saturating_mul(1_000)
                    .min(i64::MAX as u64) as i64;
                let active_until = now.saturating_sub(inactive_ms);
                self.finish_at(&mut session, active_until);
                session.paused = true;
            }
            return;
        }
        let current = foreground_app()
            .filter(|(process_id, _, _, _)| *process_id != process::id())
            .and_then(|(_, executable, display_name, process_path)| {
                self.store
                    .rule_for(&executable, &display_name, &process_path)
                    .ok()
            });
        let Some(current) = current else {
            if !session.paused {
                self.finish_at(&mut session, now);
            }
            session.paused = true;
            return;
        };
        if !session.paused
            && session.app.as_ref().map(|app| app.executable.as_str())
                == Some(current.executable.as_str())
        {
            if session
                .app
                .as_ref()
                .is_some_and(|app| app.ignored != current.ignored)
            {
                if !self.finish_at(&mut session, now) {
                    return;
                }
                session.app = Some(current);
                session.started_at = now;
                session.last_checkpoint_at = now;
                return;
            }
            session.app = Some(current);
            self.checkpoint(&mut session);
            return;
        }
        if session.paused
            || session.app.as_ref().map(|app| app.executable.as_str())
                != Some(current.executable.as_str())
        {
            if !self.finish_at(&mut session, now) {
                return;
            }
            session.app = Some(current);
            session.started_at = now;
            session.last_checkpoint_at = session.started_at;
            session.paused = false;
        }
    }

    fn checkpoint(&self, session: &mut Session) {
        const CHECKPOINT_INTERVAL_MS: i64 = 30_000;
        let now = now_ms();
        if now - session.last_checkpoint_at < CHECKPOINT_INTERVAL_MS {
            return;
        }
        if let Some(app) = session.app.as_ref() {
            match self.store.append_session(app, session.started_at, now) {
                Ok(()) => {
                    session.started_at = now;
                    session.last_checkpoint_at = now;
                }
                Err(err) => eprintln!("Unable to checkpoint usage session: {err}"),
            }
        }
    }

    fn finish_at(&self, session: &mut Session, ended_at: i64) -> bool {
        let ended_at = session.pending_end_at.unwrap_or(ended_at);
        if let Some(app) = session.app.as_ref() {
            if let Err(err) = self.store.append_session(app, session.started_at, ended_at) {
                session.pending_end_at = Some(ended_at);
                eprintln!("Unable to finish usage session: {err}");
                return false;
            }
        }
        session.app = None;
        session.started_at = 0;
        session.last_checkpoint_at = 0;
        session.pending_end_at = None;
        true
    }
}

fn idle_seconds() -> u64 {
    unsafe {
        let mut input = LASTINPUTINFO {
            cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32,
            dwTime: 0,
        };
        if GetLastInputInfo(&mut input).as_bool() {
            let elapsed = (windows::Win32::System::SystemInformation::GetTickCount() as u32)
                .wrapping_sub(input.dwTime);
            return (elapsed / 1000) as u64;
        }
    }
    0
}

fn foreground_app() -> Option<(u32, String, String, String)> {
    unsafe {
        let window: HWND = GetForegroundWindow();
        if window.0.is_null() {
            return None;
        }
        let mut process_id = 0;
        GetWindowThreadProcessId(window, Some(&mut process_id));
        if process_id == 0 {
            return None;
        }
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, BOOL(0), process_id).ok()?;
        let mut buffer = [0u16; 32768];
        let mut length = buffer.len() as u32;
        let query = QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_FORMAT(0),
            PWSTR(buffer.as_mut_ptr()),
            &mut length,
        );
        let _ = CloseHandle(process);
        query.ok()?;
        let path = PathBuf::from(String::from_utf16_lossy(&buffer[..length as usize]));
        let executable = path.file_name()?.to_string_lossy().to_string();
        let display_name = path
            .file_stem()
            .unwrap_or_else(|| path.as_os_str())
            .to_string_lossy()
            .to_string();
        Some((
            process_id,
            executable,
            display_name,
            path.to_string_lossy().to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_threshold_is_five_minutes_by_default() {
        assert_eq!(5 * 60, 300);
    }

    #[test]
    fn clear_usage_discards_pending_session() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path()).unwrap();
        let tracker = Tracker::new(Arc::clone(&store));
        let now = now_ms();
        let app = AppInfo {
            executable: "test.exe".into(),
            display_name: "Test".into(),
            process_path: "C:\\test.exe".into(),
            category: "其他".into(),
            ignored: false,
        };
        store
            .append_session(&app, now - 10_000, now - 5_000)
            .unwrap();
        {
            let mut session = tracker.session.lock();
            session.app = Some(app);
            session.started_at = now - 5_000;
            session.last_checkpoint_at = now - 5_000;
            session.pending_end_at = Some(now);
        }

        tracker.clear_usage().unwrap();
        tracker.flush();

        assert_eq!(store.today_app_seconds("test.exe").unwrap(), 0);
    }
}
