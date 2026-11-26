//! One meeting's diagnostic bundle: a `.zip` with everything a developer needs to see what happened in it.
//!
//! ```text
//! manifest.json                 version, build, commit, system, device, memory, storage, models, what is inside
//! meeting/meeting.json          the meeting as kept, secrets taken out
//! meeting/settings.json         how it was set up: people, language, where sound came from, the model
//! transcript/transcript.jsonl   each line: when, who, what, how long hearing and drawing it took
//! transcript/transcript.txt     the same to read
//! board/page-N/steps.jsonl      every change of board N: when, after which line, the line's operations, the change
//! board/page-N/graph.json       board N as it is now, its hand annotations and their anchors with it
//! board/page-N/annotations.json the hand annotations alone
//! board/page-N/final.png|.svg   board N as it looks
//! board/consolidations.jsonl    each rewrite of the board: before, after, how long
//! events/events.jsonl           the events of the meeting's time, filtered to it ([`crate::event`])
//! logs/app.log                  the app log of the meeting's time
//! config/…                      every settings file, secrets masked ([`crate::redact`])
//! audio/audio-….opus            the meeting's sound, when it was kept and is asked for
//! ```
//!
//! A meeting's time runs from when it began (or its first line) to its last change, or to now while it is open, a
//! minute either side; "the last N minutes" keeps only the end of it. Events put down to another meeting are left out.

use crate::device::ModelFile;
use crate::redact;
use anyhow::{Context, Result};
use chrono::{Local, TimeZone};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

/// What goes in, as the export sheet chooses it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Options {
    /// The meeting's sound, when it was kept.
    pub audio: bool,
    /// What was asked of the model and what it said, word for word. Without it a request keeps its size and times.
    pub prompts: bool,
    /// Only the meeting's last so many minutes, for sending what just went wrong.
    pub last_minutes: Option<u32>,
}

impl Default for Options {
    fn default() -> Self {
        Self { audio: true, prompts: true, last_minutes: None }
    }
}

/// The choices of "only the last N minutes", `None` for all of it.
pub const LAST_MINUTES: [Option<u32>; 4] = [Some(5), Some(15), Some(30), None];

/// A board as it looks, drawn by the caller (crates/export).
#[derive(Clone, Debug, Default)]
pub struct Board {
    pub png: Option<Vec<u8>>,
    pub svg: Option<String>,
}

/// Everything a bundle is made from.
#[derive(Clone, Debug, Default)]
pub struct Input {
    /// The meeting as it would be saved now (`Live::snapshot`).
    pub meeting: Value,
    /// The folder with its pictures and sound.
    pub meeting_dir: Option<PathBuf>,
    /// How the meeting is set up.
    pub settings: Value,
    /// The logs folder ([`crate::logs_dir`]).
    pub logs: PathBuf,
    /// Settings files: their name in the bundle, and where they are.
    pub configs: Vec<(String, PathBuf)>,
    /// Secrets known to the app (the API key, the proxy's password): never written anywhere in the bundle.
    pub secrets: Vec<String>,
    /// Each board drawn, in order.
    pub boards: Vec<Board>,
    /// The app and the device ([`crate::device::describe`]).
    pub environment: Value,
    pub models: Vec<ModelFile>,
    pub now_ms: u64,
    /// The meeting is open now, so its time runs to now.
    pub open_now: bool,
}

/// How far either side of a meeting's time its events and log are taken.
const MARGIN_MS: u64 = 60_000;
/// Fields of an event left out unless prompts are asked for: what was said to the model and by it.
const PRIVATE: [&str; 5] = ["prompt", "response", "thinking", "outline", "error_reply"];
/// The largest settings file carried.
const CONFIG_LIMIT: u64 = 1 << 20;

enum Body {
    Text(String),
    Bytes(Vec<u8>),
    File(PathBuf),
}

struct Entry {
    path: String,
    body: Body,
}

impl Entry {
    fn text(path: impl Into<String>, text: String) -> Self {
        Self { path: path.into(), body: Body::Text(text) }
    }

    fn size(&self) -> u64 {
        match &self.body {
            Body::Text(t) => t.len() as u64,
            Body::Bytes(b) => b.len() as u64,
            Body::File(p) => p.metadata().map_or(0, |m| m.len()),
        }
    }

    /// Already compressed: stored as it is.
    fn packed(&self) -> bool {
        [".opus", ".png", ".ogg", ".m4a", ".jpg"].iter().any(|x| self.path.ends_with(x))
    }
}

fn ms_at(v: &Value) -> Option<u64> {
    v.as_f64().filter(|x| *x > 0.0).map(|x| x as u64).or_else(|| v.as_u64())
}

/// The meeting's time, in Unix milliseconds: see the module's notes.
pub fn range(input: &Input, options: &Options) -> (u64, u64) {
    let m = &input.meeting;
    let lines = m["lines"].as_array().map(Vec::as_slice).unwrap_or_default();
    let line_times: Vec<u64> = lines.iter().filter_map(|l| ms_at(&l["at"])).collect();
    let step_times: Vec<u64> = all_steps(m).iter().flatten().filter_map(|s| ms_at(&s["t"])).collect();
    let start = [ms_at(&m["createdAt"]), line_times.iter().min().copied()].into_iter().flatten().min().unwrap_or(input.now_ms);
    let end = if input.open_now {
        input.now_ms
    } else {
        [ms_at(&m["updatedAt"]), line_times.iter().max().copied(), step_times.iter().max().copied()].into_iter().flatten().max().unwrap_or(input.now_ms)
    };
    match options.last_minutes {
        Some(n) => (end.saturating_sub(u64::from(n) * 60_000).max(start.saturating_sub(MARGIN_MS)), end + MARGIN_MS),
        None => (start.saturating_sub(MARGIN_MS), end + MARGIN_MS),
    }
}

/// Each board's record of changes, as the meeting keeps them.
fn all_steps(m: &Value) -> Vec<Vec<Value>> {
    match m["pages"].as_array().filter(|p| !p.is_empty()) {
        Some(pages) => pages.iter().map(|p| p["steps"].as_array().cloned().unwrap_or_default()).collect(),
        None => vec![m["steps"].as_array().cloned().unwrap_or_default()],
    }
}

/// Each board as it is now.
fn all_graphs(m: &Value) -> Vec<Value> {
    match m["pages"].as_array().filter(|p| !p.is_empty()) {
        Some(pages) => pages.iter().enumerate().map(|(i, p)| if Some(i as u64) == m["page"].as_u64() && !m["graph"].is_null() { m["graph"].clone() } else { p["graph"].clone() }).collect(),
        None => vec![m["graph"].clone()],
    }
}

fn iso(ms: u64) -> String {
    Local.timestamp_millis_opt(ms as i64).single().map_or_else(String::new, |t| t.to_rfc3339_opts(chrono::SecondsFormat::Millis, false))
}

/// The day a log file is for, local time, of a moment.
fn day(ms: u64) -> Option<chrono::NaiveDate> {
    Local.timestamp_millis_opt(ms as i64).single().map(|t| t.date_naive())
}

/// When an event happened, read from its start without reading the rest.
fn event_time(line: &str) -> Option<u64> {
    let digits = line.strip_prefix("{\"t\":")?;
    let end = digits.find(|c: char| !c.is_ascii_digit())?;
    digits[..end].parse().ok()
}

/// When an app log line was written, from its start; `None` for a line that goes on from the one before.
fn log_time(line: &str) -> Option<u64> {
    let stamp = line.split(' ').next()?;
    chrono::DateTime::parse_from_rfc3339(stamp).ok().map(|t| t.timestamp_millis() as u64)
}

/// The log files of `prefix` for the days of `from..to`, a day either side for clocks that moved.
fn files_for(logs: &Path, prefix: &str, from: u64, to: u64) -> Vec<PathBuf> {
    let (Some(first), Some(last)) = (day(from), day(to)) else { return vec![] };
    let (first, last) = (first - chrono::Duration::days(1), last + chrono::Duration::days(1));
    crate::writer::log_files(logs).into_iter().filter(|(d, p, _)| *d >= first && *d <= last && p.file_name().is_some_and(|n| n.to_string_lossy().starts_with(prefix))).map(|(_, p, _)| p).collect()
}

/// The events of the meeting's time, as the bundle keeps them: another meeting's left out, secrets masked, and what
/// was said to the model only when `prompts`.
fn events(input: &Input, id: &str, from: u64, to: u64, prompts: bool) -> Vec<Value> {
    let mut out = vec![];
    for path in files_for(&input.logs, "events-", from, to) {
        let Ok(file) = std::fs::File::open(&path) else { continue };
        for line in BufReader::new(file).lines().map_while(std::result::Result::ok) {
            if !event_time(&line).is_some_and(|t| (from..=to).contains(&t)) {
                continue;
            }
            let Ok(mut event) = serde_json::from_str::<Value>(&line) else { continue };
            if event["meeting"].as_str().is_some_and(|m| m != id) {
                continue;
            }
            if !prompts {
                if let Some(map) = event.as_object_mut() {
                    let before = map.len();
                    map.retain(|k, _| !PRIVATE.contains(&k.as_str()));
                    if map.len() != before {
                        map.insert("text_left_out".into(), json!(true));
                    }
                }
            }
            redact::json(&mut event);
            out.push(event);
        }
    }
    out.sort_by_key(|e| e["t"].as_u64().unwrap_or(0));
    out
}

/// The app log of the meeting's time.
fn log(input: &Input, from: u64, to: u64) -> String {
    let mut out = String::new();
    for path in files_for(&input.logs, "app-", from, to) {
        let Ok(file) = std::fs::File::open(&path) else { continue };
        let mut inside = false;
        for line in BufReader::new(file).lines().map_while(std::result::Result::ok) {
            if let Some(t) = log_time(&line) {
                inside = (from..=to).contains(&t);
            }
            if inside {
                out.push_str(&line);
                out.push('\n');
            }
        }
    }
    out
}

/// The transcript, a line of JSON each and as text, of the lines said within `from..to`.
fn transcript(m: &Value, from: u64, to: u64) -> (String, String) {
    let (mut jsonl, mut text) = (String::new(), String::new());
    for (i, line) in m["lines"].as_array().map(Vec::as_slice).unwrap_or_default().iter().enumerate() {
        let at = ms_at(&line["at"]);
        if at.is_some_and(|t| !(from..=to).contains(&t)) {
            continue;
        }
        let row = json!({
            "index": i,
            "at": at,
            "when": at.map(iso),
            "speaker": line["speaker"],
            "text": line["text"],
            "page": line["page"],
            "asr_ms": line["heard"]["asrMs"],
            "model_ms": line["ms"],
            "ops": line["ops"],
            "failed": line["failed"],
            "also": line["also"],
            "manual": line["manual"],
        });
        jsonl.push_str(&row.to_string());
        jsonl.push('\n');
        let who = line["speaker"].as_str().unwrap_or("?");
        let when = at.map(iso).unwrap_or_default();
        text.push_str(&format!("[{i}] {when} {who}: {}\n", line["text"].as_str().unwrap_or_default()));
    }
    (jsonl, text)
}

/// A board's changes, each with the line it followed and the operations drawn for that line.
fn steps_jsonl(m: &Value, steps: &[Value]) -> String {
    let lines = m["lines"].as_array().map(Vec::as_slice).unwrap_or_default();
    let mut out = String::new();
    for (i, step) in steps.iter().enumerate() {
        let after = step["line"].as_u64().map(|n| n as usize);
        let said = after.and_then(|n| n.checked_sub(1)).and_then(|n| lines.get(n));
        let row = json!({
            "step": i,
            "t": step["t"],
            "when": ms_at(&step["t"]).map(iso),
            "after_line": after.and_then(|n| n.checked_sub(1)),
            "line_text": said.map(|l| l["text"].clone()),
            "ops": said.map(|l| l["ops"].clone()),
            "d": step["d"],
        });
        out.push_str(&row.to_string());
        out.push('\n');
    }
    out
}

/// Everything that goes in, before the manifest.
fn entries(input: &Input, options: &Options) -> Vec<Entry> {
    let m = &input.meeting;
    let id = m["id"].as_str().unwrap_or_default();
    let (from, to) = range(input, options);
    let mut out = vec![];
    let mut meeting = m.clone();
    redact::json(&mut meeting);
    out.push(Entry::text("meeting/meeting.json", serde_json::to_string_pretty(&meeting).unwrap_or_default()));
    let mut settings = input.settings.clone();
    redact::json(&mut settings);
    out.push(Entry::text("meeting/settings.json", serde_json::to_string_pretty(&settings).unwrap_or_default()));
    let (jsonl, text) = transcript(m, from, to);
    out.push(Entry::text("transcript/transcript.jsonl", jsonl));
    out.push(Entry::text("transcript/transcript.txt", text));
    let graphs = all_graphs(m);
    for (i, steps) in all_steps(m).iter().enumerate() {
        let n = i + 1;
        out.push(Entry::text(format!("board/page-{n}/steps.jsonl"), steps_jsonl(m, steps)));
        let graph = graphs.get(i).cloned().unwrap_or(Value::Null);
        out.push(Entry::text(format!("board/page-{n}/annotations.json"), serde_json::to_string_pretty(&graph["annotations"]).unwrap_or_default()));
        out.push(Entry::text(format!("board/page-{n}/graph.json"), serde_json::to_string_pretty(&graph).unwrap_or_default()));
        if let Some(board) = input.boards.get(i) {
            if let Some(png) = &board.png {
                out.push(Entry { path: format!("board/page-{n}/final.png"), body: Body::Bytes(png.clone()) });
            }
            if let Some(svg) = &board.svg {
                out.push(Entry::text(format!("board/page-{n}/final.svg"), svg.clone()));
            }
        }
    }
    let events = events(input, id, from, to, options.prompts);
    let mut consolidations = String::new();
    let mut kept = String::new();
    for mut event in events {
        if event["kind"] == "board.consolidate" {
            consolidations.push_str(&event.to_string());
            consolidations.push('\n');
            // The boards themselves are in their own file; the timeline keeps how big they were.
            for side in ["before", "after"] {
                let nodes = event[side]["nodes"].as_array().map(Vec::len);
                if let Some(map) = event.as_object_mut() {
                    if map.remove(side).is_some() {
                        map.insert(format!("{side}_nodes"), json!(nodes));
                    }
                }
            }
        }
        kept.push_str(&event.to_string());
        kept.push('\n');
    }
    out.push(Entry::text("board/consolidations.jsonl", consolidations));
    out.push(Entry::text("events/events.jsonl", kept));
    out.push(Entry::text("logs/app.log", log(input, from, to)));
    for (name, path) in &input.configs {
        if path.metadata().map_or(true, |m| !m.is_file() || m.len() > CONFIG_LIMIT) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(path) else { continue };
        out.push(Entry::text(format!("config/{name}"), redact::config_file(name, &text, &input.secrets)));
    }
    if options.audio {
        if let Some(dir) = &input.meeting_dir {
            for a in m["audio"].as_array().map(Vec::as_slice).unwrap_or_default() {
                let (Some(file), Some(start)) = (a["file"].as_str(), ms_at(&a["start"])) else { continue };
                let end = start + ms_at(&a["ms"]).unwrap_or(0);
                // Only plain file names: nothing from outside the meeting's folder.
                if file.contains(['/', '\\']) || file.starts_with('.') || end < from || start > to {
                    continue;
                }
                let path = dir.join(file);
                if path.is_file() {
                    out.push(Entry { path: format!("audio/{file}"), body: Body::File(path) });
                }
            }
        }
    }
    // Nothing the app knows to be secret leaves in a text.
    for entry in &mut out {
        if let Body::Text(text) = &mut entry.body {
            *text = redact::scrub(text, &input.secrets);
        }
    }
    out
}

fn manifest(input: &Input, options: &Options, entries: &[Entry]) -> String {
    let m = &input.meeting;
    let (from, to) = range(input, options);
    let mut manifest = json!({
        "format": 1,
        "made": iso(input.now_ms),
        "environment": input.environment,
        "meeting": {
            "id": m["id"],
            "title": m["title"],
            "created": ms_at(&m["createdAt"]).map(iso),
            "updated": ms_at(&m["updatedAt"]).map(iso),
            "lines": m["lines"].as_array().map_or(0, Vec::len),
            "boards": all_steps(m).len(),
            "open_now": input.open_now,
        },
        "range": { "from": from, "to": to, "from_iso": iso(from), "to_iso": iso(to) },
        "options": { "audio": options.audio, "prompts": options.prompts, "last_minutes": options.last_minutes },
        "models": input.models.iter().map(ModelFile::json).collect::<Vec<_>>(),
        "contents": entries.iter().map(|e| json!({ "path": e.path, "bytes": e.size() })).collect::<Vec<_>>(),
    });
    redact::json(&mut manifest);
    redact::scrub(&serde_json::to_string_pretty(&manifest).unwrap_or_default(), &input.secrets)
}

/// About how big the bundle will be, in bytes: text packs to about a fifth.
pub fn estimate(input: &Input, options: &Options) -> u64 {
    entries(input, options).iter().map(|e| if matches!(e.body, Body::Text(_)) { e.size() / 5 } else { e.size() }).sum::<u64>() + 4096
}

/// What was written.
#[derive(Debug)]
pub struct Written {
    pub bytes: u64,
    pub files: usize,
}

/// Write the bundle to `out`, beside it first and moved into place, so a failure leaves nothing half written.
pub fn write(input: &Input, options: &Options, out: &Path) -> Result<Written> {
    let entries = entries(input, options);
    let manifest = manifest(input, options, &entries);
    let name = out.file_name().context("the path has no file name")?.to_string_lossy();
    let part = out.with_file_name(format!(".{name}.part"));
    let written = (|| -> Result<()> {
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&part).with_context(|| format!("writing {}", part.display()))?);
        let deflated = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated).large_file(true);
        let stored = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored).large_file(true);
        zip.start_file("manifest.json", deflated)?;
        zip.write_all(manifest.as_bytes())?;
        for entry in &entries {
            zip.start_file(entry.path.as_str(), if entry.packed() { stored } else { deflated })?;
            match &entry.body {
                Body::Text(text) => zip.write_all(text.as_bytes())?,
                Body::Bytes(bytes) => zip.write_all(bytes)?,
                Body::File(path) => {
                    std::io::copy(&mut std::fs::File::open(path).with_context(|| format!("reading {}", path.display()))?, &mut zip)?;
                }
            }
        }
        zip.finish()?.sync_all()?;
        Ok(())
    })();
    if let Err(e) = written {
        let _ = std::fs::remove_file(&part);
        return Err(e);
    }
    std::fs::rename(&part, out).inspect_err(|_| {
        let _ = std::fs::remove_file(&part);
    })?;
    Ok(Written { bytes: out.metadata().map_or(0, |m| m.len()), files: entries.len() + 1 })
}

/// `alcyo-diag-<meeting title>-<yyyymmdd-hhmm>.zip`, the title kept to what any file system takes.
pub fn file_name(title: &str, when: chrono::DateTime<Local>) -> String {
    let mut safe: String = title.chars().map(|c| if c.is_control() || "/\\:*?\"<>|".contains(c) { '-' } else { c }).collect::<String>().trim().trim_matches('.').to_string();
    if safe.chars().count() > 60 {
        safe = safe.chars().take(60).collect::<String>().trim_end().to_string();
    }
    if safe.is_empty() {
        safe = "meeting".into();
    }
    format!("alcyo-diag-{safe}-{}.zip", when.format("%Y%m%d-%H%M"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    const KEY: &str = "sk-live-0123456789abcdefSECRET";
    const PASSWORD: &str = "pr0xy-pa55word!";

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("alcyo-bundle-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A meeting of 20 minutes from `t0`, a line a minute, with an event and a log line a minute, another meeting's
    /// event between, a settings file and a kept recording, and the secrets scattered where a careless writer would
    /// copy them from.
    fn sample(dir: &Path) -> (Input, u64) {
        let t0: u64 = Local.with_ymd_and_hms(2026, 11, 26, 10, 0, 0).unwrap().timestamp_millis() as u64;
        let minute = 60_000;
        let logs = dir.join("logs");
        std::fs::create_dir_all(&logs).unwrap();
        let stamp = day(t0).unwrap().format("%Y%m%d").to_string();
        let mut events = String::new();
        let mut app = String::new();
        for i in 0..20 {
            let t = t0 + i * minute;
            events += &crate::record("model.request", json!({ "model": "local", "ms": 900 + i, "prompt": format!("prompt {i} with {KEY}"), "response": "N 「x」" }), Some("m-1"), t, i as f64 * 60.0);
            events.push('\n');
            events += &crate::record("model.request", json!({ "model": "other" }), Some("m-2"), t + 1, 0.0);
            events.push('\n');
            app += &format!("{} line {i}\n  goes on {i}\n", iso(t));
        }
        events += &crate::record("board.consolidate", json!({ "before": { "nodes": [1, 2, 3] }, "after": { "nodes": [1] }, "ms": 5000 }), Some("m-1"), t0 + 19 * minute + 5, 0.0);
        events.push('\n');
        // Long before: another day's events stay out.
        events += &crate::record("app.start", json!({}), None, t0 - 3 * 3_600_000, 0.0);
        events.push('\n');
        std::fs::write(logs.join(format!("events-{stamp}.jsonl")), events).unwrap();
        std::fs::write(logs.join(format!("app-{stamp}.log")), app + &format!("{} the key was {KEY}\n", iso(t0 + 2 * minute))).unwrap();
        let config = dir.join("remote.json");
        std::fs::write(&config, json!({ "baseUrl": "https://api.example", "apiKey": KEY, "proxyPassword": PASSWORD, "proxyUrl": format!("http://me:{PASSWORD}@proxy:8080"), "model": "m" }).to_string()).unwrap();
        let meeting_dir = dir.join("m-1");
        std::fs::create_dir_all(&meeting_dir).unwrap();
        std::fs::write(meeting_dir.join("audio-1.opus"), b"OggS fake sound").unwrap();
        let lines: Vec<Value> = (0..20).map(|i| json!({ "text": format!("line {i}"), "speaker": if i % 2 == 0 { "A" } else { "B" }, "at": t0 + i * minute, "ms": 800, "ops": format!("N 「n{i}」"), "heard": { "asrMs": 120 } })).collect();
        let steps: Vec<Value> = (0..20).map(|i| json!({ "t": t0 + i * minute + 900, "line": i + 1, "d": { "nodes": { "set": [{ "id": format!("n{i}"), "label": format!("n{i}") }] } } })).collect();
        let meeting = json!({
            "id": "m-1", "title": "Weekly / sync", "createdAt": t0, "updatedAt": t0 + 19 * minute + 1000,
            "lines": lines, "steps": steps, "page": 0,
            "pages": [{ "graph": { "nodes": [], "annotations": [{ "tool": "pen", "points": [[1.0, 2.0]], "anchor": "n1" }] }, "steps": steps }],
            "audio": [{ "file": "audio-1.opus", "start": t0, "ms": 20 * minute }, { "file": "../escape.opus", "start": t0, "ms": 1 }],
            "remote": { "apiKey": KEY },
        });
        let input = Input {
            meeting,
            meeting_dir: Some(meeting_dir),
            settings: json!({ "participants": 2, "language": "zh-CN", "listen": "microphone", "proxy": format!("http://me:{PASSWORD}@proxy:8080") }),
            logs,
            configs: vec![("remote.json".into(), config)],
            secrets: vec![KEY.into(), PASSWORD.into()],
            boards: vec![Board { png: Some(b"\x89PNG fake".to_vec()), svg: Some("<svg/>".into()) }],
            environment: json!({ "app": { "version": "0.34.1" } }),
            models: vec![ModelFile { name: "a.gguf".into(), bytes: 3, sha256: "abc".into() }],
            now_ms: t0 + 3 * 3_600_000,
            open_now: false,
        };
        (input, t0)
    }

    fn read_all(path: &Path) -> Vec<(String, Vec<u8>)> {
        let mut zip = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
        (0..zip.len())
            .map(|i| {
                let mut file = zip.by_index(i).unwrap();
                let name = file.name().unwrap().to_string();
                let mut bytes = vec![];
                file.read_to_end(&mut bytes).unwrap();
                (name, bytes)
            })
            .collect()
    }

    fn text_of<'a>(files: &'a [(String, Vec<u8>)], name: &str) -> &'a str {
        std::str::from_utf8(&files.iter().find(|(n, _)| n == name).unwrap_or_else(|| panic!("no {name}")).1).unwrap()
    }

    #[test]
    fn a_bundle_holds_the_whole_meeting() {
        let dir = scratch("whole");
        let (input, _) = sample(&dir);
        let out = dir.join("out.zip");
        let written = write(&input, &Options::default(), &out).unwrap();
        let files = read_all(&out);
        let names: Vec<&str> = files.iter().map(|(n, _)| n.as_str()).collect();
        for name in ["manifest.json", "meeting/meeting.json", "meeting/settings.json", "transcript/transcript.jsonl", "transcript/transcript.txt", "board/page-1/steps.jsonl", "board/page-1/graph.json", "board/page-1/annotations.json", "board/page-1/final.png", "board/page-1/final.svg", "board/consolidations.jsonl", "events/events.jsonl", "logs/app.log", "config/remote.json", "audio/audio-1.opus"] {
            assert!(names.contains(&name), "{name} missing from {names:?}");
        }
        assert_eq!(written.files, files.len());
        assert!(!names.iter().any(|n| n.contains("escape")), "a sound file named outside the meeting's folder stays out");
        let manifest: Value = serde_json::from_str(text_of(&files, "manifest.json")).unwrap();
        assert_eq!(manifest["meeting"]["id"], "m-1");
        assert_eq!(manifest["models"][0]["sha256"], "abc");
        assert_eq!(manifest["environment"]["app"]["version"], "0.34.1");
        let events = text_of(&files, "events/events.jsonl");
        assert_eq!(events.lines().count(), 21, "this meeting's 20 requests and its consolidation; another meeting's and another time's left out");
        assert!(events.contains("\"before_nodes\":3") && !events.contains("\"before\":"), "the timeline keeps the size of a consolidation, not the board");
        assert!(text_of(&files, "board/consolidations.jsonl").contains("\"before\":{\"nodes\":[1,2,3]}"));
        assert!(events.contains("prompt 0"), "prompts go in by default");
        assert_eq!(text_of(&files, "transcript/transcript.jsonl").lines().count(), 20);
        assert!(text_of(&files, "logs/app.log").contains("  goes on 3"), "a line that goes on stays with its first");
        let steps = text_of(&files, "board/page-1/steps.jsonl");
        let first: Value = serde_json::from_str(steps.lines().next().unwrap()).unwrap();
        assert_eq!((first["after_line"].as_u64(), first["ops"].as_str()), (Some(0), Some("N 「n0」")));
        assert!(text_of(&files, "board/page-1/annotations.json").contains("\"anchor\": \"n1\""));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The test #360 asks for: no secret the app knows of is anywhere in a bundle, whatever was asked for.
    #[test]
    fn no_secret_is_anywhere_in_a_bundle() {
        let dir = scratch("secrets");
        let (input, _) = sample(&dir);
        for options in [Options::default(), Options { audio: false, prompts: false, last_minutes: Some(5) }] {
            let out = dir.join("out.zip");
            write(&input, &options, &out).unwrap();
            let raw = std::fs::read(&out).unwrap();
            for (name, bytes) in read_all(&out).iter().chain(std::iter::once(&("(the zip itself)".to_string(), raw))) {
                for secret in &input.secrets {
                    assert!(!bytes.windows(secret.len()).any(|w| w == secret.as_bytes()), "{secret} in {name}");
                }
            }
            let files = read_all(&out);
            let config: Value = serde_json::from_str(text_of(&files, "config/remote.json")).unwrap();
            assert_eq!(config["apiKey"], "set (30 characters, sk…ET)", "whether it is set, and its ends");
            assert_eq!(config["proxyPassword"], "set (15 characters, pr…d!)");
            assert_eq!(config["model"], "m");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_last_minutes_and_privacy_are_kept_to() {
        let dir = scratch("filter");
        let (input, t0) = sample(&dir);
        let out = dir.join("out.zip");
        write(&input, &Options { audio: false, prompts: false, last_minutes: Some(5) }, &out).unwrap();
        let files = read_all(&out);
        let transcript = text_of(&files, "transcript/transcript.jsonl");
        let first: Value = serde_json::from_str(transcript.lines().next().unwrap()).unwrap();
        // The meeting ends with its last change, 19 minutes and a second in: five minutes back reaches line 15.
        assert_eq!(first["index"], 15);
        assert_eq!(transcript.lines().count(), 5);
        let events = text_of(&files, "events/events.jsonl");
        assert!(events.lines().all(|l| serde_json::from_str::<Value>(l).unwrap()["t"].as_u64().unwrap() >= t0 + 14 * 60_000));
        assert!(!events.contains("\"prompt\"") && !events.contains("\"response\"") && events.contains("\"text_left_out\":true"), "{events}");
        assert!(events.contains("\"ms\":915"), "sizes and times stay");
        assert!(!files.iter().any(|(n, _)| n.starts_with("audio/")));
        assert!(!text_of(&files, "logs/app.log").contains("line 3\n"));
        assert_eq!(text_of(&files, "board/page-1/steps.jsonl").lines().count(), 20, "the board's whole record stays, to replay it");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_open_meeting_runs_to_now() {
        let dir = scratch("open");
        let (mut input, t0) = sample(&dir);
        input.open_now = true;
        input.now_ms = t0 + 30 * 60_000;
        assert_eq!(range(&input, &Options::default()), (t0 - MARGIN_MS, t0 + 31 * 60_000));
        assert_eq!(range(&input, &Options { last_minutes: Some(15), ..Options::default() }).0, t0 + 15 * 60_000);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_size_is_estimated_near_enough() {
        let dir = scratch("estimate");
        let (input, _) = sample(&dir);
        let out = dir.join("out.zip");
        let written = write(&input, &Options::default(), &out).unwrap();
        let estimate = estimate(&input, &Options::default());
        assert!(estimate > written.bytes / 4 && estimate < written.bytes * 4, "{estimate} for {}", written.bytes);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_file_is_named_for_the_meeting_and_when() {
        let when = Local.with_ymd_and_hms(2026, 11, 26, 9, 5, 0).unwrap();
        assert_eq!(file_name("Weekly / sync: Q4?", when), "alcyo-diag-Weekly - sync- Q4--20261126-0905.zip");
        assert_eq!(file_name("  ", when), "alcyo-diag-meeting-20261126-0905.zip");
        assert_eq!(file_name("周会", when), "alcyo-diag-周会-20261126-0905.zip");
    }
}
