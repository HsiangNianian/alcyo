//! What on a board came from which lines of the transcript, and how much of the board an edit of those lines
//! redraws (#362).
//!
//! A board change the model drew for a batch of lines records them in its step (`"from": [places]`). An item
//! traces to a line when the step that first put it on the board came from that line, or a later step from that
//! line changed it. Meetings drawn before steps recorded their lines, and lines drawn before then, fall back to
//! the guess the transcript's "jump to the line" makes ([`said_at`]): the line by the same speaker most like the
//! item's words, before the step that first showed it. Such a trace is not [`Traced::confident`].

use crate::graph::{Graph, Kind};
use crate::speakers::{by_of_edge, by_of_node, first_seen, said_at};
use serde_json::{json, Value};
use std::collections::{BTreeSet, HashSet};

/// The items of one board that trace back to some lines.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Traced {
    /// Nodes, in board order.
    pub nodes: Vec<String>,
    /// Links, as (from, to).
    pub edges: Vec<(String, String)>,
    /// False when some line's items were guessed rather than recorded.
    pub confident: bool,
}

impl Traced {
    /// How many items: nodes and links.
    pub fn len(&self) -> usize {
        self.nodes.len() + self.edges.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

fn places(step: &Value) -> Vec<usize> {
    step["from"].as_array().map(|f| f.iter().filter_map(|x| x.as_u64().map(|x| x as usize)).collect()).unwrap_or_default()
}

fn set_ids(step: &Value, part: &str) -> Vec<Value> {
    step.pointer(&format!("/d/{part}/set")).and_then(Value::as_array).cloned().unwrap_or_default()
}

/// The lines as they came in: an edit's words and speaker back to what was heard, gone lines shown, so the guess
/// matches items against what the model was given.
fn as_heard(lines: &[Value]) -> Vec<Value> {
    lines
        .iter()
        .map(|l| {
            let mut l = l.clone();
            if let Some(text) = l["heard"]["text"].as_str().map(str::to_string) {
                l["text"] = json!(text);
            }
            if let Some(speaker) = l["heard"]["speaker"].as_str().map(str::to_string) {
                l["speaker"] = json!(speaker);
            }
            l
        })
        .collect()
}

/// The items of `graph`, board `page` of the meeting with these `lines` and this board's `steps`, that the lines at
/// `picked` drew.
pub fn trace(lines: &[Value], steps: &[Value], graph: &Graph, page: usize, picked: &[usize]) -> Traced {
    let picked: BTreeSet<usize> = picked.iter().copied().filter(|i| lines.get(*i).is_some_and(|l| l["page"].as_u64().unwrap_or(0) as usize == page)).collect();
    let mut out = Traced { confident: true, ..Traced::default() };
    if picked.is_empty() {
        return out;
    }
    // Lines are drawn in order, so once a step records its lines every later line is recorded too.
    let first_recorded = steps.iter().flat_map(places).min();
    let recorded = |i: usize| first_recorded.is_some_and(|f| i >= f);
    let mut nodes: HashSet<String> = HashSet::new();
    let mut edges: HashSet<(String, String)> = HashSet::new();
    let mut seen_nodes: HashSet<String> = HashSet::new();
    let mut seen_edges: HashSet<(String, String)> = HashSet::new();
    for step in steps {
        let from = places(step);
        let ours = from.iter().any(|i| picked.contains(i));
        for n in set_ids(step, "nodes") {
            let Some(id) = n["id"].as_str() else { continue };
            let new = seen_nodes.insert(id.to_string());
            if ours && (new || !from.is_empty()) {
                nodes.insert(id.to_string());
            }
        }
        for e in set_ids(step, "edges") {
            let (Some(a), Some(b)) = (e["from"].as_str(), e["to"].as_str()) else { continue };
            let key = (a.to_string(), b.to_string());
            let new = seen_edges.insert(key.clone());
            if ours && (new || !from.is_empty()) {
                edges.insert(key);
            }
        }
    }
    let guessed: Vec<usize> = picked.iter().copied().filter(|i| !recorded(*i)).collect();
    if !guessed.is_empty() {
        out.confident = false;
        let heard = as_heard(lines);
        for n in &graph.nodes {
            let Some(who) = by_of_node(n) else { continue };
            let mut names = vec![n.label.as_str()];
            names.extend(n.aliases.iter().map(String::as_str));
            if said_at(&heard, who, &names, page, first_seen(steps, &n.id)).is_some_and(|i| guessed.contains(&i)) {
                nodes.insert(n.id.clone());
            }
        }
        for e in &graph.edges {
            if by_of_edge(e).is_some() && (nodes.contains(&e.from) && nodes.contains(&e.to)) {
                edges.insert((e.from.clone(), e.to.clone()));
            }
        }
    }
    // Only what is still there, never imported text, in board order.
    out.nodes = graph.nodes.iter().filter(|n| nodes.contains(&n.id) && n.kind != Kind::Region).map(|n| n.id.clone()).collect();
    out.edges = graph.edges.iter().filter(|e| edges.contains(&(e.from.clone(), e.to.clone()))).map(|e| (e.from.clone(), e.to.clone())).collect();
    out
}

/// The part of the board a node belongs to: its highest topic, or its highest ancestor on a board without topics.
pub fn section_of(graph: &Graph, id: &str) -> Option<String> {
    let mut at = graph.nodes.iter().find(|n| n.id == id)?;
    let mut topic = (at.kind == Kind::Topic).then(|| at.id.clone());
    let mut hops = 0;
    while let Some(parent) = at.parent.as_deref().and_then(|p| graph.nodes.iter().find(|n| n.id == p)) {
        at = parent;
        if at.kind == Kind::Topic {
            topic = Some(at.id.clone());
        }
        hops += 1;
        if hops > graph.nodes.len() {
            break;
        }
    }
    Some(topic.unwrap_or_else(|| at.id.clone()))
}

/// The sections these items are in, each once, in the order first met. A link counts in its ends' sections.
pub fn sections(graph: &Graph, traced: &Traced) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    let ends = traced.edges.iter().flat_map(|(a, b)| [a, b]);
    for id in traced.nodes.iter().chain(ends) {
        if let Some(s) = section_of(graph, id) {
            if !out.contains(&s) {
                out.push(s);
            }
        }
    }
    out
}

/// What kind of change was made to the lines.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    /// The words of a line.
    Words,
    /// Who said a line.
    Speaker,
    /// Lines joined into one.
    Merge,
    /// Lines taken out.
    Delete,
}

/// How much of the board an edit redraws.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Scope {
    /// Nothing to do on the board.
    Nothing,
    /// The items change hands, with no model: [`set_owner`].
    Owner,
    /// The lines drew nothing: the corrected words are drawn as a new turn.
    NewTurn,
    /// Only the traced items.
    Items,
    /// These sections, each rewritten.
    Section(Vec<String>),
    /// The whole board, asked first.
    Board,
}

impl Scope {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Nothing => "nothing",
            Self::Owner => "owner",
            Self::NewTurn => "turn",
            Self::Items => "items",
            Self::Section(_) => "section",
            Self::Board => "board",
        }
    }
}

/// At most this many items in one section are redrawn on their own.
pub const ITEMS_MOST: usize = 3;
/// At most this many items, in at most [`SECTIONS_MOST`] sections and under [`SHARE_PERCENT`] of the board, are
/// redrawn as their sections; more is the whole board.
pub const SECTION_MOST: usize = 12;
pub const SECTIONS_MOST: usize = 2;
pub const SHARE_PERCENT: usize = 40;

/// How much of `graph` an edit of lines redraws, from how many of its items trace back to them and how spread they
/// are. A guessed trace may have missed items, so it redraws one step more.
pub fn scope(change: Change, traced: &Traced, graph: &Graph) -> Scope {
    match change {
        Change::Speaker => return if traced.is_empty() { Scope::Nothing } else { Scope::Owner },
        Change::Delete if traced.is_empty() => return Scope::Nothing,
        Change::Words | Change::Merge if traced.is_empty() => return Scope::NewTurn,
        _ => {}
    }
    let found = sections(graph, traced);
    let count = traced.len();
    let all = (graph.nodes.iter().filter(|n| n.kind != Kind::Region).count() + graph.edges.len()).max(1);
    let section = || Scope::Section(found.clone());
    let picked = if count <= ITEMS_MOST && found.len() <= 1 {
        if traced.confident { Scope::Items } else { section() }
    } else if count <= SECTION_MOST && found.len() <= SECTIONS_MOST && count * 100 < all * SHARE_PERCENT {
        if traced.confident { section() } else { Scope::Board }
    } else {
        Scope::Board
    };
    // A board with no sections to speak of has nothing smaller to rewrite than its items.
    match picked {
        Scope::Section(s) if s.is_empty() => Scope::Items,
        other => other,
    }
}

/// Give these items to someone else: who raised them, and a label written as "who：…" (or "who: …").
pub fn set_owner(graph: &mut Graph, traced: &Traced, from: &str, to: &str) {
    let relabel = |label: &mut String| {
        for colon in ["：", ": ", ":"] {
            if let Some(rest) = label.strip_prefix(&format!("{from}{colon}")) {
                *label = format!("{to}{colon}{rest}");
                return;
            }
        }
    };
    for n in graph.nodes.iter_mut().filter(|n| traced.nodes.contains(&n.id)) {
        n.extra.insert("by".into(), json!(to));
        relabel(&mut n.label);
    }
    for e in graph.edges.iter_mut().filter(|e| traced.edges.contains(&(e.from.clone(), e.to.clone()))) {
        e.extra.insert("by".into(), json!(to));
    }
}

/// Take these items off the board, as deleting their lines "and what they drew" asks. What hangs under a node
/// that goes is kept, under that node's parent: it may have come from other lines.
pub fn remove_items(graph: &mut Graph, traced: &Traced) {
    let gone: HashSet<&String> = traced.nodes.iter().collect();
    let parent_of = |id: &str, graph: &Graph| graph.nodes.iter().find(|n| n.id == id).and_then(|n| n.parent.clone());
    let lifted: Vec<(String, Option<String>)> = graph
        .nodes
        .iter()
        .filter(|n| !gone.contains(&n.id) && n.parent.as_ref().is_some_and(|p| gone.contains(p)))
        .map(|n| {
            let mut up = n.parent.clone();
            while let Some(p) = up.clone().filter(|p| gone.contains(p)) {
                up = parent_of(&p, graph);
            }
            (n.id.clone(), up)
        })
        .collect();
    for (id, up) in lifted {
        if let Some(n) = graph.nodes.iter_mut().find(|n| n.id == id) {
            n.parent = up;
        }
    }
    graph.nodes.retain(|n| !gone.contains(&n.id));
    graph.edges.retain(|e| !gone.contains(&e.from) && !gone.contains(&e.to) && !traced.edges.contains(&(e.from.clone(), e.to.clone())));
}

/// One section of a board: its top node and everything under it, with the links between them.
pub fn subtree(graph: &Graph, top: &str) -> Graph {
    let mut ids: HashSet<String> = HashSet::from([top.to_string()]);
    loop {
        let before = ids.len();
        for n in &graph.nodes {
            if n.parent.as_ref().is_some_and(|p| ids.contains(p)) {
                ids.insert(n.id.clone());
            }
        }
        if ids.len() == before {
            break;
        }
    }
    let mut part = graph.clone();
    part.nodes.retain(|n| ids.contains(&n.id));
    part.edges.retain(|e| ids.contains(&e.from) && ids.contains(&e.to));
    part.extra.remove("annotations");
    part.extra.remove("underlay");
    part
}

/// Put a rewritten section back in place of the old one: the old section's nodes and inner links go, the rewrite's
/// come in where the old top was, links to the rest of the board stay while both ends do. New nodes of the rewrite
/// are numbered on from the board, so no id names two things.
pub fn splice(graph: &Graph, old: &Graph, rewritten: &Graph) -> Graph {
    let mut out = graph.clone();
    let was: HashSet<&str> = old.nodes.iter().map(|n| n.id.as_str()).collect();
    let mut renamed: Vec<(String, String)> = vec![];
    for n in &rewritten.nodes {
        let clashes = !was.contains(n.id.as_str()) && graph.nodes.iter().any(|m| m.id == n.id);
        if clashes {
            let id = format!("n{}", out.next_node);
            out.next_node += 1;
            renamed.push((n.id.clone(), id));
        } else if let Some(num) = n.id.strip_prefix('n').and_then(|x| x.parse::<u32>().ok()) {
            out.next_node = out.next_node.max(num + 1);
        }
    }
    let fresh = |id: &str| renamed.iter().find(|(a, _)| a == id).map_or_else(|| id.to_string(), |(_, b)| b.clone());
    let at = out.nodes.iter().position(|n| was.contains(n.id.as_str())).unwrap_or(out.nodes.len());
    out.nodes.retain(|n| !was.contains(n.id.as_str()));
    let at = at.min(out.nodes.len());
    let incoming: Vec<_> = rewritten
        .nodes
        .iter()
        .map(|n| {
            let mut n = n.clone();
            n.id = fresh(&n.id);
            n.parent = n.parent.as_deref().map(fresh);
            n
        })
        .collect();
    out.nodes.splice(at..at, incoming);
    let present: HashSet<String> = out.nodes.iter().map(|n| n.id.clone()).collect();
    out.edges.retain(|e| !(was.contains(e.from.as_str()) && was.contains(e.to.as_str())));
    for e in &rewritten.edges {
        let mut e = e.clone();
        e.from = fresh(&e.from);
        e.to = fresh(&e.to);
        out.edges.push(e);
    }
    out.edges.retain(|e| present.contains(&e.from) && present.contains(&e.to));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::parse_ops;
    use crate::steps::diff;

    /// A board drawn line by line, each line's ops recorded in a step as the app records them.
    fn drawn(said: &[(&str, &str)]) -> (Vec<Value>, Vec<Value>, Graph) {
        let (mut lines, mut steps, mut g) = (vec![], vec![], Graph::default());
        for (i, (who, ops)) in said.iter().enumerate() {
            let before = g.clone();
            g.apply(&parse_ops(ops));
            for n in g.nodes.iter_mut().filter(|n| !before.nodes.iter().any(|m| m.id == n.id)) {
                n.extra.insert("by".into(), json!(who));
            }
            lines.push(json!({ "text": ops, "speaker": who, "page": 0, "ms": 1 }));
            if let Some(d) = diff(&before, &g) {
                steps.push(json!({ "t": i, "line": i + 1, "d": d, "from": [i] }));
            }
        }
        (lines, steps, g)
    }

    #[test]
    fn items_trace_to_the_line_that_drew_or_changed_them() {
        let (lines, steps, g) = drawn(&[("D", "T 「两个选项」\nL 「两个选项」 「选项一」"), ("E", "L 「两个选项」 「删掉选项一」"), ("B", "M 「选项一」 划掉")]);
        let t = trace(&lines, &steps, &g, 0, &[1]);
        assert_eq!(t.nodes, vec!["n3".to_string()]);
        assert!(t.confident);
        let t = trace(&lines, &steps, &g, 0, &[2]);
        assert_eq!(t.nodes, vec!["n2".to_string()], "the mark B put on it");
        assert!(trace(&lines, &steps, &g, 1, &[1]).is_empty(), "that line was said on another board");
    }

    #[test]
    fn older_meetings_are_guessed_from_who_said_what() {
        let (mut lines, mut steps, g) = drawn(&[("D", "T 「结算」"), ("E", "L 「结算」 「按比例分成」")]);
        for s in &mut steps {
            s.as_object_mut().unwrap().remove("from");
        }
        lines[1]["text"] = json!("那就按比例分成吧");
        lines[1]["heard"] = json!({ "text": "那就按比例分成吧" });
        let t = trace(&lines, &steps, &g, 0, &[1]);
        assert_eq!(t.nodes, vec!["n2".to_string()]);
        assert!(!t.confident);
    }

    #[test]
    fn the_scope_grows_with_the_items_and_their_spread() {
        let mut g = Graph::default();
        g.apply(&parse_ops("T 「甲」\nT 「乙」\nT 「丙」"));
        for (t, n) in [("甲", 8), ("乙", 8), ("丙", 8)] {
            for k in 0..n {
                g.apply(&parse_ops(&format!("L 「{t}」 「{t}{k}」")));
            }
        }
        let ids = |labels: &[&str]| Traced { nodes: labels.iter().map(|l| g.nodes.iter().find(|n| n.label == *l).unwrap().id.clone()).collect(), edges: vec![], confident: true };
        assert_eq!(scope(Change::Words, &ids(&["甲0", "甲1"]), &g), Scope::Items);
        assert_eq!(scope(Change::Words, &ids(&["甲0", "乙1"]), &g), Scope::Section(vec!["n1".into(), "n2".into()]));
        assert_eq!(scope(Change::Merge, &ids(&["甲0", "甲1", "甲2", "甲3", "甲4"]), &g), Scope::Section(vec!["n1".into()]));
        assert_eq!(scope(Change::Words, &ids(&["甲0", "乙0", "丙0"]), &g), Scope::Board, "three sections");
        let many: Vec<String> = (0..8).map(|k| format!("甲{k}")).chain((0..5).map(|k| format!("乙{k}"))).collect();
        let many: Vec<&str> = many.iter().map(String::as_str).collect();
        assert_eq!(scope(Change::Words, &ids(&many), &g), Scope::Board, "more than twelve");
        let mut guessed = ids(&["甲0"]);
        guessed.confident = false;
        assert_eq!(scope(Change::Words, &guessed, &g), Scope::Section(vec!["n1".into()]), "a guess redraws one step more");
        assert_eq!(scope(Change::Words, &Traced::default(), &g), Scope::NewTurn);
        assert_eq!(scope(Change::Delete, &Traced::default(), &g), Scope::Nothing);
        assert_eq!(scope(Change::Speaker, &ids(&["甲0"]), &g), Scope::Owner);
        // Forty percent of a small board is the whole board.
        let mut small = Graph::default();
        small.apply(&parse_ops("T 「甲」\nL 「甲」 「一」\nL 「甲」 「二」\nL 「甲」 「三」\nL 「甲」 「四」"));
        let four = Traced { nodes: vec!["n2".into(), "n3".into(), "n4".into(), "n5".into()], edges: vec![], confident: true };
        assert_eq!(scope(Change::Words, &four, &small), Scope::Board);
    }

    #[test]
    fn items_change_hands_and_their_labels_follow() {
        let mut g = Graph::default();
        g.apply(&parse_ops("N 「D：忘记了」\nN 「别的」"));
        g.nodes[0].extra.insert("by".into(), json!("D"));
        let t = Traced { nodes: vec!["n1".into()], edges: vec![], confident: true };
        set_owner(&mut g, &t, "D", "B");
        assert_eq!(g.nodes[0].label, "B：忘记了");
        assert_eq!(g.nodes[0].extra["by"], "B");
        assert!(g.nodes[1].extra.get("by").is_none());
    }

    #[test]
    fn removing_items_keeps_what_hangs_under_them() {
        let mut g = Graph::default();
        g.apply(&parse_ops("T 「甲」\nL 「甲」 「乙」\nL 「乙」 「丙」\nN 「丁」\nE 「乙」 -> 「丁」"));
        remove_items(&mut g, &Traced { nodes: vec!["n2".into()], edges: vec![], confident: true });
        assert!(g.nodes.iter().all(|n| n.label != "乙"));
        assert_eq!(g.nodes.iter().find(|n| n.label == "丙").unwrap().parent.as_deref(), Some("n1"));
        assert!(g.edges.is_empty());
    }

    #[test]
    fn a_rewritten_section_goes_back_in_its_place() {
        let mut g = Graph::default();
        g.apply(&parse_ops("T 「甲」\nL 「甲」 「一」\nT 「乙」\nL 「乙」 「二」\nE 「一」 -> 「二」"));
        let old = subtree(&g, "n1");
        assert_eq!(old.nodes.len(), 2);
        let mut new = old.clone();
        new.apply(&parse_ops("R 「一」 「一改」\nL 「甲」 「新的」"));
        // The rewrite numbered its new node from the section, which clashes with the board.
        assert_eq!(new.nodes.last().unwrap().id, "n5");
        let mut clash = new.clone();
        clash.nodes.last_mut().unwrap().id = "n3".into();
        let out = splice(&g, &old, &clash);
        assert_eq!(out.nodes.iter().map(|n| n.label.as_str()).collect::<Vec<_>>(), vec!["甲", "一改", "新的", "乙", "二"]);
        let added = out.nodes.iter().find(|n| n.label == "新的").unwrap();
        assert_ne!(added.id, "n3", "renumbered, as n3 is 乙");
        assert_eq!(added.parent.as_deref(), Some("n1"));
        assert_eq!(out.edges.len(), 1, "the link to the other section stays");
    }
}
