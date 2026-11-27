//! Redrawing what edited lines of the transcript put on the board (#362).
//!
//! The scope comes from the board ([`alcyo_board::trace::scope`]): the traced items alone are asked about in an
//! ordinary turn with the correction as its hint, and whatever the reply does outside them is dropped
//! ([`Graph::keep_within`]); a section is rewritten as a consolidation of that topic alone and put back in place
//! ([`splice`]); the whole board is a consolidation from the corrected transcript. Lines that drew nothing are
//! drawn again as a new turn. Changing who said a line, and deleting lines with what they drew, need no model.

use crate::consolidate::{self, Style, When};
use crate::{propose_with, sound, Guidance, Provider};
use alcyo_board::graph::{board_is_english, parse_ops, Graph, Utterance};
use alcyo_board::speakers::attribute;
use alcyo_board::trace::{remove_items, scope, set_owner, splice, subtree, trace, Change, Scope, Traced};
use anyhow::{bail, Result};
use serde_json::Value;
use std::collections::HashSet;

const ITEMS: [&str; 2] = [include_str!("../skills/board-redraw/items.zh.md"), include_str!("../skills/board-redraw/items.en.md")];
const SECTION: [&str; 2] = [include_str!("../skills/board-redraw/section.zh.md"), include_str!("../skills/board-redraw/section.en.md")];

/// One redraw to make: an edit of the transcript, on one board.
#[derive(Clone, Debug)]
pub struct Redraw {
    /// The meeting it is for, from `Session::meeting`: another meeting opened meanwhile drops it.
    pub meeting: u64,
    /// The edit's number, which the answer carries back to join its undo position.
    pub edit: u64,
    /// The board, by page.
    pub page: usize,
    /// Whether that board is the one on screen, which the session draws on: its answer is brought up to what was
    /// drawn meanwhile.
    pub on_screen: bool,
    pub change: Change,
    pub scope: Scope,
    /// The board as it was when the edit was made.
    pub board: Graph,
    pub traced: Traced,
    /// The edited lines as they were.
    pub before: Vec<Utterance>,
    /// The edited lines now: one for a merge.
    pub after: Vec<Utterance>,
    /// The lines said just before, for a turn's context.
    pub context: Vec<Utterance>,
    /// What was said on this board, corrected, for rewriting a section or the board.
    pub history: Vec<Utterance>,
}

/// What a redraw came back with.
#[derive(Clone, Debug)]
pub struct Redrawn {
    pub graph: Graph,
    /// What the model was asked, and what it answered, for the diagnostic bundle.
    pub asked: String,
    pub answer: String,
    pub ms: u128,
    pub tokens_in: u64,
    pub tokens_out: u64,
}

fn said(u: &Utterance) -> String {
    u.speaker.as_ref().map_or_else(|| u.text.clone(), |s| format!("{s}：{}", u.text))
}

/// The correction, as the model is told it.
pub fn describe_change(req: &Redraw, english: bool) -> String {
    let join = |all: &[Utterance]| all.iter().map(said).collect::<Vec<_>>().join(" / ");
    match (req.change, english) {
        (Change::Merge, false) => format!("这几句其实是一句话，已经合并：\n  原来：{}\n  合并后：{}", join(&req.before), join(&req.after)),
        (Change::Merge, true) => format!("These lines were one sentence, now joined:\n  Before: {}\n  Joined: {}", join(&req.before), join(&req.after)),
        (_, false) => format!("这句话听错了，已经改正：\n  原来听到：{}\n  改正后：{}", join(&req.before), join(&req.after)),
        (_, true) => format!("This line was misheard and has been corrected:\n  Heard: {}\n  Corrected: {}", join(&req.before), join(&req.after)),
    }
}

/// The traced items, one a line: id, words, and the point they hang under.
fn describe_items(req: &Redraw, english: bool) -> String {
    let label = |id: &str| req.board.nodes.iter().find(|n| n.id == id).map(|n| n.label.clone()).unwrap_or_default();
    let mut lines: Vec<String> = req
        .traced
        .nodes
        .iter()
        .filter_map(|id| req.board.nodes.iter().find(|n| &n.id == id))
        .map(|n| match n.parent.as_deref().map(label).filter(|p| !p.is_empty()) {
            Some(p) if english => format!("{}「{}」 (under 「{p}」)", n.id, n.label),
            Some(p) => format!("{}「{}」（在「{p}」下）", n.id, n.label),
            None => format!("{}「{}」", n.id, n.label),
        })
        .collect();
    lines.extend(req.traced.edges.iter().map(|(a, b)| format!("{a}({}) -> {b}({})", label(a), label(b))));
    lines.join("\n")
}

/// The hint an items redraw is asked with.
pub fn items_hint(req: &Redraw, english: bool) -> String {
    ITEMS[usize::from(english)].trim_end().replace("{change}", &describe_change(req, english)).replace("{items}", &describe_items(req, english))
}

/// What an items redraw may change, and where it may hang new points.
pub fn allowed(req: &Redraw) -> (HashSet<String>, HashSet<String>) {
    let editable: HashSet<String> = req.traced.nodes.iter().cloned().collect();
    let anchors: HashSet<String> = req.board.nodes.iter().filter(|n| editable.contains(&n.id)).filter_map(|n| n.parent.clone()).collect();
    (editable, anchors)
}

/// The reply of an items redraw applied to the board as asked, kept to its items.
pub fn apply_items(req: &Redraw, ops: &str) -> Graph {
    let (editable, anchors) = allowed(req);
    let kept = req.board.keep_within(parse_ops(ops), &editable, &anchors);
    let mut graph = req.board.clone();
    graph.apply(&kept);
    attribute(&req.board, &mut graph, &req.after);
    graph
}

fn joined(after: &[Utterance]) -> Utterance {
    match after {
        [one] => one.clone(),
        many => Utterance { speaker: many.first().and_then(|u| u.speaker.clone()), text: many.iter().map(|u| u.text.as_str()).collect::<Vec<_>>().join(" "), pointing: vec![] },
    }
}

/// Ask the model for a redraw and give the board it comes to, for the board as asked. A section or board that
/// cannot be consolidated (a flow chart) is redrawn as its items.
pub fn redraw(provider: &Provider, req: &Redraw, guidance: Guidance, glossary: &sound::Glossary, references: &[(u32, String)]) -> Result<Redrawn> {
    let _purpose = alcyo_diag::Purpose::set("redraw");
    let started = std::time::Instant::now();
    let next = joined(&req.after);
    let english = board_is_english(&req.context, &next);
    let turn = |hint: Option<String>| -> Result<Redrawn> {
        let guidance = Guidance { hint: hint.is_some(), ..guidance };
        let reply = propose_with(provider, &req.board, &req.context, &next, guidance, hint.as_deref(), glossary, references)?;
        let graph = if hint.is_some() {
            apply_items(req, &reply.ops)
        } else {
            let mut graph = req.board.clone();
            graph.apply(&parse_ops(&reply.ops));
            attribute(&req.board, &mut graph, std::slice::from_ref(&next));
            graph
        };
        Ok(Redrawn { graph, asked: hint.unwrap_or_default(), answer: reply.ops, ms: started.elapsed().as_millis(), tokens_in: reply.tokens_in, tokens_out: reply.tokens_out })
    };
    let rewrite = |board: &Graph, system: String| -> Result<Option<consolidate::Consolidated>> { consolidate::consolidate(provider, board, &req.history, english, Some(&system), "high", When::Asked) };
    match &req.scope {
        Scope::NewTurn => turn(None),
        Scope::Items => turn(Some(items_hint(req, english))),
        Scope::Section(tops) => {
            let mut graph = req.board.clone();
            let (mut asked, mut answer) = (String::new(), String::new());
            let (mut tokens_in, mut tokens_out) = (0, 0);
            let note = SECTION[usize::from(english)].replace("{change}", &describe_change(req, english));
            for top in tops {
                let old = subtree(&graph, top);
                let system = format!("{}{note}", Style::General.system(english));
                let Some(done) = rewrite(&old, system.clone())? else {
                    return turn(Some(items_hint(req, english)));
                };
                graph = splice(&graph, &old, &done.graph);
                asked.push_str(&system);
                answer.push_str(&done.outline);
                answer.push('\n');
                tokens_in += done.tokens_in;
                tokens_out += done.tokens_out;
            }
            Ok(Redrawn { graph, asked, answer, ms: started.elapsed().as_millis(), tokens_in, tokens_out })
        }
        Scope::Board => {
            let system = Style::General.system(english);
            match rewrite(&req.board, system.clone())? {
                Some(done) => Ok(Redrawn { graph: done.graph, asked: system, answer: done.outline, ms: started.elapsed().as_millis(), tokens_in: done.tokens_in, tokens_out: done.tokens_out }),
                None => turn(Some(items_hint(req, english))),
            }
        }
        Scope::Nothing | Scope::Owner => bail!("this edit needs no model"),
    }
}

/// The scope an edit on `board` really gets: a board that is not an outline has no section to rewrite, so its
/// redraw stays with the items.
pub fn fitted(scope: Scope, board: &Graph) -> Scope {
    match scope {
        Scope::Section(_) | Scope::Board if !consolidate::applies_to(board) => Scope::Items,
        other => other,
    }
}

/// One board of a meeting, as an edit's plan reads it.
pub struct Board {
    pub page: usize,
    pub graph: Graph,
    pub steps: Vec<Value>,
}

fn utterance(line: &Value) -> Utterance {
    Utterance { speaker: line["speaker"].as_str().filter(|s| !s.is_empty()).map(str::to_string), text: line["text"].as_str().unwrap_or_default().to_string(), pointing: vec![] }
}

fn gone(line: &Value) -> bool {
    line["deleted"].as_bool().unwrap_or(false) || !line["mergedInto"].is_null()
}

fn said_on(line: &Value, page: usize) -> bool {
    !line["manual"].as_bool().unwrap_or(false) && !gone(line) && line["page"].as_u64().unwrap_or(0) as usize == page
}

/// What an edit of lines changes on each board (#362), after the edit is made: one [`Redraw`] for each board an
/// edited line drew on (or was said on, for lines that drew nothing), with its scope. Shared by every window, so a
/// window only shows it. `touched` and `before` are the edit's lines, by place, as they were before it; `line` the
/// one that holds the result. Scopes that need no model ([`Scope::Owner`], [`Scope::Nothing`]) come back too, for
/// [`without_model`].
#[allow(clippy::too_many_arguments)]
pub fn plan(lines: &[Value], boards: &[Board], change: Change, line: usize, touched: &[usize], before: &[Value], on_screen: usize, meeting: u64, edit: u64) -> Vec<Redraw> {
    let before: Vec<Utterance> = before.iter().filter(|l| !gone(l)).map(utterance).collect();
    let after: Vec<Utterance> = lines.get(line).filter(|l| !gone(l)).map(utterance).into_iter().collect();
    let pages_said: HashSet<usize> = touched.iter().filter_map(|i| lines.get(*i)).map(|l| l["page"].as_u64().unwrap_or(0) as usize).collect();
    let mut out = vec![];
    for board in boards {
        let traced = trace(lines, &board.steps, &board.graph, board.page, touched);
        if traced.is_empty() && !pages_said.contains(&board.page) {
            continue;
        }
        let scope = fitted(scope(change, &traced, &board.graph), &board.graph);
        if scope == Scope::Nothing {
            continue;
        }
        // The lines said before on this board, for a turn's context, and all that was said on it for a rewrite.
        let earlier: Vec<Utterance> = lines[..line.min(lines.len())].iter().filter(|l| said_on(l, board.page)).map(utterance).collect();
        let context = earlier[earlier.len().saturating_sub(crate::TURN_CONTEXT_LINES)..].to_vec();
        let history: Vec<Utterance> = lines.iter().filter(|l| said_on(l, board.page) && !l["ms"].is_null()).map(utterance).collect();
        out.push(Redraw {
            meeting,
            edit,
            page: board.page,
            on_screen: board.page == on_screen,
            change,
            scope,
            board: board.graph.clone(),
            traced,
            before: before.clone(),
            after: after.clone(),
            context,
            history,
        });
    }
    out
}

/// The board after an edit that needs no model: the items given to the line's new speaker, or deleted with their
/// lines. `None` when the model has to redraw it.
pub fn without_model(req: &Redraw) -> Option<Graph> {
    let mut graph = req.board.clone();
    match (&req.scope, req.change) {
        (Scope::Owner, _) => {
            let from = req.before.first().and_then(|u| u.speaker.clone()).unwrap_or_default();
            let to = req.after.first().and_then(|u| u.speaker.clone()).unwrap_or_default();
            set_owner(&mut graph, &req.traced, &from, &to);
        }
        (_, Change::Delete) => remove_items(&mut graph, &req.traced),
        _ => return None,
    }
    Some(graph)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    fn board() -> Graph {
        let mut g = Graph::default();
        g.apply(&parse_ops("T 「两个选项」\nL 「两个选项」 「选项一」\nL 「两个选项」 「删掉选项一」\nT 「待定」\nL 「待定」 「结算」"));
        g
    }

    fn request(scope: Scope) -> Redraw {
        let u = |who: &str, text: &str| Utterance { speaker: Some(who.into()), text: text.into(), pointing: vec![] };
        Redraw {
            meeting: 1,
            edit: 1,
            page: 0,
            on_screen: true,
            change: Change::Words,
            scope,
            board: board(),
            traced: Traced { nodes: vec!["n3".into()], edges: vec![], confident: true },
            before: vec![u("E", "那就删掉选项一。")],
            after: vec![u("E", "那就先留着选项一。")],
            context: vec![u("B", "视频没提出来，人就错了。")],
            history: vec![u("D", "两个选项，选项一按比例分成。"), u("E", "那就先留着选项一。")],
        }
    }

    #[test]
    fn the_items_request_names_the_correction_and_the_items() {
        let hint = items_hint(&request(Scope::Items), false);
        assert!(hint.contains("原来听到：E：那就删掉选项一。"), "{hint}");
        assert!(hint.contains("改正后：E：那就先留着选项一。"), "{hint}");
        assert!(hint.contains("n3「删掉选项一」（在「两个选项」下）"), "{hint}");
        let (editable, anchors) = allowed(&request(Scope::Items));
        assert_eq!((editable.len(), anchors.iter().next().map(String::as_str)), (1, Some("n1")));
    }

    #[test]
    fn an_items_redraw_drops_what_it_may_not_touch() {
        let asked = Arc::new(Mutex::new(String::new()));
        let seen = asked.clone();
        let provider = Provider::Scripted(Arc::new(move |said: &str| {
            *seen.lock().unwrap() = said.to_string();
            Ok("R n3 「先留着选项一」\nD n5\nR n1 「改了主题」\nL n1 「新加一点」".into())
        }));
        let req = request(Scope::Items);
        let done = redraw(&provider, &req, provider.default_guidance(), &sound::Glossary::default(), &[]).unwrap();
        assert!(asked.lock().unwrap().contains("只改这几项"), "the hint reaches the model");
        let label = |id: &str| done.graph.nodes.iter().find(|n| n.id == id).map(|n| n.label.clone());
        assert_eq!(label("n3").as_deref(), Some("先留着选项一"));
        assert_eq!(label("n1").as_deref(), Some("两个选项"), "its parent is not its to change");
        assert!(label("n5").is_some(), "not its to delete");
        assert!(done.graph.nodes.iter().any(|n| n.label == "新加一点" && n.parent.as_deref() == Some("n1")));
    }

    #[test]
    fn a_section_is_rewritten_alone_and_put_back() {
        let provider = Provider::Scripted(Arc::new(|_: &str| Ok("T 两个选项\n  L 选项一，按比例分成\n  L 先留着选项一".into())));
        let mut req = request(Scope::Section(vec!["n1".into()]));
        req.change = Change::Merge;
        let done = redraw(&provider, &req, provider.default_guidance(), &sound::Glossary::default(), &[]).unwrap();
        let labels: Vec<&str> = done.graph.nodes.iter().map(|n| n.label.as_str()).collect();
        assert_eq!(labels, vec!["两个选项", "选项一，按比例分成", "先留着选项一", "待定", "结算"]);
        assert!(done.asked.contains("合并后"), "the section is told what was merged");
        assert_eq!(done.graph.nodes[0].id, "n1", "the topic keeps its id");
    }

    #[test]
    fn lines_that_drew_nothing_are_drawn_as_a_turn() {
        let provider = Provider::Scripted(Arc::new(|said: &str| {
            assert!(!said.contains("只改这几项"));
            Ok("L 「两个选项」 「先留着选项一」".into())
        }));
        let mut req = request(Scope::NewTurn);
        req.traced = Traced::default();
        let done = redraw(&provider, &req, provider.default_guidance(), &sound::Glossary::default(), &[]).unwrap();
        let added = done.graph.nodes.iter().find(|n| n.label == "先留着选项一").unwrap();
        assert_eq!(added.extra.get("by").and_then(|b| b.as_str()), Some("E"));
    }

    #[test]
    fn an_edit_is_planned_on_every_board_its_lines_drew_on() {
        use serde_json::json;
        let mut first = Graph::default();
        first.apply(&parse_ops("T 「两个选项」\nL 「两个选项」 「删掉选项一」"));
        let steps = vec![json!({ "t": 1, "line": 1, "d": alcyo_board::steps::diff(&Graph::default(), &first).unwrap(), "from": [0] })];
        let lines = vec![
            json!({ "text": "那就先留着选项一。", "speaker": "E", "page": 0, "ms": 1, "heard": { "text": "那就删掉选项一。" } }),
            json!({ "text": "下一块板上说的", "speaker": "B", "page": 1, "ms": 1 }),
        ];
        let boards = vec![Board { page: 0, graph: first.clone(), steps }, Board { page: 1, graph: Graph::default(), steps: vec![] }];
        let before = vec![json!({ "text": "那就删掉选项一。", "speaker": "E", "page": 0 })];
        let planned = plan(&lines, &boards, Change::Words, 0, &[0], &before, 1, 3, 9);
        assert_eq!(planned.len(), 1, "only the board it drew on");
        let r = &planned[0];
        assert_eq!((r.page, r.on_screen, r.edit, r.meeting), (0, false, 9, 3), "redrawn in the background");
        assert_eq!(r.scope, Scope::Items);
        assert_eq!(r.after[0].text, "那就先留着选项一。");
        assert_eq!(r.before[0].text, "那就删掉选项一。");
        // Speaker only: no model; delete with its items: no model.
        let mut moved = lines.clone();
        moved[0]["speaker"] = json!("D");
        let before = vec![json!({ "text": "那就先留着选项一。", "speaker": "E", "page": 0 })];
        let owner = &plan(&moved, &boards, Change::Speaker, 0, &[0], &before, 0, 3, 10)[0];
        assert_eq!(owner.scope, Scope::Owner);
        let mut b = first.clone();
        b.nodes[1].extra.insert("by".into(), json!("E"));
        let owner = Redraw { board: b, ..owner.clone() };
        assert_eq!(without_model(&owner).unwrap().nodes[1].extra["by"], "D");
        let mut deleted = lines.clone();
        deleted[0]["deleted"] = json!(true);
        let del = &plan(&deleted, &boards, Change::Delete, 0, &[0], &before, 0, 3, 11)[0];
        assert!(without_model(del).unwrap().nodes.is_empty(), "what it drew goes with it");
    }

    #[test]
    fn a_flow_chart_is_redrawn_by_its_items() {
        let mut flow = Graph::default();
        flow.apply(&parse_ops("N 「网关」\nN 「鉴权」\nE 「网关」 -> 「鉴权」"));
        assert_eq!(fitted(Scope::Board, &flow), Scope::Items);
        assert_eq!(fitted(Scope::Board, &board()), Scope::Board);
    }
}
