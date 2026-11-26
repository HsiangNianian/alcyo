//! Diagnostics for testing on real devices (#360): what went on, kept on disk, and a meeting's diagnostic bundle.
//!
//! Two records are kept in the data folder's `logs`, a file of each a day, at most [`KEEP_DAYS`] days and
//! [`MAX_BYTES`] together ([`writer`]):
//!
//! - the app log, `app-YYYYMMDD.log`: everything the app writes to its error output (`eprintln!`), each line with
//!   when it was written ([`capture_stderr`]), and what [`line`] is given;
//! - events, `events-YYYYMMDD.jsonl`: one JSON object a line, with when on the wall clock (`t`, Unix milliseconds)
//!   and since the app started (`mono`, seconds), the meeting open at the time, and what happened ([`event`]). The
//!   model's start, stop and requests, what was heard and by whom, the board's consolidations, memory.
//!
//! Recording never waits: a full queue drops what does not fit rather than hold up the window or the audio, and
//! everything is written by a thread of its own.
//!
//! [`bundle`] puts one meeting's share of both together with the meeting, its sound, its boards, the settings (with
//! secrets taken out, [`redact`]) and what the device is ([`device`]) into one `.zip`; [`read`] reads one back for
//! `alcyo-tools diag`.

pub mod bundle;
pub mod device;
pub mod read;
pub mod redact;
mod writer;

pub use writer::{capture_stderr, current_dir, flush, init, line, logs_dir, prune, KEEP_DAYS, MAX_BYTES};

use serde_json::{json, Map, Value};
use std::cell::Cell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// The meeting open now, which every event is put down to.
static MEETING: Mutex<Option<String>> = Mutex::new(None);
/// Whether the app is listening, when memory is looked at every [`SAMPLE_EVERY`].
static LISTENING: AtomicBool = AtomicBool::new(false);
/// How much more memory the app may use, where the system says (the iPad's `os_proc_available_memory`).
static PROBE: OnceLock<fn() -> Option<u64>> = OnceLock::new();

/// How often memory is noted while listening.
pub const SAMPLE_EVERY: Duration = Duration::from_secs(30);

/// What one model request may keep of its prompt and of its reply, in characters: enough to read what was asked,
/// without a long meeting's events filling the log.
pub const KEEP_CHARS: usize = 48_000;

/// When the app started, as near as can be told: the first time anything here was asked.
pub fn started() -> Instant {
    static STARTED: OnceLock<Instant> = OnceLock::new();
    *STARTED.get_or_init(Instant::now)
}

/// Now on the wall clock, in Unix milliseconds.
pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64
}

/// Put what happens from now on down to meeting `id`, or to none.
pub fn set_meeting(id: Option<&str>) {
    let mut meeting = MEETING.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if meeting.as_deref() != id {
        *meeting = id.map(str::to_string);
        drop(meeting);
        event("app.meeting", json!({ "open": id }));
    }
}

/// The meeting events are put down to now.
pub fn meeting() -> Option<String> {
    MEETING.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
}

/// Where memory can be asked about: noted with every model event, and every [`SAMPLE_EVERY`] while listening.
pub fn set_memory_probe(probe: fn() -> Option<u64>) {
    let _ = PROBE.set(probe);
}

/// How much more memory the app may use, where that is known.
pub fn available_memory() -> Option<u64> {
    PROBE.get().and_then(|probe| probe())
}

/// The app started or stopped listening.
pub fn set_listening(on: bool) {
    if LISTENING.swap(on, Ordering::Relaxed) != on {
        event(if on { "speech.listening" } else { "speech.stopped" }, json!({}));
    }
}

pub(crate) fn listening() -> bool {
    LISTENING.load(Ordering::Relaxed)
}

thread_local! {
    static PURPOSE: Cell<&'static str> = const { Cell::new("") };
}

/// What the model requests made on this thread are for, until the guard goes: "summary", "consolidate".
pub struct Purpose(&'static str);

impl Purpose {
    pub fn set(purpose: &'static str) -> Self {
        Self(PURPOSE.with(|p| p.replace(purpose)))
    }
}

impl Drop for Purpose {
    fn drop(&mut self) {
        PURPOSE.with(|p| p.set(self.0));
    }
}

/// What this thread's requests are for, if it was said.
pub fn purpose() -> &'static str {
    PURPOSE.with(Cell::get)
}

/// One event as it is written: `t` first, so a reader can tell when it was without reading the rest.
pub fn record(kind: &str, fields: Value, meeting: Option<&str>, t: u64, mono: f64) -> String {
    let mut out = Map::new();
    out.insert("t".into(), json!(t));
    out.insert("mono".into(), json!((mono * 1000.0).round() / 1000.0));
    out.insert("kind".into(), json!(kind));
    if let Some(meeting) = meeting {
        out.insert("meeting".into(), json!(meeting));
    }
    if kind.starts_with("model.") || kind.starts_with("mem.") {
        if let Some(bytes) = available_memory() {
            out.insert("avail_mem".into(), json!(bytes));
        }
    }
    if let Value::Object(fields) = fields {
        out.extend(fields);
    }
    Value::Object(out).to_string()
}

/// Note that something happened, with what there is to know about it. Never waits; dropped if the app has not set
/// up its logs ([`init`]), as in tests and tools.
pub fn event(kind: &str, fields: Value) {
    if !writer::running() {
        return;
    }
    let meeting = meeting();
    writer::send_event(record(kind, fields, meeting.as_deref(), now_ms(), started().elapsed().as_secs_f64()));
}

/// Text cut to at most `chars` characters, saying how much was left out.
pub fn clip(text: &str, chars: usize) -> String {
    match text.char_indices().nth(chars) {
        Some((at, _)) => format!("{}… [{} more characters]", &text[..at], text[at..].chars().count()),
        None => text.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_event_starts_with_when_and_carries_its_meeting() {
        let line = record("model.start", json!({ "model": "a.gguf" }), Some("m-1"), 1_700_000_000_123, 2.5);
        assert!(line.starts_with("{\"t\":1700000000123,"), "{line}");
        let v: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v["meeting"], "m-1");
        assert_eq!(v["kind"], "model.start");
        assert_eq!(v["model"], "a.gguf");
        assert_eq!(v["mono"], 2.5);
    }

    #[test]
    fn a_purpose_lasts_as_long_as_its_guard() {
        assert_eq!(purpose(), "");
        {
            let _summary = Purpose::set("summary");
            assert_eq!(purpose(), "summary");
            {
                let _inner = Purpose::set("consolidate");
                assert_eq!(purpose(), "consolidate");
            }
            assert_eq!(purpose(), "summary");
        }
        assert_eq!(purpose(), "");
    }

    #[test]
    fn long_text_is_clipped_with_a_count() {
        assert_eq!(clip("abc", 5), "abc");
        assert_eq!(clip("一二三四五", 2), "一二… [3 more characters]");
    }
}
