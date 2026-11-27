//! The meeting a window has open: the board, what was said, undo, and the replay record.
//! It keeps the file's raw JSON and overwrites only the fields it owns.

use crate::edit::{self, assign_ids, editable, history_of, is_gone, join_words, keep_heard, mark_edited, next_line_id, shown, EditKind};
use crate::store::{new_id, Loaded, Origin};
use alcyo_board::graph::{Graph, Kind};
use alcyo_board::speakers::{identify_in_slot, move_sentence, MOST_SPEAKERS, raised_by, rename_by, rename_by_in_steps, rename_voice, Voice};
use alcyo_board::steps::{diff, ink_only, replay};
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};
use std::time::{SystemTime, UNIX_EPOCH};

pub const UNTITLED: &str = "Untitled meeting";
const UNDO_LIMIT: usize = 100;

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64
}

/// The undo stacks, ported from the earlier app's edit.ts.
pub struct History<T> {
    past: Vec<T>,
    future: Vec<T>,
}

impl<T> Default for History<T> {
    fn default() -> Self {
        Self { past: vec![], future: vec![] }
    }
}

impl<T> History<T> {
    /// Remember the state before a change. A new change ends what could be redone.
    pub fn record(&mut self, before: T) {
        self.past.push(before);
        if self.past.len() > UNDO_LIMIT {
            self.past.remove(0);
        }
        self.future.clear();
    }

    pub fn undo(&mut self, current: T) -> Option<T> {
        let previous = self.past.pop()?;
        self.future.push(current);
        Some(previous)
    }

    pub fn redo(&mut self, current: T) -> Option<T> {
        let next = self.future.pop()?;
        self.past.push(current);
        Some(next)
    }

    pub fn can_undo(&self) -> bool {
        !self.past.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.future.is_empty()
    }

    /// What undo would go back to, without going.
    pub fn last(&self) -> Option<&T> {
        self.past.last()
    }

    /// What redo would go forward to, without going.
    pub fn next(&self) -> Option<&T> {
        self.future.last()
    }

    fn each_mut(&mut self, mut change: impl FnMut(&mut T)) {
        self.past.iter_mut().chain(&mut self.future).for_each(&mut change);
    }
}

/// Why a meeting opens without editing. Its file is left exactly as it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewOnly {
    /// The board could not be read.
    Unreadable,
    /// Saved by a newer version.
    NewerFormat,
    /// Free-edit content, which this version cannot show or edit.
    ImportedContent,
}

/// One undo position: the board, and where everything was placed. An edit of the transcript also keeps the
/// lines it changed, by their place, and any other board its redraw changed, so undo puts all of it back at once.
#[derive(Clone, Default)]
pub struct Snap {
    pub graph: Graph,
    pub memory: Value,
    /// Lines as they were, by place: only for an edit of the transcript.
    pub lines: Vec<(usize, Value)>,
    /// Boards other than the one on screen, by page, as their graphs were before a redraw changed them.
    pub pages: Vec<(usize, Value)>,
    /// The edit this position comes before, which its redraw joins rather than making a step of its own.
    pub edit: Option<u64>,
}

impl Snap {
    fn board(graph: Graph, memory: Value) -> Self {
        Self { graph, memory, ..Self::default() }
    }
}

/// A line handed to the model and given back with what it drew: its id where it has one, so it is found again
/// whatever its words, and who said what.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Spoken {
    pub id: Option<String>,
    pub speaker: Option<String>,
    pub text: String,
}

impl From<(Option<String>, String)> for Spoken {
    /// A line sent without an id, found again by its words.
    fn from((speaker, text): (Option<String>, String)) -> Self {
        Self { id: None, speaker, text }
    }
}

/// An edit of the transcript, as made: what the board then has to follow.
#[derive(Clone, Debug, PartialEq)]
pub struct LineEdit {
    /// Its number in this meeting, which its redraw carries.
    pub edit: u64,
    pub kind: EditKind,
    /// The line that holds the result: the edited line, the line merged into, the first deleted.
    pub line: usize,
    /// Every line it touched, by place, gone ones included.
    pub lines: Vec<usize>,
    /// Those lines as they were before, in the same order.
    pub before: Vec<Value>,
}

/// A note about a change made by hand: its text in English, which the meeting keeps as the line's text and a
/// model reads, and the message that shows it in the interface's language (`{"id", "args"}`), or null.
#[derive(Clone, Debug, Default)]
pub struct Note {
    pub text: String,
    pub message: Value,
}

impl Note {
    /// A note with no message: shown as written, in any language.
    pub fn plain(text: &str) -> Self {
        Self { text: text.to_string(), message: Value::Null }
    }
}

pub struct Live {
    raw: Map<String, Value>,
    pub origin: Origin,
    pub revision: u64,
    pub graph: Graph,
    pub memory: Value,
    pub lines: Vec<Value>,
    pub history: Vec<Value>,
    pub steps: Vec<Value>,
    voices: Vec<Voice>,
    /// A file's voices stay as written unless this meeting changed them.
    voices_changed: bool,
    /// The voiceprint each line joined its speaker's voice with, by the line's place, while the meeting is
    /// open: what moving one line to someone else takes from one voice and gives the other. Not saved.
    prints: HashMap<usize, Vec<f64>>,
    /// What the replay record has reached, to diff the next change against.
    recorded: Graph,
    undo: History<Snap>,
    /// Edits of the transcript made in this window, to number the next.
    edits: u64,
    /// Bumped by every change, so a save knows whether it caught up.
    pub changes: u64,
    /// Why this meeting can only be viewed, if so.
    pub read_only: Option<ViewOnly>,
}

fn array(raw: &Map<String, Value>, key: &str) -> Vec<Value> {
    raw.get(key).and_then(Value::as_array).cloned().unwrap_or_default()
}

fn scene_has_content(scene: Option<&Value>) -> bool {
    scene.is_some_and(|s| !s.is_null() && s["elements"].as_array().is_some_and(|e| !e.is_empty()))
}

impl Live {
    pub fn new() -> Self {
        let now = now_ms();
        let Value::Object(raw) = json!({ "version": 2, "id": new_id(), "title": UNTITLED, "createdAt": now, "updatedAt": now }) else {
            unreachable!("literal object")
        };
        Self::build(raw, Origin::New, 0)
    }

    pub fn from_loaded(loaded: Loaded) -> Self {
        Self::build(loaded.raw, loaded.origin, loaded.revision)
    }

    fn build(raw: Map<String, Value>, origin: Origin, revision: u64) -> Self {
        let graph: Result<Graph, _> = raw.get("graph").map_or_else(|| Ok(Graph::default()), |g| serde_json::from_value(g.clone()));
        let unreadable = graph.is_err();
        let graph = graph.unwrap_or_default();
        let mut lines = array(&raw, "lines");
        // Lines kept before lines had ids get them now; the next save keeps them.
        assign_ids(&mut lines);
        // A line still waiting for the model when the meeting closed will never be drawn.
        for line in &mut lines {
            if line["ms"].is_null() && !line["manual"].as_bool().unwrap_or(false) {
                line["ms"] = json!(0);
                line["failed"] = json!(true);
                line["boardFailure"] = json!("cancelled");
                line["ops"] = json!("");
            }
        }
        let mut steps = array(&raw, "steps");
        // Older meetings have no record of how the board grew, so the whole board counts as one step.
        if steps.is_empty() {
            if let Some(d) = diff(&Graph::default(), &ink_only(&graph)) {
                steps.push(json!({ "t": raw.get("updatedAt").cloned().unwrap_or(json!(0)), "line": lines.len(), "d": d }));
            }
        }
        let recorded = replay(&steps);
        let pages = raw.get("pages").and_then(Value::as_array);
        let imported = pages.is_some_and(|p| p.iter().any(|page| scene_has_content(page.get("scene")))) || scene_has_content(raw.get("scene"));
        let read_only = if unreadable {
            Some(ViewOnly::Unreadable)
        } else if raw.get("version").and_then(Value::as_u64).unwrap_or(1) > crate::store::MEETING_VERSION {
            Some(ViewOnly::NewerFormat)
        } else if imported {
            Some(ViewOnly::ImportedContent)
        } else {
            None
        };
        Self {
            voices: raw.get("voices").and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or_default(),
            voices_changed: false,
            prints: HashMap::new(),
            memory: raw.get("layout").cloned().unwrap_or(Value::Null),
            history: array(&raw, "history"),
            lines,
            steps,
            recorded,
            graph,
            undo: History::default(),
            edits: 0,
            changes: 0,
            read_only,
            raw,
            origin,
            revision,
        }
    }

    pub fn id(&self) -> &str {
        self.raw["id"].as_str().unwrap_or_default()
    }

    pub fn title(&self) -> &str {
        self.raw.get("title").and_then(Value::as_str).unwrap_or(UNTITLED)
    }

    /// Whether there is anything in the meeting to keep: something said, drawn by the model or by hand, or a
    /// summary.
    pub fn has_content(&self) -> bool {
        !self.lines.is_empty() || !self.graph.nodes.is_empty() || self.graph.has_annotations() || !self.summary().is_empty()
    }

    /// This meeting must not leave the machine, so it only uses the local model.
    pub fn local_only(&self) -> bool {
        self.raw.get("localOnly").and_then(Value::as_bool).unwrap_or(false)
    }

    pub fn pages(&self) -> usize {
        self.raw.get("pages").and_then(Value::as_array).map_or(1, |p| p.len().max(1))
    }

    pub fn page(&self) -> usize {
        self.raw.get("page").and_then(Value::as_u64).unwrap_or(0) as usize
    }

    /// Move to another board of this meeting, keeping the one being left.
    pub fn go_to_page(&mut self, page: usize) {
        if page >= self.pages() || page == self.page() {
            return;
        }
        self.store_page();
        let next = self.raw["pages"][page].clone();
        self.graph = serde_json::from_value(next["graph"].clone()).unwrap_or_default();
        self.memory = next.get("layout").cloned().unwrap_or(Value::Null);
        self.steps = next["steps"].as_array().cloned().unwrap_or_default();
        self.recorded = replay(&self.steps);
        self.undo = History::default();
        self.raw.insert("page".into(), json!(page));
    }

    /// A new board from step `at` of this one, placed as `memory` says. The original stays as it was.
    pub fn fork_page(&mut self, at: usize, memory: Value, note: &Note) {
        self.store_page();
        let at = at.min(self.steps.len());
        if self.raw.get("pages").and_then(Value::as_array).is_none_or(Vec::is_empty) {
            self.raw.insert("pages".into(), json!([{ "graph": self.graph, "layout": self.memory, "steps": self.steps }]));
        }
        let steps: Vec<Value> = self.steps[..at].to_vec();
        let graph = replay(&steps);
        let from = self.page();
        let pages = self.raw.get_mut("pages").and_then(Value::as_array_mut).expect("made above");
        let underlay = pages.get(from).and_then(|p| p.get("underlay")).cloned();
        let mut entry = json!({ "graph": graph, "layout": memory, "steps": steps });
        if let Some(underlay) = underlay {
            entry["underlay"] = underlay;
        }
        pages.push(entry);
        let to = pages.len() - 1;
        self.raw.insert("page".into(), json!(to));
        self.recorded = graph.clone();
        self.graph = graph;
        self.memory = memory;
        self.steps = steps;
        self.undo = History::default();
        self.add_note(note);
        self.touch();
    }

    /// Start a blank board after the others and show it. The one being left is kept as it is.
    pub fn add_board(&mut self, note: &Note) {
        self.store_page();
        if self.raw.get("pages").and_then(Value::as_array).is_none_or(Vec::is_empty) {
            self.raw.insert("pages".into(), json!([{ "graph": self.graph, "layout": self.memory, "steps": self.steps }]));
        }
        let pages = self.raw.get_mut("pages").and_then(Value::as_array_mut).expect("made above");
        pages.push(json!({ "graph": Graph::default(), "layout": Value::Null, "steps": [] }));
        let to = pages.len() - 1;
        self.raw.insert("page".into(), json!(to));
        self.graph = Graph::default();
        self.memory = Value::Null;
        self.steps = vec![];
        self.recorded = Graph::default();
        self.undo = History::default();
        self.add_note(note);
        self.touch();
    }

    /// Carry a topic, with everything under it, over to a new board and show that board: the talk moved on
    /// to it. On the board it leaves, its going can be undone and is replayed like any change.
    pub fn split_topic(&mut self, topic: &str, note: &Note) -> bool {
        self.split_nodes(&[topic.to_string()], note)
    }

    /// Carry some things, each with everything under it, over to a new board and show that board, as
    /// [`Self::split_topic`] does for a topic: what was circled on the iPad, sent to a board of its own.
    pub fn split_nodes(&mut self, ids: &[String], note: &Note) -> bool {
        let mut moved: HashSet<String> = ids.iter().filter(|id| self.graph.nodes.iter().any(|n| &n.id == *id)).cloned().collect();
        if moved.is_empty() {
            return false;
        }
        loop {
            let more: Vec<String> = self.graph.nodes.iter().filter(|n| n.parent.as_ref().is_some_and(|p| moved.contains(p)) && !moved.contains(&n.id)).map(|n| n.id.clone()).collect();
            if more.is_empty() {
                break;
            }
            moved.extend(more);
        }
        if moved.len() == self.graph.nodes.len() {
            return false;
        }
        let mut left = self.graph.clone();
        left.nodes.retain(|n| !moved.contains(&n.id));
        left.edges.retain(|e| !moved.contains(&e.from) && !moved.contains(&e.to));
        let mut taken = self.graph.clone();
        taken.nodes.retain(|n| moved.contains(&n.id));
        taken.edges.retain(|e| moved.contains(&e.from) && moved.contains(&e.to));
        taken.groups.clear();
        taken.extra.remove("underlay");
        let memory = self.memory.clone();
        self.change_board(left, memory);
        self.store_page();
        if self.raw.get("pages").and_then(Value::as_array).is_none_or(Vec::is_empty) {
            self.raw.insert("pages".into(), json!([{ "graph": self.graph, "layout": self.memory, "steps": self.steps }]));
        }
        let shown = ink_only(&taken);
        let steps: Vec<Value> = diff(&Graph::default(), &shown).map(|d| json!({ "t": now_ms(), "line": self.lines.len(), "d": d })).into_iter().collect();
        let pages = self.raw.get_mut("pages").and_then(Value::as_array_mut).expect("made above");
        pages.push(json!({ "graph": taken, "layout": Value::Null, "steps": steps }));
        let to = pages.len() - 1;
        self.raw.insert("page".into(), json!(to));
        self.recorded = shown;
        self.graph = taken;
        self.memory = Value::Null;
        self.steps = steps;
        self.undo = History::default();
        self.add_note(note);
        self.touch();
        true
    }

    /// Add imported pages, each a board and how to find its picture, and show the first. They take the
    /// place of a board with nothing on it, and otherwise go after the others.
    pub fn add_pages(&mut self, added: Vec<(Graph, String)>, note: &Note) {
        self.add_document(added, note, None);
    }

    /// As [`Live::add_pages`], marking the pages as one document, `(id, name)`, which a card on another board
    /// can open.
    pub fn add_document(&mut self, added: Vec<(Graph, String)>, note: &Note, doc: Option<(&str, &str)>) {
        if added.is_empty() {
            return;
        }
        self.store_page();
        if self.raw.get("pages").and_then(Value::as_array).is_none_or(Vec::is_empty) {
            self.raw.insert("pages".into(), json!([{ "graph": self.graph, "layout": self.memory, "steps": self.steps }]));
        }
        let current = self.page();
        let blank = self.graph.underlay().is_none() && !self.graph.nodes.iter().any(|n| n.ink == "ink");
        let pages = self.raw.get_mut("pages").and_then(Value::as_array_mut).expect("made above");
        let at = if blank { current } else { pages.len() };
        if blank {
            pages.remove(current);
        }
        let first = added[0].0.clone();
        let entries = added.into_iter().enumerate().map(|(i, (graph, underlay))| {
            let mut entry = json!({ "graph": graph, "underlay": underlay });
            if let Some((id, name)) = doc {
                entry["doc"] = json!(id);
                if i == 0 {
                    entry["docName"] = json!(name);
                }
            }
            entry
        });
        pages.splice(at..at, entries);
        self.raw.insert("page".into(), json!(at));
        self.graph = first;
        self.memory = Value::Null;
        self.steps = vec![];
        self.recorded = Graph::default();
        self.undo = History::default();
        self.add_note(note);
        // A meeting already under way keeps the name its own board gave it.
        if blank {
            self.refresh_title();
        }
        self.touch();
    }

    /// The boards of an imported document, in order.
    pub fn document_pages(&self, doc: &str) -> Vec<usize> {
        array(&self.raw, "pages").iter().enumerate().filter(|(_, p)| p["doc"].as_str() == Some(doc)).map(|(i, _)| i).collect()
    }

    /// The document a board belongs to, with its name, if it is an imported document's page.
    pub fn document_of(&self, page: usize) -> Option<(String, String)> {
        let pages = array(&self.raw, "pages");
        let doc = pages.get(page)?["doc"].as_str()?.to_string();
        let name = pages.iter().find(|p| p["doc"].as_str() == Some(&doc) && p["docName"].is_string()).and_then(|p| p["docName"].as_str()).unwrap_or_default().to_string();
        Some((doc, name))
    }

    /// Name the meeting, so its board's contents no longer do.
    pub fn set_title(&mut self, title: &str) {
        self.raw.insert("title".into(), json!(title));
        self.raw.insert("titled".into(), json!(true));
        self.touch();
    }

    /// The board on screen belongs in `pages` as well as the top level, as the older app keeps it.
    fn store_page(&mut self) {
        let page = self.page();
        let (graph, memory, steps) = (json!(self.graph), self.memory.clone(), json!(self.steps));
        if let Some(entry) = self.raw.get_mut("pages").and_then(|p| p.get_mut(page)).and_then(Value::as_object_mut) {
            entry.insert("graph".into(), graph);
            entry.insert("layout".into(), memory);
            entry.insert("steps".into(), steps);
        }
    }

    /// A line of speech or typing. Without `ms` it counts as still waiting for the model. Gives its id, for the
    /// model's answer to find it by.
    pub fn add_line(&mut self, speaker: Option<&str>, text: &str, asr_ms: Option<u128>) -> String {
        // When it was said, on the wall clock, for the transcript to show beside the speaker.
        let id = next_line_id(&self.lines);
        let mut line = json!({ "id": id, "text": text, "page": self.page(), "at": now_ms() });
        if let Some(speaker) = speaker {
            line["speaker"] = json!(speaker);
        }
        if let Some(ms) = asr_ms {
            line["heard"] = json!({ "asrMs": ms });
        }
        self.lines.push(line);
        self.touch();
        id
    }

    /// Where the line with this id is now.
    pub fn line_index(&self, id: &str) -> Option<usize> {
        self.lines.iter().position(|l| l["id"] == id)
    }

    /// Who else was heard talking at the same time as the last line, whose words are lost under it. An
    /// empty name is someone not yet named. Kept in the meeting as the line's `also`.
    pub fn mark_also(&mut self, names: &[String]) {
        if names.is_empty() {
            return;
        }
        if let Some(line) = self.lines.last_mut() {
            line["also"] = json!(names);
            self.touch();
        }
    }

    /// The last line asked to turn to another board rather than to draw: it is done, and says where it
    /// turned, so a replay can follow.
    pub fn turned_to(&mut self, page: usize) {
        if let Some(line) = self.lines.last_mut() {
            line["ms"] = json!(0);
            line["turnedTo"] = json!(page);
        }
        self.touch();
    }

    /// Whose voice this is, joining the ones this meeting has heard so that it keeps its name. `None` if
    /// the sentence is too short to name someone new. `slot_name` is who the diarization model's slot for
    /// this sentence was last, which a voice like no other joins rather than becoming someone new.
    pub fn identify_voice(&mut self, voiceprint: &[f32], seconds: f64, slot_name: Option<&str>) -> Option<String> {
        let print: Vec<f64> = voiceprint.iter().map(|x| f64::from(*x)).collect();
        let taken: Vec<&str> = self.lines.iter().filter_map(|l| l["speaker"].as_str()).collect();
        let most = self.participants().unwrap_or(MOST_SPEAKERS);
        let name = identify_in_slot(&mut self.voices, &print, seconds, &taken, slot_name, most)?;
        // The line this voice is for is the next one added.
        self.prints.insert(self.lines.len(), print);
        self.voices_changed = true;
        self.touch();
        Some(name)
    }

    /// How many people the meeting was said to have when it started, 1 to [`MOST_SPEAKERS`]; `None` for "Auto". Voices
    /// are told apart up to that many, never more ([`identify_in_slot`]).
    pub fn participants(&self) -> Option<usize> {
        self.raw.get("participants").and_then(Value::as_u64).map(|n| n as usize).filter(|n| (1..=MOST_SPEAKERS).contains(n))
    }

    /// Say how many people take part, or `None` for "Auto". Saved with the meeting.
    pub fn set_participants(&mut self, participants: Option<usize>) {
        if participants == self.participants() {
            return;
        }
        match participants.filter(|n| (1..=MOST_SPEAKERS).contains(n)) {
            Some(n) => self.raw.insert("participants".into(), json!(n)),
            None => self.raw.remove("participants"),
        };
        self.touch();
    }

    /// Everyone in this meeting, in the order they first spoke.
    pub fn speakers(&self) -> Vec<String> {
        let mut names: Vec<String> = vec![];
        let spoken = shown(&self.lines).filter_map(|(_, l)| l["speaker"].as_str());
        for name in spoken.chain(self.voices.iter().map(|v| v.name.as_str())) {
            if !name.is_empty() && !names.iter().any(|n| n == name) {
                names.push(name.to_string());
            }
        }
        names
    }

    /// What renaming a speaker touches: lines of the transcript, things on boards, and how many boards.
    pub fn speaker_impact(&self, who: &str) -> (usize, usize, usize) {
        let lines = shown(&self.lines).filter(|(_, l)| l["speaker"] == who).count();
        let raised: Vec<usize> = self.boards().iter().map(|graph| raised_by(graph, who)).collect();
        (lines, raised.iter().sum(), raised.iter().filter(|n| **n > 0).count())
    }

    /// Put one line down to someone else, leaving the speaker's other lines as they are. Its voiceprint, when
    /// this meeting heard it, moves too, so later lines in that voice go to the right person.
    pub fn move_line(&mut self, index: usize, to: &str) {
        let Some(line) = self.lines.get_mut(index) else { return };
        let from = line["speaker"].as_str().unwrap_or_default().to_string();
        if from == to || to.is_empty() {
            return;
        }
        line["speaker"] = json!(to);
        if let Some(print) = self.prints.get(&index) {
            move_sentence(&mut self.voices, print, &from, to);
            self.voices_changed = true;
        }
        self.touch();
    }

    /// Give a speaker a new name everywhere it is written. Taking a name someone has joins the two.
    pub fn rename_speaker(&mut self, from: &str, to: &str) {
        if from == to {
            return;
        }
        for said in self.lines.iter_mut().chain(&mut self.history) {
            if said["speaker"] == from {
                said["speaker"] = json!(to);
            }
            if let Some(also) = said.get_mut("also").and_then(Value::as_array_mut) {
                for name in also.iter_mut().filter(|n| *n == from) {
                    *name = json!(to);
                }
                // Renaming one voice to another's name joins them: each is listed once.
                let mut seen = std::collections::HashSet::new();
                also.retain(|n| seen.insert(n.to_string()));
            }
        }
        rename_voice(&mut self.voices, from, to);
        self.voices_changed = true;
        rename_by(&mut self.graph, from, to);
        rename_by(&mut self.recorded, from, to);
        rename_by_in_steps(&mut self.steps, from, to);
        self.undo.each_mut(|snap| rename_by(&mut snap.graph, from, to));
        // The boards not on screen stay in the file's own form until they are opened.
        if let Some(pages) = self.raw.get_mut("pages").and_then(Value::as_array_mut) {
            for page in pages {
                for board in ["graph", "base"] {
                    for part in ["nodes", "edges"] {
                        let Some(items) = page.get_mut(board).and_then(|b| b.get_mut(part)).and_then(Value::as_array_mut) else { continue };
                        for item in items.iter_mut().filter(|item| item["by"] == from) {
                            item["by"] = json!(to);
                        }
                    }
                }
                if let Some(steps) = page.get_mut("steps").and_then(Value::as_array_mut) {
                    rename_by_in_steps(steps, from, to);
                }
            }
        }
        self.touch();
    }

    /// Try a line the model never drew again: it goes back to waiting, and out of the context it was
    /// added to when it failed. Gives back who said what, to send on.
    pub fn retry_line(&mut self, index: usize) -> Option<(Option<String>, String)> {
        let line = self.lines.get_mut(index).filter(|l| l["failed"].as_bool().unwrap_or(false))?;
        let was_remembered = line["boardFailure"] == "request";
        let fields = line.as_object_mut()?;
        for field in ["ms", "failed", "boardFailure", "ops"] {
            fields.remove(field);
        }
        let text = line["text"].as_str()?.to_string();
        let speaker = line["speaker"].as_str().map(str::to_string);
        if was_remembered {
            if let Some(at) = self.history.iter().rposition(|said| said["text"] == text.as_str()) {
                self.history.remove(at);
            }
        }
        self.touch();
        Some((speaker, text))
    }

    /// A note about a hand-made change. Marked manual, so it is never read as something said; its text is in
    /// English, and its message shows it in the language of whoever looks.
    pub fn add_note(&mut self, note: &Note) {
        let mut line = json!({ "id": next_line_id(&self.lines), "text": note.text, "manual": true, "ms": 0, "page": self.page() });
        if !note.message.is_null() {
            line["note"] = note.message.clone();
        }
        self.lines.push(line);
    }

    /// Finish the waiting line this was: by its id, or for a line sent without one, the earliest waiting line with
    /// its text, as lines finish in the order they were said. Gives its place.
    fn settle(&mut self, said: &Spoken, update: impl Fn(&mut Value)) -> Option<usize> {
        let at = match &said.id {
            Some(id) => self.lines.iter().position(|l| l["ms"].is_null() && l["id"] == id.as_str()),
            None => self.lines.iter().position(|l| l["ms"].is_null() && l["text"] == said.text.as_str()),
        }?;
        update(&mut self.lines[at]);
        Some(at)
    }

    /// The model finished this batch of lines and the board became `graph`.
    pub fn model_drew(&mut self, spoken: &[Spoken], graph: Graph, memory: Value, ops: &str, llm_ms: u128) {
        let mut from = vec![];
        for (i, said) in spoken.iter().enumerate() {
            // A merged batch produced one set of operations, shown on its last line.
            let shown = if i + 1 == spoken.len() { ops } else { "" };
            from.extend(self.settle(said, |line| {
                line["ms"] = json!(llm_ms);
                line["ops"] = json!(shown);
            }));
            self.remember(said.speaker.as_deref(), &said.text);
        }
        self.change_board_from(graph, memory, from);
    }

    /// The model could not answer. The lines stay in the transcript, marked for a retry later.
    pub fn model_failed(&mut self, spoken: &[Spoken]) {
        for said in spoken {
            let (speaker, text) = (&said.speaker, &said.text);
            self.settle(said, |line| {
                line["ms"] = json!(0);
                line["failed"] = json!(true);
                line["boardFailure"] = json!("request");
                line["ops"] = json!("");
            });
            self.remember(speaker.as_deref(), text);
        }
        self.touch();
    }

    fn remember(&mut self, speaker: Option<&str>, text: &str) {
        let mut said = json!({ "text": text });
        if let Some(speaker) = speaker {
            said["speaker"] = json!(speaker);
        }
        self.history.push(said);
    }

    /// Any change to the board: it can be undone, and replay records it.
    pub fn change_board(&mut self, graph: Graph, memory: Value) {
        self.change_board_from(graph, memory, vec![]);
    }

    /// A change to the board drawn from these lines, by place, which its step records so that an edit of one of
    /// them later finds what it drew.
    fn change_board_from(&mut self, graph: Graph, memory: Value, from: Vec<usize>) {
        if graph == self.graph {
            self.memory = memory;
            self.touch();
            return;
        }
        let before = Snap::board(std::mem::replace(&mut self.graph, graph), std::mem::replace(&mut self.memory, memory));
        self.undo.record(before);
        self.record_step(&from, None);
        self.refresh_title();
        self.touch();
    }

    fn record_step(&mut self, from: &[usize], edit: Option<u64>) {
        let shown = ink_only(&self.graph);
        if let Some(d) = diff(&self.recorded, &shown) {
            let mut step = json!({ "t": now_ms(), "line": self.lines.len(), "d": d });
            if !from.is_empty() {
                step["from"] = json!(from);
            }
            if let Some(edit) = edit {
                step["edit"] = json!(edit);
            }
            self.steps.push(step);
            self.recorded = shown;
        }
    }

    pub fn can_undo(&self) -> bool {
        self.read_only.is_none() && self.undo.can_undo()
    }

    pub fn can_redo(&self) -> bool {
        self.read_only.is_none() && self.undo.can_redo()
    }

    /// Where things stand now, covering what `like` covers: the same lines and other boards, so going back to
    /// `like` can come forward again.
    fn current(&self, like: Option<&Snap>) -> Snap {
        let mut now = Snap::board(self.graph.clone(), self.memory.clone());
        if let Some(like) = like {
            now.lines = like.lines.iter().filter_map(|(i, _)| Some((*i, self.lines.get(*i)?.clone()))).collect();
            now.pages = like.pages.iter().map(|(p, _)| (*p, self.page_graph(*p))).collect();
            now.edit = like.edit;
        }
        now
    }

    fn step_to(&mut self, snap: Snap) -> Snap {
        self.graph = snap.graph.clone();
        self.memory = snap.memory.clone();
        for (i, line) in &snap.lines {
            if let Some(slot) = self.lines.get_mut(*i) {
                *slot = line.clone();
            }
        }
        for (page, graph) in &snap.pages {
            self.set_page_graph(*page, graph.clone());
        }
        if !snap.lines.is_empty() {
            self.history = history_of(&self.lines);
        }
        self.record_step(&[], snap.edit);
        self.touch();
        snap
    }

    /// Go back one change. What comes back says whether lines changed too ([`Snap::lines`]): the model's context
    /// then has to be told.
    pub fn undo(&mut self) -> Option<Snap> {
        let current = self.current(self.undo.last());
        let snap = self.undo.undo(current)?;
        Some(self.step_to(snap))
    }

    pub fn redo(&mut self) -> Option<Snap> {
        let current = self.current(self.undo.next());
        let snap = self.undo.redo(current)?;
        Some(self.step_to(snap))
    }

    /// A board's graph as the file has it; the one on screen as it is now.
    fn page_graph(&self, page: usize) -> Value {
        if page == self.page() {
            return json!(self.graph);
        }
        self.raw.get("pages").and_then(|p| p.get(page)).map_or(Value::Null, |p| p["graph"].clone())
    }

    fn set_page_graph(&mut self, page: usize, graph: Value) {
        if page == self.page() {
            return;
        }
        if let Some(entry) = self.raw.get_mut("pages").and_then(|p| p.get_mut(page)).and_then(Value::as_object_mut) {
            entry.insert("graph".into(), graph);
        }
    }

    /// The record of one board, by page: the one on screen as it is now.
    pub fn page_steps(&self, page: usize) -> Vec<Value> {
        if page == self.page() {
            return self.steps.clone();
        }
        self.raw.get("pages").and_then(|p| p.get(page)).and_then(|p| p["steps"].as_array()).cloned().unwrap_or_default()
    }

    /// One board's graph, by page.
    pub fn board_of(&self, page: usize) -> Graph {
        if page == self.page() {
            return self.graph.clone();
        }
        serde_json::from_value(self.page_graph(page)).unwrap_or_default()
    }

    // ---- editing the transcript (#362) ----

    /// Start an edit: the lines it will change are kept as they are, with the board, as one undo position.
    fn begin_edit(&mut self, kind: EditKind, line: usize, touched: &[usize]) -> LineEdit {
        self.edits += 1;
        let before: Vec<Value> = touched.iter().map(|i| self.lines[*i].clone()).collect();
        let mut snap = Snap::board(self.graph.clone(), self.memory.clone());
        snap.lines = touched.iter().copied().zip(before.iter().cloned()).collect();
        snap.edit = Some(self.edits);
        self.undo.record(snap);
        LineEdit { edit: self.edits, kind, line, lines: touched.to_vec(), before }
    }

    fn end_edit(&mut self, edit: LineEdit) -> Option<LineEdit> {
        self.history = history_of(&self.lines);
        self.touch();
        Some(edit)
    }

    /// Fix the words of a line already on the board. Nothing happens for the same words, an empty text, or a line
    /// that cannot be edited ([`edit::editable`]).
    pub fn edit_text(&mut self, i: usize, text: &str) -> Option<LineEdit> {
        let text = text.trim();
        let line = self.lines.get(i).filter(|l| editable(l))?;
        if text.is_empty() || line["text"] == text {
            return None;
        }
        let edit = self.begin_edit(EditKind::Text, i, &[i]);
        let line = &mut self.lines[i];
        keep_heard(line);
        line["text"] = json!(text);
        mark_edited(line, EditKind::Text, now_ms());
        self.end_edit(edit)
    }

    /// Give one line to someone else, voice and all ([`Live::move_line`]), as an edit that can be undone.
    pub fn edit_speaker(&mut self, i: usize, to: &str) -> Option<LineEdit> {
        let line = self.lines.get(i).filter(|l| editable(l))?;
        if to.is_empty() || line["speaker"] == to {
            return None;
        }
        let edit = self.begin_edit(EditKind::Speaker, i, &[i]);
        keep_heard(&mut self.lines[i]);
        self.move_line(i, to);
        mark_edited(&mut self.lines[i], EditKind::Speaker, now_ms());
        self.end_edit(edit)
    }

    /// Join lines said one after another into the first: their words in order, under `speaker` (the first line's
    /// when none is given) or as `text` when it was written out. The others stay where they were, merged into it.
    pub fn merge_lines(&mut self, picked: &[usize], speaker: Option<&str>, text: Option<&str>) -> Option<LineEdit> {
        if !edit::adjacent(&self.lines, picked) {
            return None;
        }
        let mut picked = picked.to_vec();
        picked.sort_unstable();
        picked.dedup();
        let first = picked[0];
        let joined = join_words(picked.iter().map(|i| self.lines[*i]["text"].as_str().unwrap_or_default()));
        let text = text.map(str::trim).filter(|t| !t.is_empty()).map_or(joined, str::to_string);
        let edit = self.begin_edit(EditKind::Merge, first, &picked);
        let at = now_ms();
        let line = &mut self.lines[first];
        keep_heard(line);
        line["text"] = json!(text);
        if let Some(speaker) = speaker.filter(|s| !s.is_empty()) {
            line["speaker"] = json!(speaker);
        }
        mark_edited(line, EditKind::Merge, at);
        line["edited"]["merged"] = json!(picked[1..]);
        for i in &picked[1..] {
            self.lines[*i]["mergedInto"] = json!(first);
        }
        self.end_edit(edit)
    }

    /// Take lines out of the transcript. They stay in the file, deleted, with what was said.
    pub fn delete_lines(&mut self, picked: &[usize]) -> Option<LineEdit> {
        let mut picked: Vec<usize> = picked.iter().copied().filter(|i| self.lines.get(*i).is_some_and(editable)).collect();
        picked.sort_unstable();
        picked.dedup();
        let first = *picked.first()?;
        let edit = self.begin_edit(EditKind::Delete, first, &picked);
        let at = now_ms();
        for i in &picked {
            let line = &mut self.lines[*i];
            line["deleted"] = json!(true);
            mark_edited(line, EditKind::Delete, at);
        }
        self.end_edit(edit)
    }

    /// Put a line back to what was heard: its words, who said it, and the lines merged into it.
    pub fn restore_line(&mut self, i: usize) -> Option<LineEdit> {
        let line = self.lines.get(i).filter(|l| !is_gone(l))?;
        let (text, speaker) = edit::heard(line).map(|(t, s)| (t.to_string(), s.map(str::to_string)))?;
        let merged: Vec<usize> = line["edited"]["merged"].as_array().map(|m| m.iter().filter_map(|x| x.as_u64().map(|x| x as usize)).collect()).unwrap_or_default();
        let mut touched = vec![i];
        touched.extend(merged.iter().copied().filter(|m| self.lines.get(*m).is_some_and(|l| l["mergedInto"] == i)));
        let edit = self.begin_edit(EditKind::Restore, i, &touched);
        let line = &mut self.lines[i];
        line["text"] = json!(text);
        match speaker {
            Some(s) => line["speaker"] = json!(s),
            None => {
                if let Some(fields) = line.as_object_mut() {
                    fields.remove("speaker");
                }
            }
        }
        for field in ["edited"] {
            if let Some(fields) = line.as_object_mut() {
                fields.remove(field);
            }
        }
        if let Some(heard) = line["heard"].as_object_mut() {
            heard.remove("text");
            heard.remove("speaker");
        }
        for m in &touched[1..] {
            if let Some(fields) = self.lines[*m].as_object_mut() {
                fields.remove("mergedInto");
            }
        }
        self.end_edit(edit)
    }

    /// The board an edit's redraw came back with, for `page`. It joins the edit's undo position when nothing came
    /// between them, and is its own position otherwise. `false` if the edit has been undone meanwhile: the board is
    /// left as it is.
    pub fn redrawn(&mut self, edit: u64, page: usize, graph: Graph, memory: Option<Value>) -> bool {
        let Some(at) = self.undo.past.iter().rposition(|s| s.edit == Some(edit)) else { return false };
        if page != self.page() {
            if page >= self.pages() {
                return false;
            }
            let before = self.page_graph(page);
            let snap = &mut self.undo.past[at];
            if !snap.pages.iter().any(|(p, _)| *p == page) {
                snap.pages.push((page, before));
            }
            self.set_page_graph(page, json!(graph));
            // The other board's record goes on from where it was, so its replay shows the redraw.
            if let Some(entry) = self.raw.get_mut("pages").and_then(|p| p.get_mut(page)).and_then(Value::as_object_mut) {
                let mut steps = entry.get("steps").and_then(Value::as_array).cloned().unwrap_or_default();
                if let Some(d) = diff(&replay(&steps), &ink_only(&graph)) {
                    steps.push(json!({ "t": now_ms(), "line": self.lines.len(), "d": d, "edit": edit }));
                    entry.insert("steps".into(), json!(steps));
                }
            }
            self.touch();
            return true;
        }
        let memory = memory.unwrap_or_else(|| self.memory.clone());
        if at + 1 == self.undo.past.len() {
            self.graph = graph;
            self.memory = memory;
            self.record_step(&[], Some(edit));
            self.refresh_title();
            self.touch();
        } else {
            let before = Snap::board(std::mem::replace(&mut self.graph, graph), std::mem::replace(&mut self.memory, memory));
            self.undo.record(before);
            self.record_step(&[], Some(edit));
            self.touch();
        }
        true
    }

    /// Every board of the meeting, with the one on screen as it is now.
    pub fn boards(&self) -> Vec<Graph> {
        self.board_pages().into_iter().map(|(graph, _)| graph).collect()
    }

    /// Every board with where its things were placed.
    pub fn board_pages(&self) -> Vec<(Graph, Value)> {
        let Some(pages) = self.raw.get("pages").and_then(Value::as_array).filter(|p| !p.is_empty()) else {
            return vec![(self.graph.clone(), self.memory.clone())];
        };
        let at = self.page();
        pages
            .iter()
            .enumerate()
            .map(|(i, p)| {
                if i == at {
                    (self.graph.clone(), self.memory.clone())
                } else {
                    (serde_json::from_value(p["graph"].clone()).unwrap_or_default(), p.get("layout").cloned().unwrap_or(Value::Null))
                }
            })
            .collect()
    }

    /// The record of every board, in the order of `board_pages`.
    pub fn board_steps(&self) -> Vec<Vec<Value>> {
        let Some(pages) = self.raw.get("pages").and_then(Value::as_array).filter(|p| !p.is_empty()) else {
            return vec![self.steps.clone()];
        };
        let at = self.page();
        pages.iter().enumerate().map(|(i, p)| if i == at { self.steps.clone() } else { array(p.as_object().unwrap_or(&Map::new()), "steps") }).collect()
    }

    /// How each board refers to its imported page's image, in the order of `board_pages`.
    pub fn board_images(&self) -> Vec<Option<String>> {
        let Some(pages) = self.raw.get("pages").and_then(Value::as_array).filter(|p| !p.is_empty()) else {
            return vec![None];
        };
        pages.iter().map(|p| p["underlay"].as_str().map(str::to_string)).collect()
    }

    /// When the meeting began, in milliseconds.
    pub fn created_at(&self) -> Option<f64> {
        self.raw.get("createdAt").and_then(Value::as_f64)
    }

    /// How long the microphone has been on for this meeting, in milliseconds.
    pub fn recorded_ms(&self) -> f64 {
        self.raw.get("recordedMs").and_then(Value::as_f64).unwrap_or(0.0)
    }

    /// Count another stretch of listening towards the meeting.
    pub fn add_recorded(&mut self, ms: f64) {
        let total = self.recorded_ms() + ms.max(0.0);
        self.raw.insert("recordedMs".into(), json!(total.round()));
        self.touch();
    }

    /// The audio kept of this meeting, a file for each time it listened: its name, when it began and how long
    /// it is, in milliseconds.
    pub fn audio(&self) -> Vec<(String, f64, f64)> {
        array(&self.raw, "audio").iter().filter_map(|a| Some((a["file"].as_str()?.to_string(), a["start"].as_f64()?, a["ms"].as_f64()?))).collect()
    }

    /// Note a file of this meeting's audio, once listening into it has stopped.
    pub fn add_audio(&mut self, file: &str, start: f64, ms: f64) {
        let mut all = array(&self.raw, "audio");
        all.push(json!({ "file": file, "start": start.round(), "ms": ms.max(0.0).round() }));
        self.raw.insert("audio".into(), Value::Array(all));
        self.touch();
    }

    pub fn summary(&self) -> &str {
        self.raw.get("summary").and_then(Value::as_str).unwrap_or_default()
    }

    /// The fingerprint of what the summary was made from.
    pub fn summary_source(&self) -> Option<&str> {
        self.raw.get("summarySource").and_then(Value::as_str)
    }

    /// An empty summary is no summary: the file then has neither field, as the older app writes it.
    pub fn set_summary(&mut self, text: &str, source: Option<&str>) {
        if text.is_empty() {
            self.raw.remove("summary");
            self.raw.remove("summarySource");
            self.raw.remove("summaryWritten");
        } else {
            self.raw.insert("summary".into(), json!(text));
            match source {
                Some(source) => self.raw.insert("summarySource".into(), json!(source)),
                None => self.raw.remove("summarySource"),
            };
        }
        self.touch();
    }

    /// When the summary was written by the model, in milliseconds since 1970: none for meetings saved before this was
    /// kept, or for a summary only ever typed.
    pub fn summary_written(&self) -> Option<i64> {
        self.raw.get("summaryWritten").and_then(Value::as_i64)
    }

    /// Note when the summary there now was written. Nothing is noted without a summary.
    pub fn note_summary_written(&mut self, at_ms: i64) {
        if !self.summary().is_empty() {
            self.raw.insert("summaryWritten".into(), json!(at_ms));
            self.touch();
        }
    }

    /// Older meetings have a summary but no fingerprint, so opening one takes the meeting as it stands.
    pub fn adopt_summary_source(&mut self, source: &str) {
        if !self.summary().is_empty() && self.summary_source().is_none() {
            self.raw.insert("summarySource".into(), json!(source));
        }
    }

    /// An unnamed meeting takes its title from the board: first topic, then first node, then first line.
    fn refresh_title(&mut self) {
        if self.raw.get("titled").and_then(Value::as_bool).unwrap_or(false) {
            return;
        }
        let name = self
            .graph
            .nodes
            .iter()
            .find(|n| n.kind == Kind::Topic)
            .or_else(|| self.graph.nodes.first())
            .map(|n| n.label.clone())
            .or_else(|| shown(&self.lines).next().and_then(|(_, l)| l["text"].as_str()).map(|t| t.chars().take(20).collect()));
        if let Some(name) = name {
            self.raw.insert("title".into(), json!(name));
        }
    }

    fn touch(&mut self) {
        self.changes += 1;
    }

    /// The meeting as it should be written now.
    pub fn snapshot(&mut self) -> Map<String, Value> {
        self.store_page();
        let mut out = self.raw.clone();
        out.insert("updatedAt".into(), json!(now_ms()));
        out.insert("graph".into(), json!(self.graph));
        out.insert("layout".into(), self.memory.clone());
        out.insert("lines".into(), json!(self.lines));
        out.insert("history".into(), json!(self.history));
        out.insert("steps".into(), json!(self.steps));
        if self.voices_changed {
            out.insert("voices".into(), json!(self.voices));
        }
        out
    }

    /// The saved copy has this revision; a later save builds on it.
    pub fn saved(&mut self, revision: u64) {
        // The store records this on the first save, and every later save must keep it.
        if let Origin::Legacy { source_hash } = &self.origin {
            self.raw.entry("migration").or_insert_with(|| json!({ "sourceHash": source_hash }));
        }
        self.revision = revision;
        self.origin = Origin::Current;
    }
}

impl Default for Live {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;
    use alcyo_board::graph::parse_ops;
    use alcyo_board::steps::replay;

    fn with(graph: &Graph, ops: &str) -> Graph {
        let mut g = graph.clone();
        g.apply(&parse_ops(ops));
        g
    }

    fn loaded(value: Value, origin: Origin) -> Live {
        let Value::Object(raw) = value else { panic!("object") };
        Live::from_loaded(Loaded { raw, origin, revision: 0 })
    }

    /// A meeting with these lines said and drawn, each drawing one node named after its words.
    fn drawn(said: &[(&str, &str)]) -> Live {
        let mut live = Live::new();
        for (who, text) in said {
            let id = live.add_line(Some(who), text, Some(10));
            let mut g = live.graph.clone();
            g.apply(&parse_ops(&format!("N 「{text}」")));
            live.model_drew(&[Spoken { id: Some(id), speaker: Some(who.to_string()), text: text.to_string() }], g, Value::Null, "N", 5);
        }
        live
    }

    #[test]
    fn lines_get_ids_and_the_model_finds_them_by_id() {
        let mut live = Live::new();
        let a = live.add_line(Some("A"), "同一句", None);
        let b = live.add_line(Some("A"), "同一句", None);
        assert_ne!(a, b);
        // The second comes back first: found by its id, not by its words.
        live.model_drew(&[Spoken { id: Some(b.clone()), speaker: Some("A".into()), text: "同一句".into() }], Graph::default(), Value::Null, "", 7);
        assert!(live.lines[0]["ms"].is_null());
        assert_eq!(live.lines[1]["ms"], 7);
        assert_eq!(live.line_index(&b), Some(1));
        // Lines from before ids get them when opened.
        let old = loaded(json!({ "id": "m", "lines": [{ "text": "a", "ms": 1 }, { "text": "b", "ms": 1 }] }), Origin::Current);
        assert_eq!(old.lines[0]["id"], "L1");
        assert_eq!(old.lines[1]["id"], "L2");
    }

    #[test]
    fn a_step_records_the_lines_it_drew() {
        let live = drawn(&[("A", "甲"), ("B", "乙")]);
        assert_eq!(live.steps[0]["from"], json!([0]));
        assert_eq!(live.steps[1]["from"], json!([1]));
    }

    #[test]
    fn an_edit_keeps_what_was_heard_and_its_redraw_undoes_with_it() {
        let mut live = drawn(&[("E", "删掉选项一")]);
        let before = live.graph.clone();
        let edit = live.edit_text(0, "先留着选项一").unwrap();
        assert_eq!(edit.kind, EditKind::Text);
        assert_eq!(live.lines[0]["text"], "先留着选项一");
        assert_eq!(live.lines[0]["heard"]["text"], "删掉选项一");
        assert_eq!(live.lines[0]["heard"]["asrMs"], 10);
        assert_eq!(live.history, vec![json!({ "text": "先留着选项一", "speaker": "E" })]);
        let mut redrawn = live.graph.clone();
        redrawn.apply(&parse_ops("R n1 「先留着选项一」"));
        assert!(live.redrawn(edit.edit, 0, redrawn.clone(), None));
        assert_eq!(live.steps.last().unwrap()["edit"], edit.edit);
        // One undo puts back both the words and the board.
        let snap = live.undo().unwrap();
        assert!(!snap.lines.is_empty());
        assert_eq!(live.lines[0]["text"], "删掉选项一");
        assert_eq!(live.graph, before);
        assert_eq!(live.history, vec![json!({ "text": "删掉选项一", "speaker": "E" })]);
        // Redo brings both back without asking the model again.
        live.redo().unwrap();
        assert_eq!(live.lines[0]["text"], "先留着选项一");
        assert_eq!(live.graph, redrawn);
    }

    #[test]
    fn a_redraw_after_other_changes_is_its_own_step_and_an_undone_edit_takes_none() {
        let mut live = drawn(&[("A", "甲")]);
        let edit = live.edit_text(0, "甲乙").unwrap();
        let mut g = live.graph.clone();
        g.apply(&parse_ops("N 「手画的」"));
        live.change_board(g, Value::Null);
        let mut redrawn = live.graph.clone();
        redrawn.apply(&parse_ops("R n1 「甲乙」"));
        assert!(live.redrawn(edit.edit, 0, redrawn.clone(), None));
        assert_eq!(live.graph, redrawn);
        live.undo();
        assert!(live.graph.nodes.iter().any(|n| n.label == "甲"), "undo takes off the redraw first");
        let mut live = drawn(&[("A", "甲")]);
        let edit = live.edit_text(0, "乙").unwrap();
        live.undo();
        assert!(!live.redrawn(edit.edit, 0, Graph::default(), None), "the edit was undone before its redraw came");
        assert_eq!(live.graph.nodes.len(), 1);
    }

    #[test]
    fn merging_keeps_the_others_as_tombstones_and_restore_brings_them_back() {
        let mut live = drawn(&[("D", "可以共同的多少"), ("B", "钱"), ("D", "要算清楚。")]);
        let steps = live.steps.len();
        assert!(live.merge_lines(&[0, 2], None, None).is_none(), "B said something between");
        let edit = live.merge_lines(&[0, 1, 2], None, None).unwrap();
        assert_eq!(edit.lines, vec![0, 1, 2]);
        assert_eq!(live.lines[0]["text"], "可以共同的多少钱要算清楚。");
        assert_eq!(live.lines[0]["speaker"], "D", "the first line's speaker by default");
        assert_eq!(live.lines[1]["mergedInto"], 0);
        assert_eq!(live.lines.len(), 3, "no line leaves the file");
        assert_eq!(shown(&live.lines).count(), 1);
        assert_eq!(live.steps.len(), steps, "steps keep their places");
        assert_eq!(live.speakers(), vec!["D".to_string()]);
        live.restore_line(0).unwrap();
        assert_eq!(live.lines[0]["text"], "可以共同的多少");
        assert!(live.lines[0]["edited"].is_null() && live.lines[0]["heard"]["text"].is_null());
        assert_eq!(shown(&live.lines).count(), 3);
        // A merge written out, under another speaker.
        let edit = live.merge_lines(&[1, 2], Some("B"), Some("钱要算清楚")).unwrap();
        assert_eq!((edit.line, live.lines[1]["text"].as_str(), live.lines[1]["speaker"].as_str()), (1, Some("钱要算清楚"), Some("B")));
    }

    #[test]
    fn deleting_hides_lines_and_waiting_or_noted_lines_cannot_be_edited() {
        let mut live = drawn(&[("A", "嗯"), ("A", "正题")]);
        live.add_line(Some("A"), "还在等", None);
        live.add_note(&Note::plain("note"));
        assert!(live.edit_text(2, "x").is_none(), "still waiting for the board");
        assert!(live.edit_text(3, "x").is_none(), "a note about a change");
        assert!(live.edit_text(1, "正题").is_none(), "the same words");
        let edit = live.delete_lines(&[0, 2]).unwrap();
        assert_eq!(edit.lines, vec![0], "only lines on the board");
        assert_eq!(live.lines[0]["deleted"], true);
        assert_eq!(live.lines[0]["text"], "嗯", "what was said stays in the file");
        assert_eq!(live.history, vec![json!({ "text": "正题", "speaker": "A" })]);
        live.undo();
        assert!(live.lines[0]["deleted"].is_null());
    }

    #[test]
    fn one_line_to_someone_else_is_an_edit_too() {
        let mut live = drawn(&[("B", "那你要什么？")]);
        live.edit_speaker(0, "D").unwrap();
        assert_eq!(live.lines[0]["speaker"], "D");
        assert_eq!(live.lines[0]["heard"]["speaker"], "B");
        live.undo();
        assert_eq!(live.lines[0]["speaker"], "B");
    }

    #[test]
    fn an_edited_meeting_is_written_as_the_newer_format_and_read_back() {
        let dir = std::env::temp_dir().join(format!("alcyo-edit-{}", std::process::id()));
        let store = Store::new(dir.clone());
        let mut live = drawn(&[("A", "甲"), ("A", "乙")]);
        let id = live.id().to_string();
        let rev = store.save(&id, live.snapshot(), &live.origin, 0).unwrap();
        assert_eq!(store.load(&id).unwrap().unwrap().raw["version"], 2, "nothing edited: older versions still edit it");
        live.saved(rev);
        live.delete_lines(&[1]).unwrap();
        let rev = store.save(&id, live.snapshot(), &live.origin, rev).unwrap();
        let back = Live::from_loaded(store.load(&id).unwrap().unwrap());
        assert_eq!(back.raw["version"], 3);
        assert!(back.read_only.is_none(), "this version reads what it wrote");
        assert_eq!(back.lines[1]["deleted"], true);
        assert_eq!(rev, 2);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_redraw_of_another_board_is_undone_with_its_edit() {
        let mut live = drawn(&[("A", "甲")]);
        live.add_board(&Note::plain("new board"));
        assert_eq!(live.page(), 1);
        let edit = live.edit_text(0, "甲乙").unwrap();
        let mut other = live.board_of(0);
        other.apply(&parse_ops("R n1 「甲乙」"));
        assert!(live.redrawn(edit.edit, 0, other, None));
        assert_eq!(live.board_of(0).nodes[0].label, "甲乙");
        assert_eq!(live.page_steps(0).last().unwrap()["edit"], edit.edit);
        live.undo();
        assert_eq!(live.board_of(0).nodes[0].label, "甲");
        assert_eq!(live.lines[0]["text"], "甲");
    }

    #[test]
    fn circled_things_move_to_a_board_of_their_own() {
        let mut live = Live::new();
        let mut g = Graph::default();
        g.apply(&alcyo_board::graph::parse_ops("N 「网关」\nN 「鉴权」\nN 「Redis」\nE 「网关」 -> 「鉴权」\nE 「鉴权」 -> 「Redis」"));
        live.change_board(g, Value::Null);
        let id = |live: &Live, l: &str| live.graph.nodes.iter().find(|n| n.label == l).unwrap().id.clone();
        let picked = vec![id(&live, "网关"), id(&live, "鉴权")];
        assert!(live.split_nodes(&picked, &Note::plain("圈出的")));
        assert_eq!((live.pages(), live.page()), (2, 1));
        assert_eq!(live.graph.nodes.len(), 2);
        assert_eq!(live.graph.edges.len(), 1, "the link between the two moved with them");
        assert!(!live.split_nodes(&["n99".into()], &Note::plain("none")), "nothing there, nothing moves");
    }

    #[test]
    fn pen_marks_old_and_new_survive_saving_and_loading() {
        use alcyo_board::graph::Annotation;
        let old = json!({ "id": "m", "graph": { "nodes": [], "edges": [], "groups": [], "nextNode": 1, "nextGroup": 1,
            "annotations": [{ "tool": "pen", "points": [[1.0, 2.0], [3.0, 4.0]] }, { "tool": "pink", "points": [[5.0, 5.0], [9.0, 5.0]] }] } });
        let mut live = loaded(old, Origin::Current);
        assert_eq!(live.graph.annotations().len(), 2, "a meeting from before the black and blue pens reads as it was");
        let mut g = live.graph.clone();
        g.annotate(Annotation { tool: "pen-blue".into(), on: None, points: vec![[0.0, 0.0], [8.0, 8.0]], anchor: None });
        g.annotate(Annotation { tool: "pen-black".into(), on: None, points: vec![[0.0, 8.0], [8.0, 0.0]], anchor: None });
        live.change_board(g, Value::Null);
        let saved = Value::Object(live.snapshot());
        let back = loaded(saved, Origin::Current);
        let tools: Vec<String> = back.graph.annotations().into_iter().map(|a| a.tool).collect();
        assert_eq!(tools, ["pen", "pink", "pen-blue", "pen-black"]);
    }

#[test]
    fn a_topic_moves_to_a_board_of_its_own() {
        let mut live = Live::new();
        let mut g = Graph::default();
        g.apply(&alcyo_board::graph::parse_ops("T 「登录」\nL 「登录」 「短信验证」\nT 「支付」\nL 「支付」 「微信支付」\nL 「微信支付」 「回调」\nE 「短信验证」 -> 「微信支付」"));
        live.change_board(g, Value::Null);
        let pay = live.graph.nodes.iter().find(|n| n.label == "支付").unwrap().id.clone();
        assert!(live.split_topic(&pay, &Note::plain("新话题")));
        assert_eq!((live.pages(), live.page()), (2, 1));
        let labels = |g: &Graph| g.nodes.iter().map(|n| n.label.clone()).collect::<Vec<_>>();
        assert_eq!(labels(&live.graph), ["支付", "微信支付", "回调"]);
        assert!(live.graph.edges.is_empty(), "a link to what stayed behind is not carried");
        assert!(!live.steps.is_empty(), "the new board replays from its first stroke");
        live.go_to_page(0);
        assert_eq!(labels(&live.graph), ["登录", "短信验证"]);
        assert!(live.graph.edges.is_empty());
        // Nothing to split off, or everything.
        let login = live.graph.nodes[0].id.clone();
        assert!(!live.split_topic(&login, &Note::plain("x")));
        assert!(!live.split_topic("n99", &Note::plain("x")));
    }

        #[test]
    fn who_else_spoke_is_kept_and_follows_a_rename() {
        let mut live = Live::build(json!({ "id": "x" }).as_object().unwrap().clone(), Origin::Current, 1);
        live.add_line(Some("A"), "first", Some(1));
        live.mark_also(&[]);
        assert!(live.lines[0].get("also").is_none());
        live.mark_also(&["B".to_string(), String::new()]);
        assert_eq!(live.lines[0]["also"], json!(["B", ""]));
        live.rename_speaker("B", "Mia");
        assert_eq!(live.lines[0]["also"], json!(["Mia", ""]));
        live.mark_also(&["Mia".to_string(), "C".to_string()]);
        live.rename_speaker("C", "Mia");
        assert_eq!(live.lines[0]["also"], json!(["Mia"]));
        assert_eq!(live.snapshot()["lines"][0]["also"], json!(["Mia"]));
    }

    #[test]
    fn a_meeting_drawn_on_only_by_hand_is_kept() {
        let mut live = Live::new();
        assert!(!live.has_content());
        let mut graph = live.graph.clone();
        graph.annotate(alcyo_board::graph::Annotation { tool: "pen-black".into(), on: None, points: vec![[1.0, 1.0], [9.0, 4.0]], anchor: None });
        live.change_board(graph, json!({}));
        assert!(live.has_content(), "a pen stroke on the board is something to save");
    }

    #[test]
    fn a_summary_is_written_and_removed_as_the_older_app_does() {
        let mut live = Live::new();
        assert!(!live.has_content());
        let before = live.changes;
        live.set_summary("### 结论\n- 好", Some("abc-1"));
        assert!(live.has_content());
        assert!(live.changes > before, "it needs saving");
        let raw = live.snapshot();
        assert_eq!(raw["summary"], "### 结论\n- 好");
        assert_eq!(raw["summarySource"], "abc-1");
        live.set_summary("", Some("abc-1"));
        let raw = live.snapshot();
        assert!(!raw.contains_key("summary") && !raw.contains_key("summarySource"));
        assert_eq!(live.summary_source(), None);
    }

    /// When the summary was written is kept with it, goes with it, and is simply absent in older meetings.
    #[test]
    fn a_summary_keeps_when_it_was_written() {
        let mut live = Live::new();
        live.note_summary_written(1_000);
        assert_eq!(live.summary_written(), None, "no summary, nothing to note");
        live.set_summary("- a", Some("s-1"));
        live.note_summary_written(1_764_000_000_000);
        assert_eq!(live.summary_written(), Some(1_764_000_000_000));
        assert_eq!(live.snapshot()["summaryWritten"], 1_764_000_000_000i64);
        live.set_summary("- a, edited", Some("s-1"));
        assert_eq!(live.summary_written(), Some(1_764_000_000_000), "an edit keeps when it was written");
        let reopened = loaded(Value::Object(live.snapshot()), Origin::Current);
        assert_eq!(reopened.summary_written(), Some(1_764_000_000_000));
        live.set_summary("", None);
        assert_eq!(live.summary_written(), None);
        assert!(!live.snapshot().contains_key("summaryWritten"));
        let older = loaded(json!({ "id": "m", "summary": "旧摘要" }), Origin::Current);
        assert_eq!(older.summary_written(), None);
    }

    #[test]
    fn a_summary_without_a_fingerprint_takes_one_only_on_opening() {
        let mut live = loaded(json!({ "id": "m", "summary": "旧摘要" }), Origin::Current);
        let changes = live.changes;
        live.adopt_summary_source("now-1");
        assert_eq!(live.summary_source(), Some("now-1"));
        assert_eq!(live.changes, changes, "opening a meeting is not a change");
        live.adopt_summary_source("later-2");
        assert_eq!(live.summary_source(), Some("now-1"));
        let mut empty = Live::new();
        empty.adopt_summary_source("now-1");
        assert_eq!(empty.summary_source(), None);
    }

    #[test]
    fn forking_keeps_the_original_and_starts_from_that_moment() {
        let mut live = Live::new();
        let one = with(&live.graph, "T 「一」");
        live.change_board(one.clone(), json!({ "n": 1 }));
        let two = with(&one, "T 「二」");
        live.change_board(two.clone(), json!({ "n": 2 }));
        assert_eq!(live.steps.len(), 2);
        live.fork_page(1, json!({ "n": 9 }), &Note::plain("另起一板"));
        assert_eq!((live.pages(), live.page()), (2, 1));
        assert_eq!(live.graph.nodes, one.nodes);
        assert_eq!(live.memory, json!({ "n": 9 }));
        assert_eq!(live.steps.len(), 1);
        assert!(!live.can_undo());
        assert_eq!(live.lines.last().unwrap()["manual"], true);
        assert_eq!(live.lines.last().unwrap()["page"], 1);
        let boards = live.board_pages();
        assert_eq!(boards[0].0.nodes, two.nodes, "the original is untouched");
        assert_eq!(boards[0].1, json!({ "n": 2 }));
        let raw = live.snapshot();
        assert_eq!(raw["pages"][0]["steps"].as_array().unwrap().len(), 2);
        assert_eq!(replay(raw["steps"].as_array().unwrap()).nodes, one.nodes);
        live.go_to_page(0);
        assert_eq!(live.graph.nodes, two.nodes);
        assert_eq!(live.steps.len(), 2);
    }

    #[test]
    fn imported_pages_can_be_edited_and_free_edit_scenes_cannot() {
        let page = json!({ "nodes": [{ "id": "n1", "label": "原文", "aliases": [], "kind": "region", "marks": [], "ink": "ink", "at": { "x": 1, "y": 2, "w": 3, "h": 4 } }], "edges": [], "groups": [], "nextNode": 2, "nextGroup": 1, "underlay": { "w": 900, "h": 1200 } });
        let imported = loaded(json!({ "id": "m", "graph": page, "page": 0, "pages": [{ "graph": page, "underlay": "asset:abc.jpg" }] }), Origin::Current);
        assert_eq!(imported.read_only, None);
        assert_eq!(imported.board_images(), [Some("asset:abc.jpg".to_string())]);
        let scene = json!({ "elements": [{ "id": "a" }], "files": {} });
        let free = loaded(json!({ "id": "m", "graph": page, "page": 0, "pages": [{ "graph": page, "scene": scene }] }), Origin::Current);
        assert_eq!(free.read_only, Some(ViewOnly::ImportedContent));
    }

    #[test]
    fn saving_a_migrated_meeting_again_keeps_its_migration_record() {
        let dir = std::env::temp_dir().join(format!("alcyo-migration-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("meetings")).unwrap();
        let text = json!({ "id": "m1", "title": "旧" }).to_string();
        std::fs::write(dir.join("meetings/m1.json"), &text).unwrap();
        let store = Store::new(dir.clone());
        let mut live = Live::from_loaded(store.load("m1").unwrap().unwrap());
        for _ in 0..3 {
            let revision = store.save("m1", live.snapshot(), &live.origin.clone(), live.revision).unwrap();
            live.saved(revision);
            let on_disk: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("meetings/v2/m1.json")).unwrap()).unwrap();
            assert_eq!(on_disk["migration"]["sourceHash"], Store::source_hash(&text));
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    fn page_board(no: u32) -> Graph {
        serde_json::from_value(json!({ "nodes": [{ "id": "n1", "label": format!("第 {no} 页"), "aliases": [], "kind": "region", "marks": [], "ink": "ink", "at": { "x": 1, "y": 2, "w": 3, "h": 4 } }], "edges": [], "groups": [], "nextNode": 2, "nextGroup": 1, "underlay": { "w": 900, "h": 1200, "no": no } })).unwrap()
    }

    #[test]
    fn kept_audio_is_noted_and_saved_with_the_meeting() {
        let mut live = Live::new();
        assert!(live.audio().is_empty());
        live.add_audio("audio-1000.opus", 1000.4, 61_234.6);
        live.add_audio("audio-90000.opus", 90_000.0, 5_000.0);
        assert_eq!(live.audio(), [("audio-1000.opus".to_string(), 1000.0, 61_235.0), ("audio-90000.opus".to_string(), 90_000.0, 5_000.0)]);
        let saved = live.snapshot();
        assert_eq!(saved["audio"].as_array().map(Vec::len), Some(2), "it goes in the meeting's file");
    }

    #[test]
    fn a_new_board_is_blank_shown_and_leaves_the_last_one_as_it_was() {
        let mut live = Live::new();
        live.change_board(page_board(1), Value::Null);
        let kept = live.graph.clone();
        live.add_board(&Note::plain("新开了一张板"));
        assert_eq!((live.pages(), live.page()), (2, 1));
        assert!(live.graph.nodes.is_empty() && live.steps.is_empty() && !live.can_undo());
        assert_eq!(live.boards()[0], kept, "the board left behind keeps what it had");
        assert_eq!(live.lines.last().unwrap()["text"], "新开了一张板");
        live.add_board(&Note::plain("again"));
        assert_eq!((live.pages(), live.page()), (3, 2));
    }

    #[test]
    fn an_imported_document_knows_its_pages_and_name() {
        let mut live = Live::new();
        live.add_pages(vec![(page_board(9), "asset:x.jpg".into())], &Note::plain("导入了"));
        live.add_document(vec![(page_board(1), "asset:a.jpg".into()), (page_board(2), "asset:b.jpg".into())], &Note::plain("导入了"), Some(("d1", "Serving latency")));
        assert_eq!(live.document_pages("d1"), [1, 2]);
        assert_eq!(live.document_of(2), Some(("d1".to_string(), "Serving latency".to_string())));
        assert_eq!(live.document_of(0), None, "a page imported on its own belongs to no document");
        assert!(live.document_pages("nothing").is_empty());
    }

    #[test]
    fn imported_pages_replace_an_empty_board() {
        let mut live = Live::new();
        live.add_pages(vec![(page_board(1), "asset:a.jpg".into()), (page_board(2), "asset:b.jpg".into())], &Note::plain("导入了"));
        assert_eq!((live.pages(), live.page()), (2, 0));
        assert_eq!(live.graph.nodes[0].label, "第 1 页");
        assert_eq!(live.board_images(), [Some("asset:a.jpg".to_string()), Some("asset:b.jpg".to_string())]);
        assert_eq!(live.lines[0]["manual"], true);
        assert_eq!(live.title(), "第 1 页");
        assert!(live.has_content());
        let raw = live.snapshot();
        assert_eq!(raw["pages"][1]["graph"]["underlay"]["no"], 2);
        live.go_to_page(1);
        assert_eq!(live.graph.nodes[0].label, "第 2 页");
    }

    #[test]
    fn imported_pages_go_after_a_board_in_use_and_leave_it_alone() {
        let mut live = Live::new();
        let drawn = with(&live.graph, "T 「已有内容」");
        live.change_board(drawn.clone(), json!({ "n": 1 }));
        live.add_pages(vec![(page_board(1), "asset:a.jpg".into())], &Note::plain("导入了"));
        assert_eq!((live.pages(), live.page()), (2, 1));
        assert_eq!(live.lines.last().unwrap()["page"], 1);
        assert!(!live.can_undo(), "an import is not undone with the board's edits");
        assert!(live.steps.is_empty());
        let boards = live.board_pages();
        assert_eq!(boards[0].0.nodes, drawn.nodes);
        assert_eq!(boards[0].1, json!({ "n": 1 }));
        assert_eq!(live.board_images(), [None, Some("asset:a.jpg".to_string())]);
        assert_eq!(live.title(), "已有内容", "the first board still names it");
        // Marking the page records a step that holds the whole page, so a replay can show it.
        let marked = with(&live.graph, "M n1 高亮");
        live.change_board(marked.clone(), Value::Null);
        assert_eq!(live.steps.len(), 1);
        assert_eq!(replay(&live.steps).nodes, marked.nodes);
        assert_eq!(replay(&live.steps).underlay(), Some((900.0, 1200.0)));
    }

    fn said_by(graph: &Graph) -> Vec<Option<&str>> {
        graph.nodes.iter().map(|n| alcyo_board::speakers::by_of_node(n)).collect()
    }

    fn board_by(who: &str) -> Graph {
        let mut graph = page_board(1);
        graph.nodes[0].extra.insert("by".into(), json!(who));
        graph.nodes[0].ink = "ink".into();
        graph
    }

    #[test]
    fn the_same_voice_keeps_its_name_and_new_ones_avoid_names_in_use() {
        let mut live = Live::new();
        live.add_line(Some("A"), "手写的名字", None);
        let (one, other) = ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        assert_eq!(live.identify_voice(&one, 3.0, None).unwrap(), "B");
        assert_eq!(live.identify_voice(&other, 3.0, None).unwrap(), "C");
        assert_eq!(live.identify_voice(&one, 3.0, None).unwrap(), "B");
        assert_eq!(live.speakers(), ["A", "B", "C"]);
    }

    #[test]
    fn a_new_name_reaches_every_page_step_and_undo_and_stays_with_the_voice() {
        let mut live = Live::new();
        let print = [1.0, 0.0];
        let name = live.identify_voice(&print, 3.0, None).unwrap();
        live.add_line(Some(&name), "网关", None);
        live.model_drew(&[Spoken::from((Some(name.clone()), "网关".into()))], board_by(&name), Value::Null, "", 1);
        live.add_pages(vec![(board_by("别人"), "asset:a.jpg".into())], &Note::plain("导入了"));
        // On this page the old name is only in the replay record and the undo stack.
        live.change_board(board_by(&name), Value::Null);
        live.change_board(board_by("别人"), Value::Null);
        assert_eq!(live.page(), 1);
        assert_eq!(live.speaker_impact(&name), (1, 1, 1));

        live.rename_speaker(&name, "李工");

        assert_eq!(live.lines[0]["speaker"], "李工");
        assert_eq!(live.history[0]["speaker"], "李工");
        assert_eq!(live.speaker_impact("李工"), (1, 1, 1));
        assert_eq!(live.speaker_impact(&name), (0, 0, 0));
        let written = serde_json::to_string(&live.snapshot()).unwrap();
        assert!(!written.contains(&format!("\"by\":\"{name}\"")), "{written}");
        assert!(written.contains("\"by\":\"别人\""));
        let snap = live.undo().unwrap();
        assert_eq!(said_by(&snap.graph), [Some("李工")]);
        live.go_to_page(0);
        assert_eq!(said_by(&live.graph), [Some("李工")]);
        // The same voice now answers to the new name.
        assert_eq!(live.identify_voice(&print, 3.0, None).unwrap(), "李工");
    }

    #[test]
    fn taking_a_name_in_use_joins_the_two_voices() {
        let mut live = Live::new();
        let (a, b) = (live.identify_voice(&[1.0, 0.0], 3.0, None).unwrap(), live.identify_voice(&[0.0, 1.0], 3.0, None).unwrap());
        live.add_line(Some(&a), "一", None);
        live.add_line(Some(&b), "二", None);
        live.rename_speaker(&a, &b);
        assert_eq!(live.speakers(), [b.as_str()]);
        assert_eq!(live.speaker_impact(&b).0, 2);
        assert_eq!(live.identify_voice(&[1.0, 1.0], 3.0, None).unwrap(), b);
    }

    #[test]
    fn voices_are_kept_in_the_file_and_left_as_written_until_changed() {
        let mut live = Live::new();
        live.identify_voice(&[1.0, 0.0], 3.0, None).unwrap();
        let written = live.snapshot();
        assert_eq!(written["voices"][0]["name"], "A");
        let mut again = Live::build(written, Origin::Current, 1);
        assert_eq!(again.identify_voice(&[1.0, 0.0], 3.0, None).unwrap(), "A");
        let strange = Live::build(json!({ "id": "x", "voices": [1] }).as_object().unwrap().clone(), Origin::Current, 1).snapshot();
        assert_eq!(strange["voices"], json!([1]));
    }

    #[test]
    fn the_number_of_participants_is_kept_and_caps_the_voices() {
        let mut live = Live::new();
        assert_eq!(live.participants(), None, "Auto unless said");
        live.set_participants(Some(2));
        let mut again = Live::build(live.snapshot(), Origin::Current, 1);
        assert_eq!(again.participants(), Some(2), "kept with the meeting");
        // Two people: a third voice, like neither, is the closer of them.
        assert_eq!(again.identify_voice(&[1.0, 0.0, 0.0], 3.0, None).as_deref(), Some("A"));
        assert_eq!(again.identify_voice(&[0.0, 1.0, 0.0], 3.0, None).as_deref(), Some("B"));
        assert_eq!(again.identify_voice(&[0.3, 0.0, 1.0], 3.0, None).as_deref(), Some("A"));
        assert_eq!(again.speakers(), vec!["A", "B"]);
        // Back to Auto, and nonsense is Auto too.
        again.set_participants(None);
        assert!(!again.snapshot().contains_key("participants"));
        let odd = Live::build(json!({ "id": "x", "participants": 40 }).as_object().unwrap().clone(), Origin::Current, 1);
        assert_eq!(odd.participants(), None);
    }

    #[test]
    fn a_line_that_was_not_drawn_can_be_tried_again() {
        let mut live = Live::new();
        live.add_line(Some("A"), "网关", None);
        live.add_line(None, "还在等", None);
        live.model_failed(&[Spoken::from((Some("A".into()), "网关".into()))]);
        assert_eq!(live.history.len(), 1);
        assert_eq!(live.retry_line(1), None, "only a failed line");
        assert_eq!(live.retry_line(0), Some((Some("A".into()), "网关".into())));
        assert!(live.lines[0]["ms"].is_null() && live.lines[0]["failed"].is_null());
        assert!(live.history.is_empty());
        // Back to waiting, so the reply settles it as usual.
        live.model_drew(&[Spoken::from((Some("A".into()), "网关".into()))], Graph::default(), Value::Null, "", 5);
        assert_eq!(live.lines[0]["ms"], 5);
        assert_eq!(live.history.len(), 1);
    }

    #[test]
    fn a_line_cut_off_by_closing_is_reopened_for_another_try() {
        let mut live = Live::new();
        live.add_line(None, "没等到", None);
        let mut again = Live::build(live.snapshot(), Origin::Current, 1);
        assert_eq!(again.lines[0]["boardFailure"], "cancelled");
        assert!(again.retry_line(0).is_some());
        assert!(again.history.is_empty());
    }

    #[test]
    fn a_given_title_stays() {
        let mut live = Live::new();
        live.set_title("论文");
        live.add_pages(vec![(page_board(1), "asset:a.jpg".into())], &Note::plain("导入了"));
        assert_eq!(live.title(), "论文");
    }

    #[test]
    fn listening_adds_up_and_is_kept() {
        let mut live = Live::new();
        live.add_recorded(1500.0);
        live.add_recorded(2500.4);
        let again = loaded(json!(live.snapshot()), Origin::Current);
        assert_eq!(again.recorded_ms(), 4000.0);
    }

    #[test]
    fn a_fork_keeps_the_page_it_was_drawn_on() {
        let page = with(&Graph::default(), "T 「一」");
        let mut live = loaded(json!({ "id": "m", "graph": page, "page": 0, "pages": [{ "graph": page, "underlay": "asset:abc.jpg" }] }), Origin::Current);
        let two = with(&page, "T 「二」");
        live.change_board(two, Value::Null);
        live.fork_page(1, Value::Null, &Note::plain("另起"));
        assert_eq!(live.board_images(), [Some("asset:abc.jpg".to_string()), Some("asset:abc.jpg".to_string())]);
    }

    #[test]
    fn boards_cover_every_page_with_the_current_one_live() {
        let one = with(&Graph::default(), "T 「一」");
        let two = with(&Graph::default(), "T 「二」");
        let mut live = loaded(json!({ "id": "m", "graph": one, "page": 0, "pages": [{ "graph": one }, { "graph": two }] }), Origin::Current);
        assert_eq!(live.boards().len(), 2);
        let changed = with(&one, "T 「三」");
        live.change_board(changed.clone(), Value::Null);
        assert_eq!(live.boards()[0].nodes, changed.nodes);
        assert_eq!(live.boards()[1].nodes, two.nodes);
        assert_eq!(Live::new().boards().len(), 1);
    }

    #[test]
    fn a_meeting_grows_and_its_steps_replay_to_the_board() {
        let mut live = Live::new();
        live.add_line(Some("A"), "用户调网关", None);
        assert!(live.lines[0]["ms"].is_null());
        let g = with(&live.graph, "E 「用户」 -> 「网关」");
        live.model_drew(&[Spoken::from((Some("A".into()), "用户调网关".into()))], g, json!({ "regionAt": [] }), "E 「用户」 -> 「网关」", 12);
        assert_eq!(live.lines[0]["ms"], 12);
        assert_eq!(live.history[0]["speaker"], "A");
        assert_eq!(live.title(), "用户");
        let raw = live.snapshot();
        assert_eq!(raw["graph"]["nodes"][0]["ink"], "ink");
        let replayed = replay(raw["steps"].as_array().unwrap());
        assert_eq!(replayed.nodes, live.graph.nodes);
        assert_eq!(replayed.edges, live.graph.edges);
    }

    #[test]
    fn undo_and_redo_move_the_board_and_stay_replayable() {
        let mut live = Live::new();
        let first = with(&live.graph, "T 「待办」");
        live.change_board(first.clone(), json!("m1"));
        let mut marked = first.clone();
        let id = marked.nodes[0].id.clone();
        marked.toggle_mark(&id, "星标");
        live.change_board(marked.clone(), json!("m2"));
        let snap = live.undo().unwrap();
        assert_eq!(snap.graph, first);
        assert_eq!(snap.memory, "m1");
        assert!(live.graph.nodes[0].marks.is_empty());
        assert_eq!(replay(&live.steps).nodes, live.graph.nodes);
        assert_eq!(live.redo().unwrap().graph, marked);
        assert_eq!(replay(&live.steps).nodes, live.graph.nodes);
        live.undo();
        live.change_board(with(&live.graph.clone(), "T 「新」"), json!("m3"));
        assert!(!live.can_redo(), "a new change ends what could be redone");
    }

    #[test]
    fn fields_it_does_not_own_survive_a_snapshot() {
        let mut live = loaded(
            json!({ "version": 1, "id": "a-1", "title": "旧", "titled": true, "summary": "摘要", "voices": [1], "future": { "x": 1 },
                    "graph": { "nodes": [{ "id": "n1", "label": "甲", "kind": "topic", "ink": "ink", "by": "A", "aliases": [], "marks": [] }], "edges": [], "groups": [], "nextNode": 2, "nextGroup": 1 },
                    "lines": [{ "text": "甲", "ms": 3, "via": "远程", "audio": { "start": 1 } }], "history": [{ "id": "x", "text": "甲" }] }),
            Origin::Legacy { source_hash: "h".into() },
        );
        assert!(live.read_only.is_none());
        let raw = live.snapshot();
        for key in ["summary", "voices", "future", "titled"] {
            assert!(raw.contains_key(key), "{key}");
        }
        assert_eq!(raw["graph"]["nodes"][0]["by"], "A");
        assert_eq!(raw["lines"][0]["via"], "远程");
        assert_eq!(raw["history"][0]["id"], "x");
        assert_eq!(raw["title"], "旧", "a named meeting keeps its name");
    }

    #[test]
    fn lines_left_waiting_are_marked_when_reopened() {
        let live = loaded(json!({ "id": "a-1", "lines": [{ "text": "没画完" }, { "text": "手动", "manual": true }] }), Origin::Current);
        assert_eq!(live.lines[0]["failed"], true);
        assert_eq!(live.lines[0]["boardFailure"], "cancelled");
        assert!(live.lines[1].get("failed").is_none());
    }

    #[test]
    fn older_meetings_without_steps_get_one_so_replay_still_works() {
        let live = loaded(
            json!({ "id": "a-1", "updatedAt": 9, "graph": { "nodes": [{ "id": "n1", "label": "甲", "kind": "node", "ink": "ink" }], "edges": [], "groups": [], "nextNode": 2, "nextGroup": 1 } }),
            Origin::Current,
        );
        assert_eq!(live.steps.len(), 1);
        assert_eq!(replay(&live.steps).nodes[0].label, "甲");
    }

    #[test]
    fn free_edit_or_unreadable_or_newer_meetings_are_view_only() {
        let g = json!({ "nodes": [], "edges": [], "groups": [], "nextNode": 1, "nextGroup": 1 });
        let cases = [
            json!({ "id": "a", "graph": g, "scene": { "elements": [{ "id": "e" }] } }),
            json!({ "id": "a", "version": 4, "graph": g }),
            json!({ "id": "a", "graph": { "nodes": [{ "id": 7 }] } }),
        ];
        for case in cases {
            assert!(loaded(case.clone(), Origin::Current).read_only.is_some(), "{case}");
        }
        assert!(loaded(json!({ "id": "a", "graph": g, "scene": { "elements": [] } }), Origin::Current).read_only.is_none());
    }

    #[test]
    fn pages_switch_without_losing_either_board() {
        let g = |label: &str| json!({ "nodes": [{ "id": "n1", "label": label, "kind": "topic", "ink": "ink" }], "edges": [], "groups": [], "nextNode": 2, "nextGroup": 1 });
        let mut live = loaded(
            json!({ "id": "a", "graph": g("一"), "page": 0, "pages": [{ "graph": g("一"), "thumb": "t1" }, { "graph": g("二"), "thumb": "t2" }] }),
            Origin::Current,
        );
        assert_eq!(live.pages(), 2);
        let changed = with(&live.graph, "T 「新」");
        live.change_board(changed, Value::Null);
        live.go_to_page(1);
        assert_eq!(live.graph.nodes[0].label, "二");
        let raw = live.snapshot();
        assert_eq!(raw["page"], 1);
        assert_eq!(raw["graph"]["nodes"][0]["label"], "二");
        assert_eq!(raw["pages"][0]["graph"]["nodes"].as_array().unwrap().len(), 2, "the first board keeps its edit");
        assert_eq!(raw["pages"][1]["thumb"], "t2");
    }

    #[test]
    fn one_line_moves_to_someone_else_with_its_voice() {
        let mut live = Live::new();
        for (print, text) in [([1.0, 0.0], "我先说"), ([1.0, 0.0], "接着说"), ([0.7, 0.7], "其实这句是另一个人")] {
            let who = live.identify_voice(&print, 3.0, None);
            live.add_line(who.as_deref(), text, None);
        }
        assert_eq!(live.lines[2]["speaker"], "A", "close enough to be taken for A");
        live.move_line(2, "B");
        assert_eq!(live.lines.iter().map(|l| l["speaker"].as_str().unwrap()).collect::<Vec<_>>(), ["A", "A", "B"]);
        // B now has a voice, so a later line in it goes to B, not A.
        assert_eq!(live.identify_voice(&[0.6, 0.8], 3.0, None).as_deref(), Some("B"));
        live.move_line(9, "C");
        live.move_line(0, "");
        assert_eq!(live.lines[0]["speaker"], "A");
    }

    #[test]
    fn a_saved_meeting_reopens_identically() {
        let path = std::env::temp_dir().join(format!("alcyo-live-{}", std::process::id()));
        let store = Store::new(path.clone());
        let mut live = Live::new();
        live.add_line(None, "用户调网关", None);
        let g = with(&live.graph, "E 「用户」 -> 「网关」\nM 「网关」 星标");
        live.model_drew(&[Spoken::from((None, "用户调网关".into()))], g, json!({ "regionAt": [["n", { "x": 1, "y": 2 }]] }), "ops", 5);
        let id = live.id().to_string();
        let revision = store.save(&id, live.snapshot(), &live.origin.clone(), live.revision).unwrap();
        live.saved(revision);
        let again = Live::from_loaded(store.load(&id).unwrap().unwrap());
        assert_eq!(again.graph, live.graph);
        assert_eq!(again.memory, live.memory);
        assert_eq!(again.lines, live.lines);
        assert_eq!(again.history, live.history);
        assert_eq!(again.steps, live.steps);
        assert_eq!(again.title(), live.title());
        assert_eq!(again.revision, 1);
        let _ = std::fs::remove_dir_all(path);
    }
}
