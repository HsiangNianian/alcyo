//! Reading a bundle back, for `alcyo-tools diag`: what happened to the model and when, what took too long, what
//! failed, and how many speakers there were as the meeting went on.

use anyhow::{Context, Result};
use serde_json::Value;
use std::collections::BTreeSet;
use std::io::Read;
use std::path::Path;

/// A bundle as read back.
#[derive(Debug, Default)]
pub struct Bundle {
    pub manifest: Value,
    pub events: Vec<Value>,
    pub transcript: Vec<Value>,
    /// Each board's changes, in order (`board/page-N/steps.jsonl`).
    pub boards: Vec<Vec<Value>>,
    pub log: String,
}

fn jsonl(text: &str) -> Vec<Value> {
    text.lines().filter_map(|l| serde_json::from_str(l).ok()).collect()
}

impl Bundle {
    pub fn open(path: &Path) -> Result<Self> {
        let mut zip = zip::ZipArchive::new(std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?).context("not a zip file")?;
        let mut files = vec![];
        for i in 0..zip.len() {
            let mut file = zip.by_index(i)?;
            let name = file.name()?.to_string();
            if name.starts_with("audio/") || name.ends_with(".png") {
                continue;
            }
            let mut text = String::new();
            file.read_to_string(&mut text).with_context(|| format!("reading {name}"))?;
            files.push((name, text));
        }
        Ok(Self::from_files(files))
    }

    /// From a bundle's text files, by their names in it.
    pub fn from_files(files: impl IntoIterator<Item = (String, String)>) -> Self {
        let mut out = Self::default();
        let mut boards: Vec<(usize, Vec<Value>)> = vec![];
        for (name, text) in files {
            match name.as_str() {
                "manifest.json" => out.manifest = serde_json::from_str(&text).unwrap_or_default(),
                "events/events.jsonl" => out.events = jsonl(&text),
                "transcript/transcript.jsonl" => out.transcript = jsonl(&text),
                "logs/app.log" => out.log = text,
                other => {
                    let page = other.strip_prefix("board/page-").and_then(|r| r.strip_suffix("/steps.jsonl")).and_then(|n| n.parse::<usize>().ok());
                    if let Some(page) = page {
                        boards.push((page, jsonl(&text)));
                    }
                }
            }
        }
        boards.sort_by_key(|(page, _)| *page);
        out.boards = boards.into_iter().map(|(_, steps)| steps).collect();
        out.events.sort_by_key(|e| e["t"].as_u64().unwrap_or(0));
        out
    }

    /// When the bundle's time starts, to count from.
    pub fn origin(&self) -> u64 {
        self.manifest["range"]["from"].as_u64().or_else(|| self.events.first().and_then(|e| e["t"].as_u64())).unwrap_or(0)
    }
}

/// Something worth a line in the summary, at a time.
#[derive(Clone, Debug, PartialEq)]
pub struct Mark {
    pub t: u64,
    pub text: String,
}

/// What `alcyo-tools diag` prints.
#[derive(Debug, Default)]
pub struct Summary {
    pub lifecycle: Vec<Mark>,
    /// What took at least the stall limit: a request, a start, a stop.
    pub stalls: Vec<Mark>,
    pub errors: Vec<Mark>,
    /// Minutes since the first line, and how many speakers had been heard by then, where it changed.
    pub speakers: Vec<(u64, usize)>,
    pub requests: usize,
    pub slowest_ms: u64,
}

const LIFECYCLE: [&str; 9] = ["model.", "mem.warning", "app.lifecycle", "app.start", "app.quit", "app.panic", "speech.listening", "speech.stopped", "speech.diarization"];

fn ms(e: &Value) -> Option<u64> {
    e["ms"].as_u64().or_else(|| e["ms"].as_f64().map(|x| x as u64))
}

/// An event's fields beyond the ones every event has, in a line.
fn details(e: &Value) -> String {
    let Some(map) = e.as_object() else { return String::new() };
    map.iter()
        .filter(|(k, _)| !["t", "mono", "kind", "meeting", "prompt", "response", "thinking", "outline"].contains(&k.as_str()))
        .map(|(k, v)| match v {
            Value::String(s) => format!("{k}={}", crate::clip(s, 120)),
            Value::Number(n) if k == "avail_mem" => format!("{k}={:.2}GB", n.as_f64().unwrap_or(0.0) / 1e9),
            other => format!("{k}={}", crate::clip(&other.to_string(), 120)),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Read the bundle's timeline: `stall_ms` is how long something may take before it counts as a stall.
pub fn summarize(bundle: &Bundle, stall_ms: u64) -> Summary {
    let mut out = Summary::default();
    for e in &bundle.events {
        let t = e["t"].as_u64().unwrap_or(0);
        let kind = e["kind"].as_str().unwrap_or_default();
        let mark = |text: String| Mark { t, text };
        if kind == "model.request" {
            out.requests += 1;
            out.slowest_ms = out.slowest_ms.max(ms(e).unwrap_or(0));
        } else if LIFECYCLE.iter().any(|p| kind.starts_with(p)) {
            out.lifecycle.push(mark(format!("{kind} {}", details(e))));
        }
        if let Some(took) = ms(e).filter(|m| *m >= stall_ms) {
            if ["model.request", "model.ready", "model.loaded", "model.stopped", "model.stop_timeout", "board.consolidate"].contains(&kind) {
                let what = e["purpose"].as_str().filter(|p| !p.is_empty()).map_or_else(|| kind.to_string(), |p| format!("{kind} ({p})"));
                out.stalls.push(mark(format!("{what} took {:.1} s", took as f64 / 1000.0)));
            }
        }
        let failed = kind.contains("fail") || kind.contains("timeout") || kind == "app.panic" || e.get("error").is_some_and(|v| !v.is_null());
        if failed {
            out.errors.push(mark(format!("{kind} {}", details(e))));
        }
    }
    for line in &bundle.transcript {
        if let Some(took) = line["model_ms"].as_u64().filter(|m| *m >= stall_ms) {
            out.stalls.push(Mark { t: line["at"].as_u64().unwrap_or(0), text: format!("line {} waited {:.1} s for the board", line["index"], took as f64 / 1000.0) });
        }
    }
    out.stalls.sort_by_key(|m| m.t);
    // Lines the log calls errors, as the events may not have them all.
    let origin = bundle.origin();
    for line in bundle.log.lines() {
        let lower = line.to_lowercase();
        if ["error", "failed", "panic", "could not", "did not stop"].iter().any(|w| lower.contains(w)) {
            let t = line.split(' ').next().and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok()).map_or(origin, |d| d.timestamp_millis() as u64);
            let text = line.split_once(' ').map_or(line, |(_, rest)| rest);
            out.errors.push(Mark { t, text: format!("log: {}", crate::clip(text, 200)) });
        }
    }
    out.errors.sort_by_key(|m| m.t);
    let first = bundle.transcript.iter().filter_map(|l| l["at"].as_u64()).min().unwrap_or(0);
    let mut seen = BTreeSet::new();
    for line in &bundle.transcript {
        let (Some(at), Some(who)) = (line["at"].as_u64(), line["speaker"].as_str()) else { continue };
        if seen.insert(who.to_string()) {
            let minute = at.saturating_sub(first) / 60_000;
            match out.speakers.last_mut() {
                Some((m, n)) if *m == minute => *n = seen.len(),
                _ => out.speakers.push((minute, seen.len())),
            }
        }
    }
    out
}

/// A time as `+mm:ss` from the bundle's start, and the clock.
fn when(t: u64, origin: u64) -> String {
    let s = t.saturating_sub(origin) / 1000;
    let clock = chrono::TimeZone::timestamp_millis_opt(&chrono::Local, t as i64).single().map_or_else(String::new, |d| d.format("%H:%M:%S").to_string());
    format!("+{:02}:{:02}:{:02} {clock}", s / 3600, s / 60 % 60, s % 60)
}

/// The summary as `alcyo-tools diag` prints it.
pub fn report(bundle: &Bundle, summary: &Summary, stall_ms: u64) -> String {
    let origin = bundle.origin();
    let m = &bundle.manifest;
    let env = &m["environment"];
    let mut out = format!(
        "{} ({})\napp {} build {} commit {} llama {}\n{} {} {} memory {} free {}\nfrom {} to {}, options {}\n",
        m["meeting"]["title"].as_str().unwrap_or("?"),
        m["meeting"]["id"].as_str().unwrap_or("?"),
        env["app"]["version"].as_str().unwrap_or("?"),
        env["app"]["build"].as_str().unwrap_or("-"),
        env["app"]["commit"].as_str().unwrap_or("?"),
        env["app"]["llama_fork"].as_str().unwrap_or("?"),
        env["device"].as_str().unwrap_or("?"),
        env["os"].as_str().unwrap_or("?"),
        env["os_version"].as_str().unwrap_or(""),
        env["memory_bytes"].as_u64().map_or("?".into(), |b| format!("{:.1} GB", b as f64 / 1e9)),
        env["free_storage_bytes"].as_u64().map_or("?".into(), |b| format!("{:.1} GB", b as f64 / 1e9)),
        m["range"]["from_iso"].as_str().unwrap_or("?"),
        m["range"]["to_iso"].as_str().unwrap_or("?"),
        m["options"],
    );
    for model in m["models"].as_array().map(Vec::as_slice).unwrap_or_default() {
        out += &format!("model {} {} bytes sha256 {}\n", model["name"].as_str().unwrap_or("?"), model["bytes"], model["sha256"].as_str().unwrap_or("?"));
    }
    out += &format!("\n{} lines, {} events, {} model requests (slowest {:.1} s), {} boards\n", bundle.transcript.len(), bundle.events.len(), summary.requests, summary.slowest_ms as f64 / 1000.0, bundle.boards.len());
    let section = |out: &mut String, title: &str, marks: &[Mark]| {
        *out += &format!("\n== {title} ({})\n", marks.len());
        for mark in marks {
            *out += &format!("{}  {}\n", when(mark.t, origin), mark.text);
        }
    };
    section(&mut out, "model lifecycle", &summary.lifecycle);
    section(&mut out, &format!("stalls of {} s or more", stall_ms as f64 / 1000.0), &summary.stalls);
    section(&mut out, "errors", &summary.errors);
    out += &format!("\n== speakers over time ({} in all)\n", summary.speakers.last().map_or(0, |s| s.1));
    for (minute, n) in &summary.speakers {
        out += &format!("minute {minute:>3}: {n}\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn files() -> Vec<(String, String)> {
        let e = |t: u64, kind: &str, rest: Value| crate::record(kind, rest, Some("m"), t, 0.0);
        let events = [
            e(1_000, "model.start", json!({ "model": "q.gguf" })),
            e(9_000, "model.ready", json!({ "ms": 8000 })),
            e(10_000, "model.request", json!({ "ms": 900, "purpose": "turn" })),
            e(20_000, "model.request", json!({ "ms": 45_000, "purpose": "summary" })),
            e(30_000, "mem.warning", json!({ "listening": true })),
            e(31_000, "model.stop", json!({ "cause": "memory warning" })),
            e(62_000, "model.stop_timeout", json!({ "cause": "memory warning", "ms": 31_000, "error": "did not stop within 30 s" })),
        ]
        .join("\n");
        let transcript = [json!({ "index": 0, "at": 1_000, "speaker": "A", "model_ms": 800 }), json!({ "index": 1, "at": 70_000, "speaker": "B", "model_ms": 12_000 }), json!({ "index": 2, "at": 130_000, "speaker": "C" }), json!({ "index": 3, "at": 131_000, "speaker": "A" })].map(|v| v.to_string()).join("\n");
        vec![
            ("manifest.json".into(), json!({ "range": { "from": 0 }, "meeting": { "title": "T", "id": "m" } }).to_string()),
            ("events/events.jsonl".into(), events),
            ("transcript/transcript.jsonl".into(), transcript),
            ("board/page-2/steps.jsonl".into(), json!({ "step": 0 }).to_string()),
            ("board/page-1/steps.jsonl".into(), [json!({ "step": 0 }), json!({ "step": 1 })].map(|v| v.to_string()).join("\n")),
            ("logs/app.log".into(), "2026-11-26T10:00:05.000+08:00 could not prepare the prompt: refused\n2026-11-26T10:00:06.000+08:00 fine\n".into()),
        ]
    }

    #[test]
    fn a_bundle_reads_back_in_order() {
        let bundle = Bundle::from_files(files());
        assert_eq!(bundle.events.len(), 7);
        assert_eq!(bundle.boards.iter().map(Vec::len).collect::<Vec<_>>(), [2, 1], "boards in their order, whatever the zip's");
        assert_eq!(bundle.transcript.len(), 4);
    }

    #[test]
    fn the_timeline_finds_stalls_errors_and_speakers() {
        let bundle = Bundle::from_files(files());
        let summary = summarize(&bundle, 10_000);
        assert_eq!(summary.requests, 2);
        assert_eq!(summary.slowest_ms, 45_000);
        let kinds: Vec<&str> = summary.lifecycle.iter().map(|m| m.text.split(' ').next().unwrap()).collect();
        assert_eq!(kinds, ["model.start", "model.ready", "mem.warning", "model.stop", "model.stop_timeout"]);
        let stalls: Vec<&str> = summary.stalls.iter().map(|m| m.text.as_str()).collect();
        assert_eq!(stalls, ["model.request (summary) took 45.0 s", "model.stop_timeout took 31.0 s", "line 1 waited 12.0 s for the board"]);
        assert_eq!(summary.errors.len(), 2, "{:?}", summary.errors);
        assert!(summary.errors.iter().any(|m| m.text.starts_with("log: could not prepare")));
        assert_eq!(summary.speakers, vec![(0, 1), (1, 2), (2, 3)]);
        let text = report(&bundle, &summary, 10_000);
        assert!(text.contains("== stalls of 10 s or more (3)") && text.contains("minute   2: 3"), "{text}");
    }
}
