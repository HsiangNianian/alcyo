//! The thread that writes the app log and the events, and keeps them to [`KEEP_DAYS`] days and [`MAX_BYTES`].

use chrono::{Local, NaiveDate};
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{sync_channel, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// Days of logs kept, today included.
pub const KEEP_DAYS: i64 = 7;
/// The most the logs take together; the oldest days go first, and today's stops being written past it.
pub const MAX_BYTES: u64 = 50 << 20;
/// Lines and events waiting for the thread. Past this they are dropped, so recording never waits.
const QUEUE: usize = 8192;
/// How often what was written is put on disk.
const FLUSH_EVERY: Duration = Duration::from_secs(1);

enum Message {
    Line(String),
    Event(String),
    Flush(SyncSender<()>),
}

struct Logger {
    dir: PathBuf,
    queue: SyncSender<Message>,
}

static LOGGER: OnceLock<Logger> = OnceLock::new();

/// Where the logs are kept, under the app's data folder.
pub fn logs_dir(data: &Path) -> PathBuf {
    data.join("logs")
}

/// Where the logs are being kept, once [`init`] has been called.
pub fn current_dir() -> Option<PathBuf> {
    LOGGER.get().map(|logger| logger.dir.clone())
}

pub(crate) fn running() -> bool {
    LOGGER.get().is_some()
}

/// Start keeping the logs in `data`'s `logs` folder. Once; later calls change nothing.
pub fn init(data: &Path) {
    let dir = logs_dir(data);
    let _ = crate::started();
    LOGGER.get_or_init(|| {
        let (queue, received) = sync_channel(QUEUE);
        let at = dir.clone();
        let _ = std::thread::Builder::new().name("diag-log".into()).spawn(move || write_until_closed(&at, &received));
        let _ = std::thread::Builder::new().name("diag-memory".into()).spawn(sample_memory);
        install_panic_hook();
        Logger { dir, queue }
    });
    crate::event("app.start", serde_json::json!({ "version": env!("CARGO_PKG_VERSION"), "commit": env!("ALCYO_GIT_COMMIT") }));
}

fn send(message: Message) {
    if let Some(logger) = LOGGER.get() {
        match logger.queue.try_send(message) {
            Ok(()) | Err(TrySendError::Disconnected(_)) => {}
            // Dropped rather than wait: the window and the audio must never be held up by their log.
            Err(TrySendError::Full(_)) => {}
        }
    }
}

pub(crate) fn send_event(line: String) {
    send(Message::Event(line));
}

/// Put a line in the app log, with when it was written.
pub fn line(text: &str) {
    if !running() {
        return;
    }
    let stamp = Local::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, false);
    let mut out = String::with_capacity(text.len() + 32);
    for part in text.trim_end_matches(['\n', '\r']).split('\n') {
        out.push_str(&stamp);
        out.push(' ');
        out.push_str(part.trim_end_matches('\r'));
        out.push('\n');
    }
    send(Message::Line(out));
}

/// Put everything recorded so far on disk, waiting at most `wait`: before a bundle reads the logs, and on a panic.
pub fn flush(wait: Duration) {
    let (done, finished) = sync_channel(1);
    if LOGGER.get().is_some_and(|logger| logger.queue.try_send(Message::Flush(done)).is_ok()) {
        let _ = finished.recv_timeout(wait);
    }
}

/// A panic is written down and put on disk before the app goes, as the system would otherwise lose it with the rest.
fn install_panic_hook() {
    let before = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        line(&format!("panic: {info}"));
        crate::event("app.panic", serde_json::json!({ "message": info.to_string() }));
        flush(Duration::from_millis(500));
        before(info);
    }));
}

/// Every [`crate::SAMPLE_EVERY`] while listening, how much memory the app has left, where that is known.
fn sample_memory() {
    loop {
        std::thread::sleep(crate::SAMPLE_EVERY);
        if crate::listening() && crate::available_memory().is_some() {
            crate::event("mem.sample", serde_json::json!({}));
        }
    }
}

/// The day a log file is for, from its name: `app-20261126.log` or `events-20261126.jsonl`.
pub(crate) fn day_of(name: &str) -> Option<NaiveDate> {
    let stem = name.strip_prefix("app-").and_then(|s| s.strip_suffix(".log")).or_else(|| name.strip_prefix("events-").and_then(|s| s.strip_suffix(".jsonl")))?;
    NaiveDate::parse_from_str(stem, "%Y%m%d").ok()
}

/// The log files in `dir`, oldest day first, with their days and sizes.
pub(crate) fn log_files(dir: &Path) -> Vec<(NaiveDate, PathBuf, u64)> {
    let Ok(entries) = std::fs::read_dir(dir) else { return vec![] };
    let mut out: Vec<(NaiveDate, PathBuf, u64)> = entries
        .flatten()
        .filter_map(|e| {
            let day = day_of(&e.file_name().to_string_lossy())?;
            Some((day, e.path(), e.metadata().map_or(0, |m| m.len())))
        })
        .collect();
    out.sort();
    out
}

/// Delete what is older than `keep_days` before `today`, then the oldest days until the rest fit in `max_bytes`.
/// Today's files stay. Returns how much is left.
pub fn prune(dir: &Path, today: NaiveDate, keep_days: i64, max_bytes: u64) -> u64 {
    let oldest = today - chrono::Duration::days(keep_days - 1);
    let mut kept = vec![];
    for (day, path, bytes) in log_files(dir) {
        if day < oldest {
            let _ = std::fs::remove_file(&path);
        } else {
            kept.push((day, path, bytes));
        }
    }
    let mut total: u64 = kept.iter().map(|(_, _, b)| b).sum();
    for (day, path, bytes) in &kept {
        if total <= max_bytes || *day >= today {
            break;
        }
        if std::fs::remove_file(path).is_ok() {
            total -= bytes;
        }
    }
    total
}

/// Today's two files, open for appending.
struct Files {
    dir: PathBuf,
    day: NaiveDate,
    app: Option<(PathBuf, BufWriter<File>)>,
    events: Option<(PathBuf, BufWriter<File>)>,
    /// What the logs take, as last counted and written since.
    total: u64,
    /// Today's files are past [`MAX_BYTES`] on their own: nothing more is written until tomorrow.
    full: bool,
}

fn open(path: &Path) -> Option<BufWriter<File>> {
    let file = OpenOptions::new().create(true).append(true).open(path).ok()?;
    Some(BufWriter::with_capacity(64 << 10, file))
}

impl Files {
    fn new(dir: &Path) -> Self {
        let mut files = Self { dir: dir.to_path_buf(), day: Local::now().date_naive(), app: None, events: None, total: 0, full: false };
        files.start_day();
        files
    }

    fn start_day(&mut self) {
        self.flush();
        let _ = std::fs::create_dir_all(&self.dir);
        self.day = Local::now().date_naive();
        self.total = prune(&self.dir, self.day, KEEP_DAYS, MAX_BYTES);
        self.full = self.total > MAX_BYTES;
        let stamp = self.day.format("%Y%m%d");
        let (app, events) = (self.dir.join(format!("app-{stamp}.log")), self.dir.join(format!("events-{stamp}.jsonl")));
        self.app = open(&app).map(|w| (app, w));
        self.events = open(&events).map(|w| (events, w));
    }

    fn write(&mut self, event: bool, text: &str) {
        if Local::now().date_naive() != self.day {
            self.start_day();
        }
        if self.full {
            return;
        }
        self.total += text.len() as u64 + 1;
        if self.total > MAX_BYTES {
            self.flush();
            self.total = prune(&self.dir, self.day, KEEP_DAYS, MAX_BYTES);
            if self.total > MAX_BYTES {
                self.full = true;
                if let Some((_, w)) = self.app.as_mut() {
                    let _ = writeln!(w, "{} the logs are full for today; nothing more is written until tomorrow", Local::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, false));
                }
                return;
            }
        }
        let target = if event { self.events.as_mut() } else { self.app.as_mut() };
        if let Some((_, w)) = target {
            let _ = w.write_all(text.as_bytes());
            if event {
                let _ = w.write_all(b"\n");
            }
        }
    }

    fn flush(&mut self) {
        for (_, w) in self.app.iter_mut().chain(self.events.iter_mut()) {
            let _ = w.flush();
        }
    }

    /// Files deleted from under the writer, as Settings › Storage does, are started again rather than written into
    /// nowhere.
    fn reopen_deleted(&mut self) {
        let gone = [&self.app, &self.events].iter().any(|f| f.as_ref().is_none_or(|(path, _)| !path.exists()));
        if gone {
            self.app = None;
            self.events = None;
            self.start_day();
        }
    }
}

fn write_until_closed(dir: &Path, received: &Receiver<Message>) {
    let mut files = Files::new(dir);
    let mut flushed = Instant::now();
    loop {
        match received.recv_timeout(FLUSH_EVERY) {
            Ok(Message::Line(text)) => files.write(false, &text),
            Ok(Message::Event(text)) => files.write(true, &text),
            Ok(Message::Flush(done)) => {
                files.flush();
                files.reopen_deleted();
                flushed = Instant::now();
                let _ = done.send(());
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                files.flush();
                return;
            }
        }
        if flushed.elapsed() >= FLUSH_EVERY {
            files.flush();
            files.reopen_deleted();
            flushed = Instant::now();
        }
    }
}

/// Copy everything the app writes to its error output into the app log, as well as where it went before (a
/// terminal, Xcode's console). Unix only: elsewhere only [`line`] reaches the log.
pub fn capture_stderr() {
    #[cfg(unix)]
    unix::capture();
}

#[cfg(unix)]
mod unix {
    use std::fs::File;
    use std::io::{BufRead, BufReader, Write};
    use std::os::fd::FromRawFd;

    pub fn capture() {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| {
            let mut fds = [0; 2];
            // SAFETY: plain descriptor calls on descriptors this owns; on any failure stderr is left as it was.
            unsafe {
                if libc::pipe(fds.as_mut_ptr()) != 0 {
                    return;
                }
                let original = libc::dup(2);
                if original < 0 || libc::dup2(fds[1], 2) < 0 {
                    libc::close(fds[0]);
                    libc::close(fds[1]);
                    if original >= 0 {
                        libc::close(original);
                    }
                    return;
                }
                libc::close(fds[1]);
                let (from, mut to) = (File::from_raw_fd(fds[0]), File::from_raw_fd(original));
                let started = std::thread::Builder::new().name("diag-stderr".into()).spawn(move || {
                    let mut reader = BufReader::new(from);
                    let mut buf = Vec::with_capacity(1024);
                    loop {
                        buf.clear();
                        match reader.read_until(b'\n', &mut buf) {
                            Ok(0) | Err(_) => return,
                            Ok(_) => {
                                let _ = to.write_all(&buf);
                                super::line(&String::from_utf8_lossy(&buf));
                            }
                        }
                    }
                });
                if started.is_err() {
                    libc::dup2(original, 2);
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("alcyo-diag-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn files_are_named_by_their_day() {
        assert_eq!(day_of("app-20261126.log"), NaiveDate::from_ymd_opt(2026, 11, 26));
        assert_eq!(day_of("events-20261101.jsonl"), NaiveDate::from_ymd_opt(2026, 11, 1));
        assert_eq!(day_of("model-hashes.json"), None);
        assert_eq!(day_of("app-2026.log"), None);
    }

    /// A week is kept; past the size limit the oldest days go first, and today stays whatever it takes.
    #[test]
    fn old_days_and_the_oldest_past_the_limit_are_deleted() {
        let dir = scratch("prune");
        let today = NaiveDate::from_ymd_opt(2026, 11, 26).unwrap();
        for (day, bytes) in [(20261101, 10), (20261120, 400), (20261121, 300), (20261125, 200), (20261126, 100)] {
            std::fs::write(dir.join(format!("app-{day}.log")), vec![b'x'; bytes]).unwrap();
        }
        std::fs::write(dir.join("events-20261126.jsonl"), vec![b'x'; 50]).unwrap();
        std::fs::write(dir.join("model-hashes.json"), "{}").unwrap();
        let left = prune(&dir, today, 7, 400);
        let names: Vec<String> = log_files(&dir).into_iter().map(|(_, p, _)| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(names, ["app-20261125.log", "app-20261126.log", "events-20261126.jsonl"]);
        assert_eq!(left, 350);
        assert!(dir.join("model-hashes.json").exists(), "only log files are pruned");
        assert_eq!(prune(&dir, today, 7, 10), 150, "today's files stay even past the limit");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_deleted_file_is_started_again() {
        let dir = scratch("reopen");
        let mut files = Files::new(&dir);
        files.write(false, "one\n");
        files.flush();
        let path = files.app.as_ref().unwrap().0.clone();
        std::fs::remove_dir_all(&dir).unwrap();
        files.reopen_deleted();
        files.write(false, "two\n");
        files.flush();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "two\n");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
