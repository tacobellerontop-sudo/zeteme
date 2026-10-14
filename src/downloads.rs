//! Downloads: zeteme takes them over from WebView2, which saves them to the
//! Downloads folder, and lists them on its downloads page and the tab bar's
//! downloads button. Finished ones are kept as `time<TAB>state<TAB>size<TAB>
//! path<TAB>url` lines in `downloads.txt`.

/// The downloads page.
pub const PAGE: &str = include_str!("downloads.html");

use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use crate::settings::js_string;

/// Finished downloads kept in the list.
const KEPT: usize = 100;

#[derive(Clone, Debug, PartialEq)]
pub enum State {
    Running,
    Paused,
    Done,
    Canceled,
    /// Why it stopped, and whether it can carry on.
    Failed(String, bool),
}

impl State {
    pub fn finished(&self) -> bool {
        matches!(self, State::Done | State::Canceled | State::Failed(..))
    }

    fn as_str(&self) -> &str {
        match self {
            State::Running => "running",
            State::Paused => "paused",
            State::Done => "done",
            State::Canceled => "canceled",
            State::Failed(..) => "failed",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Download {
    pub id: u64,
    /// Seconds since 1970.
    pub time: u64,
    pub url: String,
    pub path: PathBuf,
    pub received: i64,
    /// -1 while the size is unknown.
    pub total: i64,
    pub state: State,
}

impl Download {
    pub fn name(&self) -> String {
        self.path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.url.clone())
    }
}

/// What WebView2 says about a download.
pub enum Event {
    Started {
        url: String,
        path: PathBuf,
        total: i64,
    },
    Progress {
        received: i64,
        total: i64,
    },
    State {
        state: State,
        path: PathBuf,
    },
}

/// Downloads, newest last.
pub struct Downloads {
    file: PathBuf,
    pub list: Vec<Download>,
    /// Whether one finished since the downloads page was last looked at.
    pub unseen: bool,
}

/// A new download's id, unique while zeteme runs.
pub fn new_id() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

fn clean(text: &str) -> String {
    text.replace(['\t', '\r', '\n'], " ")
}

impl Downloads {
    pub fn load(file: &Path) -> Downloads {
        let mut downloads = Downloads {
            file: file.to_path_buf(),
            list: Vec::new(),
            unseen: false,
        };
        for line in fs::read_to_string(file).unwrap_or_default().lines() {
            let parts: Vec<&str> = line.splitn(5, '\t').collect();
            let [time, state, size, path, url] = parts[..] else {
                continue;
            };
            let state = match state {
                "done" => State::Done,
                "canceled" => State::Canceled,
                _ => State::Failed(String::new(), false),
            };
            let size = size.parse().unwrap_or(-1);
            let id = new_id();
            downloads.list.push(Download {
                id,
                time: time.parse().unwrap_or(0),
                url: url.to_string(),
                path: PathBuf::from(path),
                received: size,
                total: size,
                state,
            });
        }
        downloads
    }

    fn save(&self) {
        let text: String = self
            .list
            .iter()
            .filter(|d| d.state.finished())
            .map(|d| {
                format!(
                    "{}\t{}\t{}\t{}\t{}\n",
                    d.time,
                    d.state.as_str(),
                    d.received,
                    clean(&d.path.to_string_lossy()),
                    clean(&d.url)
                )
            })
            .collect();
        let _ = fs::write(&self.file, text);
    }

    pub fn get(&self, id: u64) -> Option<&Download> {
        self.list.iter().find(|d| d.id == id)
    }

    pub fn start(&mut self, download: Download) {
        self.list.push(download);
    }

    pub fn progress(&mut self, id: u64, received: i64, total: i64) {
        if let Some(d) = self.list.iter_mut().find(|d| d.id == id) {
            d.received = received;
            d.total = total;
        }
    }

    /// Records a download's new state, and returns it if it just finished.
    pub fn set_state(&mut self, id: u64, state: State, path: Option<PathBuf>) -> Option<&Download> {
        let index = self.list.iter().position(|d| d.id == id)?;
        let d = &mut self.list[index];
        let finished = state.finished() && !d.state.finished();
        if state == State::Done && d.total >= 0 {
            d.received = d.total;
        }
        d.state = state;
        if let Some(path) = path.filter(|p| !p.as_os_str().is_empty()) {
            d.path = path;
        }
        if !finished {
            return None;
        }
        self.unseen = true;
        // Forget the oldest finished ones beyond what is kept.
        let done = self.list.iter().filter(|d| d.state.finished()).count();
        let mut extra = done.saturating_sub(KEPT);
        self.list.retain(|d| {
            let drop = extra > 0 && d.state.finished();
            extra -= drop as usize;
            !drop
        });
        self.save();
        self.list.iter().find(|d| d.id == id)
    }

    /// Takes a finished download off the list (the file stays).
    pub fn remove(&mut self, id: u64) {
        self.list.retain(|d| d.id != id || !d.state.finished());
        self.save();
    }

    pub fn clear_finished(&mut self) {
        self.list.retain(|d| !d.state.finished());
        self.save();
    }

    /// For the downloads button: how many are going, and how far along they
    /// are together (-1 when a size is unknown).
    pub fn summary(&self) -> (usize, f64) {
        let going: Vec<&Download> = self
            .list
            .iter()
            .filter(|d| matches!(d.state, State::Running | State::Paused))
            .collect();
        if going.is_empty() || going.iter().any(|d| d.total <= 0) {
            return (going.len(), -1.0);
        }
        let received: i64 = going.iter().map(|d| d.received).sum();
        let total: i64 = going.iter().map(|d| d.total).sum();
        (going.len(), received as f64 / total as f64)
    }

    /// A script that lists every download, newest first, on an open
    /// downloads page.
    pub fn script(&self) -> String {
        let rows: Vec<String> = self
            .list
            .iter()
            .rev()
            .map(|d| {
                let (reason, resumable) = match &d.state {
                    State::Failed(reason, resumable) => (reason.as_str(), *resumable),
                    State::Paused => ("", true),
                    _ => ("", false),
                };
                format!(
                    "[{}, {}, {}, {}, {}, {}, {}, {}, {}, {}]",
                    d.id,
                    js_string(&d.name()),
                    js_string(&d.path.to_string_lossy()),
                    js_string(&d.url),
                    js_string(d.state.as_str()),
                    js_string(reason),
                    resumable,
                    d.received,
                    d.total,
                    d.time
                )
            })
            .collect();
        format!(
            "window.zetemeDownloads && zetemeDownloads([{}])",
            rows.join(", ")
        )
    }
}

/// Opens a downloaded file the way Explorer would.
#[cfg(windows)]
pub fn open(path: &Path) {
    let _ = std::process::Command::new("explorer.exe").arg(path).spawn();
}

/// Shows a downloaded file selected in its folder.
#[cfg(windows)]
pub fn reveal(path: &Path) {
    use std::os::windows::process::CommandExt;
    // Explorer wants the quotes after the comma, which Rust's own quoting
    // won't produce. Windows paths can't contain quotes.
    let _ = std::process::Command::new("explorer.exe")
        .raw_arg(format!("/select,\"{}\"", path.display()))
        .spawn();
}

#[cfg(not(windows))]
pub fn open(_: &Path) {}

#[cfg(not(windows))]
pub fn reveal(_: &Path) {}

/// Opens the Downloads folder.
pub fn reveal_folder() {
    if let Some(profile) = std::env::var_os("USERPROFILE") {
        open(&PathBuf::from(profile).join("Downloads"));
    }
}

#[cfg(windows)]
pub mod webview2 {
    //! Takes downloads over from WebView2: it still saves them, but zeteme
    //! shows them instead of its pop-up, and pauses, resumes and cancels them.

    use std::{cell::RefCell, collections::HashMap, path::PathBuf};

    use webview2_com::{
        BytesReceivedChangedEventHandler, DownloadStartingEventHandler,
        Microsoft::Web::WebView2::Win32::*, StateChangedEventHandler, take_pwstr,
    };
    use windows_core::{BOOL, Interface, PWSTR};
    use wry::{WebView, WebViewExtWindows};

    use super::{Event, State};

    thread_local! {
        /// The downloads going on, by id. WebView2's objects live on the
        /// main thread, like everything that calls this module.
        static GOING: RefCell<HashMap<u64, ICoreWebView2DownloadOperation>> =
            RefCell::new(HashMap::new());
    }

    fn reason_text(reason: COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON) -> &'static str {
        match reason {
            COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_FILE_NO_SPACE => "the disk is full",
            COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_FILE_ACCESS_DENIED => "access denied",
            COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_FILE_NAME_TOO_LONG => "file name too long",
            COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_FILE_TOO_LARGE => "file too large",
            COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_FILE_MALICIOUS
            | COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_FILE_SECURITY_CHECK_FAILED => {
                "blocked by Windows security"
            }
            COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_FILE_BLOCKED_BY_POLICY => "blocked by policy",
            COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_NETWORK_FAILED
            | COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_NETWORK_TIMEOUT
            | COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_NETWORK_DISCONNECTED
            | COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_NETWORK_SERVER_DOWN => "network error",
            COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_SERVER_UNAUTHORIZED
            | COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_SERVER_FORBIDDEN => "the server refused",
            COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_SERVER_BAD_CONTENT => "not found on the server",
            COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_USER_SHUTDOWN => "zeteme closed",
            COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_DOWNLOAD_PROCESS_CRASHED => "WebView2 crashed",
            _ => "interrupted",
        }
    }

    fn state_of(op: &ICoreWebView2DownloadOperation) -> windows_core::Result<(State, PathBuf)> {
        unsafe {
            let mut state = COREWEBVIEW2_DOWNLOAD_STATE::default();
            op.State(&mut state)?;
            let mut path = PWSTR::null();
            op.ResultFilePath(&mut path)?;
            let path = PathBuf::from(take_pwstr(path));
            let state = match state {
                COREWEBVIEW2_DOWNLOAD_STATE_IN_PROGRESS => State::Running,
                COREWEBVIEW2_DOWNLOAD_STATE_COMPLETED => State::Done,
                _ => {
                    let mut reason = COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON::default();
                    op.InterruptReason(&mut reason)?;
                    let mut can_resume = BOOL::default();
                    op.CanResume(&mut can_resume)?;
                    match reason {
                        COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_USER_PAUSED => State::Paused,
                        COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_USER_CANCELED => State::Canceled,
                        reason => {
                            State::Failed(reason_text(reason).to_string(), can_resume.as_bool())
                        }
                    }
                }
            };
            Ok((state, path))
        }
    }

    fn progress_of(op: &ICoreWebView2DownloadOperation) -> windows_core::Result<(i64, i64)> {
        let (mut received, mut total) = (0, 0);
        unsafe {
            op.BytesReceived(&mut received)?;
            op.TotalBytesToReceive(&mut total)?;
        }
        // WebView2 says 0 for a size it doesn't know.
        Ok((received, if total > 0 { total } else { -1 }))
    }

    /// Sends what happens to the downloads `webview` starts to `on`.
    pub fn watch(
        webview: &WebView,
        on: impl Fn(u64, Event) + Clone + 'static,
    ) -> windows_core::Result<()> {
        let handler = DownloadStartingEventHandler::create(Box::new(move |_, args| {
            let Some(args) = args else {
                return Ok(());
            };
            let id = super::new_id();
            unsafe {
                // No WebView2 pop-up; zeteme shows it.
                args.SetHandled(true)?;
                let op = args.DownloadOperation()?;
                let mut url = PWSTR::null();
                op.Uri(&mut url)?;
                let mut path = PWSTR::null();
                args.ResultFilePath(&mut path)?;
                let (_, total) = progress_of(&op)?;
                on(
                    id,
                    Event::Started {
                        url: take_pwstr(url),
                        path: PathBuf::from(take_pwstr(path)),
                        total,
                    },
                );

                let progress = on.clone();
                op.add_BytesReceivedChanged(
                    &BytesReceivedChangedEventHandler::create(Box::new(move |op, _| {
                        if let Some(op) = op {
                            let (received, total) = progress_of(&op)?;
                            progress(id, Event::Progress { received, total });
                        }
                        Ok(())
                    })),
                    &mut 0,
                )?;
                let changed = on.clone();
                op.add_StateChanged(
                    &StateChangedEventHandler::create(Box::new(move |op, _| {
                        if let Some(op) = op {
                            let (state, path) = state_of(&op)?;
                            // One that failed may still be resumed.
                            if matches!(
                                state,
                                State::Done | State::Canceled | State::Failed(_, false)
                            ) {
                                GOING.with(|going| going.borrow_mut().remove(&id));
                            }
                            changed(id, Event::State { state, path });
                        }
                        Ok(())
                    })),
                    &mut 0,
                )?;
                GOING.with(|going| going.borrow_mut().insert(id, op));
            }
            Ok(())
        }));
        unsafe {
            webview
                .webview()
                .cast::<ICoreWebView2_4>()?
                .add_DownloadStarting(&handler, &mut 0)
        }
    }

    fn with(id: u64, action: impl FnOnce(&ICoreWebView2DownloadOperation)) {
        GOING.with(|going| {
            if let Some(op) = going.borrow().get(&id) {
                action(op);
            }
        });
    }

    pub fn cancel(id: u64) {
        with(id, |op| unsafe {
            let _ = op.Cancel();
        });
    }

    pub fn pause(id: u64) {
        with(id, |op| unsafe {
            let _ = op.Pause();
        });
    }

    pub fn resume(id: u64) {
        with(id, |op| unsafe {
            let _ = op.Resume();
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn download(id: u64, state: State) -> Download {
        Download {
            id,
            time: 1,
            url: "https://example.com/a.zip".into(),
            path: PathBuf::from("Downloads/a.zip"),
            received: 50,
            total: 100,
            state,
        }
    }

    #[test]
    fn keeps_finished_downloads() {
        let file =
            std::env::temp_dir().join(format!("zeteme-downloads-{}.txt", std::process::id()));
        let mut downloads = Downloads::load(&file);
        let id = new_id();
        downloads.start(download(id, State::Running));
        let running = new_id();
        downloads.start(download(running, State::Running));
        assert_eq!(downloads.summary(), (2, 0.5));

        downloads.progress(id, 80, 100);
        let done = downloads.set_state(id, State::Done, None).cloned();
        assert_eq!(done.map(|d| d.received), Some(100));
        assert!(downloads.unseen);
        // Only a change to a finished state counts.
        assert!(downloads.set_state(id, State::Done, None).is_none());

        let loaded = Downloads::load(&file);
        assert_eq!(loaded.list.len(), 1);
        assert_eq!(loaded.list[0].state, State::Done);
        assert_eq!(loaded.list[0].name(), "a.zip");

        downloads.remove(running); // still going: stays
        assert_eq!(downloads.list.len(), 2);
        downloads.clear_finished();
        assert_eq!(downloads.list.len(), 1);
        assert!(Downloads::load(&file).list.is_empty());
        let _ = fs::remove_file(&file);
    }

    #[test]
    fn sums_up_progress() {
        let file = std::env::temp_dir().join("zeteme-downloads-none.txt");
        let mut downloads = Downloads::load(&file);
        assert_eq!(downloads.summary(), (0, -1.0));
        let mut unknown = download(1, State::Running);
        unknown.total = -1;
        downloads.start(unknown);
        assert_eq!(downloads.summary(), (1, -1.0));
    }

    #[test]
    fn lists_downloads() {
        let file = std::env::temp_dir().join("zeteme-downloads-none.txt");
        let mut downloads = Downloads::load(&file);
        downloads.start(download(7, State::Failed("network error".into(), true)));
        assert_eq!(
            downloads.script(),
            r#"window.zetemeDownloads && zetemeDownloads([[7, "a.zip", "Downloads/a.zip", "https://example.com/a.zip", "failed", "network error", true, 50, 100, 1]])"#
        );
    }
}
