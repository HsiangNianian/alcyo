//! Editing the transcript (#362): what a line keeps when it is fixed, merged or deleted, and how the rest of the
//! app reads lines that were.
//!
//! Lines are never taken out of a meeting's `lines`: the board's steps, replay, the diagnostic bundle and the
//! voiceprints heard this session all find a line by its place. A deleted line stays as `"deleted": true`, a line
//! merged into an earlier one as `"mergedInto": <its place>`; both are *gone* and every reader skips them through
//! [`shown`]. What was heard is kept under `heard` (`text`, `speaker`) the first time a line changes, and never
//! overwritten, so undo, "restore what was heard" and the diagnostic bundle have it.

use serde_json::{json, Value};

/// The meeting format that has lines edited, merged or deleted. An older app sees a newer format and opens the
/// meeting to view only, rather than showing deleted lines again as if they were said.
pub const EDITED_VERSION: u64 = 3;
/// The meeting format of a meeting with no edits, which every app since the v2 store reads and writes.
pub const PLAIN_VERSION: u64 = 2;

/// What an edit did to the transcript.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditKind {
    /// A line's words were fixed.
    Text,
    /// One line was given to someone else.
    Speaker,
    /// Lines next to each other became one.
    Merge,
    /// Lines were taken out.
    Delete,
    /// A line went back to what was heard.
    Restore,
}

impl EditKind {
    pub fn name(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Speaker => "speaker",
            Self::Merge => "merge",
            Self::Delete => "delete",
            Self::Restore => "restore",
        }
    }
}

/// Whether a line was deleted or merged into another: kept in the file, never shown or read as said.
pub fn is_gone(line: &Value) -> bool {
    line["deleted"].as_bool().unwrap_or(false) || !line["mergedInto"].is_null()
}

/// The lines a reader sees, with their places in the meeting: every line but the gone ones.
pub fn shown(lines: &[Value]) -> impl Iterator<Item = (usize, &Value)> {
    lines.iter().enumerate().filter(|(_, l)| !is_gone(l))
}

/// Whether the model has finished with a line: drawn, or failed to.
pub fn is_drawn(line: &Value) -> bool {
    !line["ms"].is_null()
}

/// Whether a line can be edited now: something said or typed (not a note about a change), not gone, and already
/// on the board, so no request still holds its words.
pub fn editable(line: &Value) -> bool {
    !line["manual"].as_bool().unwrap_or(false) && !is_gone(line) && is_drawn(line)
}

/// Whether a line was edited, merged into or restored after an edit: it carries what was heard.
pub fn is_edited(line: &Value) -> bool {
    !line["edited"].is_null()
}

/// What was heard for a line that has been edited: the words and who said them, as they came in.
pub fn heard(line: &Value) -> Option<(&str, Option<&str>)> {
    let heard = &line["heard"];
    let text = heard["text"].as_str()?;
    Some((text, heard["speaker"].as_str()))
}

/// The words a line had when it came in: what was heard if it was edited since, else its text.
pub fn heard_text(line: &Value) -> &str {
    heard(line).map_or_else(|| line["text"].as_str().unwrap_or_default(), |(text, _)| text)
}

/// Keep what was heard before a line first changes. Later changes keep the first.
pub(crate) fn keep_heard(line: &mut Value) {
    if heard(line).is_some() {
        return;
    }
    let text = line["text"].clone();
    let speaker = line["speaker"].clone();
    if !line["heard"].is_object() {
        line["heard"] = json!({});
    }
    line["heard"]["text"] = text;
    if !speaker.is_null() {
        line["heard"]["speaker"] = speaker;
    }
}

/// Note on a line that it was changed by hand, when and how.
pub(crate) fn mark_edited(line: &mut Value, kind: EditKind, at: u64) {
    let mut kinds: Vec<Value> = line["edited"]["kinds"].as_array().cloned().unwrap_or_default();
    if !kinds.iter().any(|k| k == kind.name()) {
        kinds.push(json!(kind.name()));
    }
    line["edited"] = json!({ "at": at, "kinds": kinds });
}

/// The format a meeting with these lines is written in.
pub fn format_version(lines: &[Value]) -> u64 {
    if lines.iter().any(|l| is_gone(l) || is_edited(l)) {
        EDITED_VERSION
    } else {
        PLAIN_VERSION
    }
}

/// A new line id for this meeting: `L` and a number after every one used.
pub fn next_line_id(lines: &[Value]) -> String {
    let most = lines.iter().filter_map(|l| l["id"].as_str()?.strip_prefix('L')?.parse::<u64>().ok()).max().unwrap_or(0);
    format!("L{}", most + 1)
}

/// Give every line without an id one, in order: lines kept before ids came get them when the meeting opens.
pub fn assign_ids(lines: &mut [Value]) {
    let mut next = lines.iter().filter_map(|l| l["id"].as_str()?.strip_prefix('L')?.parse::<u64>().ok()).max().unwrap_or(0);
    for line in lines.iter_mut().filter(|l| l["id"].as_str().is_none()) {
        next += 1;
        line["id"] = json!(format!("L{next}"));
    }
}

/// Whether these lines follow one another, with no shown line said between them: what a merge needs.
pub fn adjacent(lines: &[Value], picked: &[usize]) -> bool {
    let mut picked = picked.to_vec();
    picked.sort_unstable();
    picked.dedup();
    if picked.len() < 2 || picked.iter().any(|&i| lines.get(i).is_none_or(|l| !editable(l))) {
        return false;
    }
    let (first, last) = (picked[0], picked[picked.len() - 1]);
    shown(lines).filter(|(i, l)| (first..=last).contains(i) && !l["manual"].as_bool().unwrap_or(false)).count() == picked.len()
}

/// Lines' words joined into one: with a space between words of a language that spaces them, without between
/// Chinese or Japanese, and never doubled.
pub fn join_words<'a>(parts: impl IntoIterator<Item = &'a str>) -> String {
    let mut out = String::new();
    for part in parts.into_iter().map(str::trim).filter(|p| !p.is_empty()) {
        let spaced = |c: char| c.is_ascii_alphanumeric() || matches!(c, ',' | '.' | '?' | '!' | ';' | ':');
        if out.chars().last().is_some_and(spaced) && part.chars().next().is_some_and(|c| c.is_ascii_alphanumeric()) {
            out.push(' ');
        }
        out.push_str(part);
    }
    out
}

/// What the model has heard of a meeting, rebuilt from its lines after an edit: every shown line that was said
/// and has been drawn, or failed in a request (the context keeps those), with its corrected words.
pub fn history_of(lines: &[Value]) -> Vec<Value> {
    shown(lines)
        .filter(|(_, l)| !l["manual"].as_bool().unwrap_or(false) && is_drawn(l))
        .filter(|(_, l)| !l["failed"].as_bool().unwrap_or(false) || l["boardFailure"] == "request")
        .map(|(_, l)| {
            let mut said = json!({ "text": l["text"] });
            if let Some(speaker) = l["speaker"].as_str() {
                said["speaker"] = json!(speaker);
            }
            said
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(text: &str) -> Value {
        json!({ "text": text, "speaker": "A", "ms": 10 })
    }

    #[test]
    fn gone_lines_are_not_shown_but_keep_their_place() {
        let lines = vec![line("一"), json!({ "text": "二", "deleted": true }), json!({ "text": "三", "mergedInto": 0 }), line("四")];
        let places: Vec<usize> = shown(&lines).map(|(i, _)| i).collect();
        assert_eq!(places, vec![0, 3]);
        assert!(is_gone(&lines[1]) && is_gone(&lines[2]) && !is_gone(&lines[0]));
    }

    #[test]
    fn what_was_heard_is_kept_once() {
        let mut l = json!({ "text": "删掉", "speaker": "E", "heard": { "asrMs": 120 } });
        keep_heard(&mut l);
        l["text"] = json!("留着");
        l["speaker"] = json!("D");
        keep_heard(&mut l);
        assert_eq!(heard(&l), Some(("删掉", Some("E"))));
        assert_eq!(l["heard"]["asrMs"], 120, "the recognizer's timing stays beside it");
        assert_eq!(heard_text(&l), "删掉");
        assert_eq!(heard_text(&line("x")), "x");
    }

    #[test]
    fn ids_go_on_after_the_highest() {
        let mut lines = vec![json!({ "text": "a" }), json!({ "text": "b", "id": "L7" }), json!({ "text": "c" })];
        assign_ids(&mut lines);
        assert_eq!(lines[0]["id"], "L8");
        assert_eq!(lines[1]["id"], "L7");
        assert_eq!(lines[2]["id"], "L9");
        assert_eq!(next_line_id(&lines), "L10");
        assert_eq!(next_line_id(&[]), "L1");
    }

    #[test]
    fn only_lines_next_to_each_other_merge() {
        let lines = vec![line("a"), line("b"), json!({ "text": "x", "deleted": true }), line("c"), line("d"), json!({ "text": "w", "speaker": "A" })];
        assert!(adjacent(&lines, &[0, 1]));
        assert!(adjacent(&lines, &[1, 3]), "a deleted line between does not count");
        assert!(!adjacent(&lines, &[0, 3]), "b was said between");
        assert!(!adjacent(&lines, &[0]));
        assert!(!adjacent(&lines, &[4, 5]), "the last is still waiting for the board");
    }

    #[test]
    fn words_join_as_their_language_writes_them() {
        assert_eq!(join_words(["可以共同的多少", "钱", "要算清楚。"]), "可以共同的多少钱要算清楚。");
        assert_eq!(join_words(["we could share", "the cost,", "right?"]), "we could share the cost, right?");
        assert_eq!(join_words(["好的", "OK"]), "好的OK");
    }

    #[test]
    fn the_format_says_when_lines_were_edited() {
        assert_eq!(format_version(&[line("a")]), PLAIN_VERSION);
        assert_eq!(format_version(&[line("a"), json!({ "text": "b", "deleted": true })]), EDITED_VERSION);
        assert_eq!(format_version(&[json!({ "text": "b", "edited": { "at": 1 } })]), EDITED_VERSION);
    }

    #[test]
    fn history_is_the_corrected_lines_the_model_heard() {
        let lines = vec![
            json!({ "text": "fixed", "speaker": "A", "ms": 1, "heard": { "text": "fixd" } }),
            json!({ "text": "gone", "ms": 1, "deleted": true }),
            json!({ "text": "note", "manual": true, "ms": 0 }),
            json!({ "text": "waiting" }),
            json!({ "text": "asked", "ms": 0, "failed": true, "boardFailure": "request" }),
            json!({ "text": "never", "ms": 0, "failed": true, "boardFailure": "cancelled" }),
        ];
        let h = history_of(&lines);
        assert_eq!(h, vec![json!({ "text": "fixed", "speaker": "A" }), json!({ "text": "asked" })]);
    }
}
