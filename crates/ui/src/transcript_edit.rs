//! Editing the transcript on the iPad (#362, round 7, direction A on every page): press and hold a line for its
//! menu (Edit Text, Change Speaker, Select Lines, Copy, Delete Line), edit it where it is with Cancel and Done in
//! the card's bar; Select mode with circles and a bar of Merge, Speaker and Delete; and, while the board follows,
//! the items being redrawn glowing on it, the line saying so under itself, and a toast with Undo when it lands.
//! What an edit does to the meeting and the board is decided outside the window (`alcyo_meetings::Live`,
//! `alcyo_agent::redraw::plan`), so the desktop only needs its own way of showing it. A child of `app`.

use super::ipad_dialogs::{self, MenuRow};
use super::*;
use alcyo_agent::redraw::{self, Board, Redraw};
use alcyo_board::trace::{trace, Change, Scope};
use alcyo_meetings::edit as lines_edit;
use alcyo_meetings::{EditKind, LineEdit};
use gpui_kit::{AnyElement, BoxShadow, FontWeight};
use std::collections::BTreeSet;

/// How long a finger stays on a line before its menu opens.
const HOLD: Duration = Duration::from_millis(450);
/// How far a finger may move and still be holding rather than scrolling.
const SLOP: f32 = 10.0;
/// How long the toast after a redraw stays, and the items it changed glow.
const TOAST: Duration = Duration::from_secs(6);
const FRESH: Duration = Duration::from_millis(2400);

/// What the transcript's editing holds between frames.
#[derive(Default)]
pub(crate) struct EditState {
    /// The line whose menu is open, and whether it shows the speakers to give it to.
    pub(crate) menu: Option<(usize, bool)>,
    /// A finger down on a line: which, when and where.
    press: Option<(usize, Instant, Point<Pixels>)>,
    /// The line being edited where it is, and its field.
    pub(crate) editing: Option<(usize, Entity<InputState>)>,
    /// Select mode, and the lines picked in it.
    pub(crate) selecting: bool,
    pub(crate) picked: BTreeSet<usize>,
    /// The bar's Speaker menu in Select mode.
    speakers_open: bool,
    /// The merge sheet: the lines, the merged words' field, and who said it.
    merge: Option<(Vec<usize>, Entity<InputState>, Option<String>)>,
    /// A question before going on.
    ask: Option<Ask>,
    /// Redraws on their way.
    pending: Vec<Pending>,
    /// Redraws that failed, to try again from their line.
    failed: Vec<Pending>,
    /// Whole-board redraws waiting for listening to stop, on a local model.
    deferred: Vec<(Pending, Redraw)>,
    /// What the last redraw did, with Undo, until it goes.
    toast: Option<(String, Instant)>,
    /// What the last redraw changed, glowing for a moment on the board.
    fresh: Option<(Vec<String>, Instant)>,
    /// The edited line whose heard words are shown.
    heard_open: Option<usize>,
}

/// A question before an edit goes on.
enum Ask {
    /// Lines that drew things on the board: delete only them, or what they drew too.
    Delete(Vec<usize>, usize),
    /// A redraw of the whole board, asked first: its edit's lines are already fixed.
    WholeBoard(Pending, Redraw),
}

/// A redraw on its way, as the transcript and the board show it.
#[derive(Clone)]
struct Pending {
    edit: u64,
    /// The line that shows it.
    line: usize,
    page: usize,
    scope: Scope,
    /// The items being redrawn, on that board.
    nodes: Vec<String>,
    items: usize,
    /// That board as it was, to tell what the redraw changed.
    before: Graph,
}

fn alpha(color: u32, a: f32) -> Hsla {
    let mut c: Hsla = rgb(color).into();
    c.a = a;
    c
}

fn change_of(kind: EditKind) -> Change {
    match kind {
        EditKind::Text | EditKind::Restore => Change::Words,
        EditKind::Speaker => Change::Speaker,
        EditKind::Merge => Change::Merge,
        EditKind::Delete => Change::Delete,
    }
}

impl Workbench {
    fn edit_boards(&self) -> Vec<Board> {
        (0..self.live.pages()).map(|page| Board { page, graph: self.live.board_of(page), steps: self.live.page_steps(page) }).collect()
    }

    /// How many board items these lines drew, on every board.
    fn items_drawn(&self, lines: &[usize]) -> usize {
        self.edit_boards().iter().map(|b| trace(&self.live.lines, &b.steps, &b.graph, b.page, lines).len()).sum()
    }

    /// The items these lines drew on the board on screen.
    fn nodes_drawn(&self, lines: &[usize]) -> Vec<String> {
        trace(&self.live.lines, &self.live.steps, &self.live.graph, self.live.page(), lines).nodes
    }

    /// Whether a line can be edited now: on the board already, in a meeting that can be edited.
    pub(super) fn line_editable(&self, i: usize) -> bool {
        self.editable() && self.live.lines.get(i).is_some_and(lines_edit::editable)
    }

    // ---- holding a line ----

    pub(super) fn line_pressed(&mut self, i: usize, at: Point<Pixels>) {
        if !self.edit.selecting && self.edit.editing.is_none() {
            self.edit.press = Some((i, Instant::now(), at));
        }
    }

    pub(super) fn line_moved(&mut self, at: Point<Pixels>) {
        if let Some((_, _, from)) = self.edit.press {
            if (f32::from(at.x) - f32::from(from.x)).abs() > SLOP || (f32::from(at.y) - f32::from(from.y)).abs() > SLOP {
                self.edit.press = None;
            }
        }
    }

    pub(super) fn line_released(&mut self) {
        self.edit.press = None;
    }

    /// Open a line's menu: by holding it, or a right click.
    pub(super) fn open_line_menu(&mut self, i: usize, cx: &mut Context<Self>) {
        self.edit.press = None;
        if !self.editable() || self.live.lines.get(i).is_none_or(|l| l["manual"].as_bool().unwrap_or(false) || lines_edit::is_gone(l)) {
            return;
        }
        self.edit.menu = Some((i, false));
        self.ipad.menu = Some("line-menu");
        cx.notify();
    }

    /// Called with every frame's pump: a held line opens its menu, a toast and a glow run out, and a whole board
    /// waiting for listening to stop is redrawn once it has.
    pub(super) fn edit_tick(&mut self, changed: &mut bool, cx: &mut Context<Self>) {
        if let Some((i, since, _)) = self.edit.press {
            if since.elapsed() >= HOLD {
                self.open_line_menu(i, cx);
                *changed = true;
            }
        }
        if self.edit.menu.is_some() && self.ipad.menu != Some("line-menu") {
            self.edit.menu = None;
            *changed = true;
        }
        if self.edit.toast.as_ref().is_some_and(|(_, until)| Instant::now() >= *until) {
            self.edit.toast = None;
            *changed = true;
        }
        if self.edit.fresh.as_ref().is_some_and(|(_, until)| Instant::now() >= *until) {
            self.edit.fresh = None;
            *changed = true;
        }
        if !self.edit.deferred.is_empty() && !self.pipeline.listening() {
            for (pending, req) in std::mem::take(&mut self.edit.deferred) {
                self.send_redraw(pending, req);
            }
            *changed = true;
        }
        // Something is glowing: the board is painted again for its pulse.
        if !self.edit.pending.is_empty() {
            *changed = true;
        }
    }

    /// The rows of a line's menu: what it drew, then what can be done with it.
    fn line_menu_rows(&self, i: usize) -> Vec<MenuRow> {
        let Some(line) = self.live.lines.get(i) else { return vec![] };
        if !lines_edit::editable(line) {
            return vec![MenuRow::heading(t("line-waiting")), MenuRow::new(t("line-copy"), move |this, _, cx| this.copy_line(i, cx)).glyph("ipad/copy")];
        }
        if self.edit.menu.is_some_and(|(_, speakers)| speakers) {
            let from = line["speaker"].as_str().unwrap_or_default().to_string();
            let names = self.live.speakers();
            let mut rows = vec![MenuRow::heading(t("line-speaker-heading"))];
            rows.extend(names.iter().map(|name| {
                let to = name.clone();
                MenuRow::new(speaker_name(name), move |this, _, cx| this.give_line(i, &to, cx)).checked(*name == from)
            }));
            let new = alcyo_board::speakers::next_name(names.iter().map(String::as_str));
            rows.push(MenuRow::new(ta!("someone-new-name", name = new.clone()), move |this, _, cx| this.give_line(i, &new, cx)).glyph("ipad/person-plus").apart());
            return rows;
        }
        let items = self.items_drawn(&[i]);
        vec![
            MenuRow::heading(ta!("line-drew", items = items)),
            MenuRow::new(t("line-edit-text"), move |this, window, cx| this.start_editing(i, window, cx)).glyph("ipad/compose"),
            MenuRow::new(t("line-change-speaker"), move |this, _, cx| {
                this.edit.menu = Some((i, true));
                this.ipad.menu = Some("line-menu");
                cx.notify();
            })
            .glyph("ipad/people"),
            MenuRow::new(t("line-select-lines"), move |this, _, cx| this.enter_select(Some(i), cx)).glyph("ipad/select"),
            MenuRow::new(t("line-copy"), move |this, _, cx| this.copy_line(i, cx)).glyph("ipad/copy"),
            MenuRow::new(t("line-delete"), move |this, _, cx| this.ask_delete(vec![i], cx)).glyph("ipad/trash").apart(),
        ]
    }

    /// The menu under a line, when it is open there.
    pub(super) fn line_menu(&self, i: usize, cx: &mut Context<Self>) -> Option<gpui_kit::Div> {
        self.edit.menu.filter(|(at, _)| *at == i)?;
        let trigger = div().id("line-menu-anchor").w(px(1.)).h(px(1.));
        Some(div().flex().justify_end().h(px(1.)).child(self.ipad_menu("line-menu", trigger, self.line_menu_rows(i), false, cx)))
    }

    fn copy_line(&mut self, i: usize, cx: &mut Context<Self>) {
        if let Some(text) = self.live.lines.get(i).and_then(|l| l["text"].as_str()) {
            cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(text.to_string()));
        }
        self.edit.menu = None;
    }

    // ---- editing one line ----

    fn start_editing(&mut self, i: usize, window: &mut Window, cx: &mut Context<Self>) {
        if !self.line_editable(i) {
            return;
        }
        let text = self.live.lines[i]["text"].as_str().unwrap_or_default().to_string();
        let field = cx.new(|cx| InputState::new(window, cx));
        field.update(cx, |state, cx| {
            state.set_value(text, window, cx);
            state.focus(window, cx);
        });
        cx.subscribe_in(&field, window, |this: &mut Workbench, _, event, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.finish_editing(window, cx);
            }
        })
        .detach();
        self.edit.menu = None;
        self.edit.editing = Some((i, field));
        cx.notify();
    }

    pub(super) fn finish_editing(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let Some((i, field)) = self.edit.editing.take() else { return };
        let text = field.read(cx).value().to_string();
        if let Some(edit) = self.live.edit_text(i, &text) {
            self.after_edit(edit, true, cx);
        }
        cx.notify();
    }

    pub(super) fn cancel_editing(&mut self, cx: &mut Context<Self>) {
        self.edit.editing = None;
        cx.notify();
    }

    fn give_line(&mut self, i: usize, to: &str, cx: &mut Context<Self>) {
        self.edit.menu = None;
        if let Some(edit) = self.live.edit_speaker(i, to) {
            self.after_edit(edit, true, cx);
        }
        cx.notify();
    }

    fn restore_line(&mut self, i: usize, cx: &mut Context<Self>) {
        self.edit.heard_open = None;
        if let Some(edit) = self.live.restore_line(i) {
            self.after_edit(edit, true, cx);
        }
        cx.notify();
    }

    // ---- Select mode ----

    pub(super) fn enter_select(&mut self, first: Option<usize>, cx: &mut Context<Self>) {
        self.edit.menu = None;
        self.edit.editing = None;
        self.edit.selecting = true;
        self.edit.picked = first.filter(|i| self.line_editable(*i)).into_iter().collect();
        cx.notify();
    }

    pub(super) fn leave_select(&mut self, cx: &mut Context<Self>) {
        self.edit.selecting = false;
        self.edit.speakers_open = false;
        self.edit.picked.clear();
        cx.notify();
    }

    pub(super) fn toggle_pick(&mut self, i: usize, cx: &mut Context<Self>) {
        if !self.line_editable(i) {
            return;
        }
        if !self.edit.picked.remove(&i) {
            self.edit.picked.insert(i);
        }
        cx.notify();
    }

    fn pick_all(&mut self, cx: &mut Context<Self>) {
        let all: BTreeSet<usize> = (0..self.live.lines.len()).filter(|i| self.line_editable(*i)).collect();
        self.edit.picked = if self.edit.picked == all { BTreeSet::new() } else { all };
        cx.notify();
    }

    fn picked(&self) -> Vec<usize> {
        self.edit.picked.iter().copied().collect()
    }

    fn open_merge(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let picked = self.picked();
        if !lines_edit::adjacent(&self.live.lines, &picked) {
            return;
        }
        let words = lines_edit::join_words(picked.iter().map(|i| self.live.lines[*i]["text"].as_str().unwrap_or_default()));
        let speaker = self.live.lines[picked[0]]["speaker"].as_str().map(str::to_string);
        let field = cx.new(|cx| InputState::new(window, cx));
        field.update(cx, |state, cx| state.set_value(words, window, cx));
        self.edit.merge = Some((picked, field, speaker));
        cx.notify();
    }

    fn confirm_merge(&mut self, cx: &mut Context<Self>) {
        let Some((picked, field, speaker)) = self.edit.merge.take() else { return };
        let text = field.read(cx).value().to_string();
        if let Some(edit) = self.live.merge_lines(&picked, speaker.as_deref(), Some(&text)) {
            self.leave_select(cx);
            self.after_edit(edit, true, cx);
        }
        cx.notify();
    }

    fn give_picked(&mut self, to: &str, cx: &mut Context<Self>) {
        self.edit.speakers_open = false;
        for i in self.picked() {
            if let Some(edit) = self.live.edit_speaker(i, to) {
                self.after_edit(edit, false, cx);
            }
        }
        self.leave_select(cx);
    }

    /// Delete lines: at once if they drew nothing, else asking whether what they drew goes too.
    fn ask_delete(&mut self, lines: Vec<usize>, cx: &mut Context<Self>) {
        self.edit.menu = None;
        let lines: Vec<usize> = lines.into_iter().filter(|i| self.line_editable(*i)).collect();
        if lines.is_empty() {
            return;
        }
        let items = self.items_drawn(&lines);
        if items == 0 {
            self.delete_now(&lines, false, cx);
        } else {
            self.edit.ask = Some(Ask::Delete(lines, items));
        }
        cx.notify();
    }

    fn delete_now(&mut self, lines: &[usize], with_items: bool, cx: &mut Context<Self>) {
        if let Some(edit) = self.live.delete_lines(lines) {
            self.leave_select(cx);
            self.after_edit(edit, with_items, cx);
        }
    }

    // ---- the board following ----

    /// After an edit: the model's context hears the corrected lines, and every board the lines drew on follows,
    /// at once when no model is needed, else by a redraw (asked first for the whole board). With `redraw` false
    /// (deleting a line only) the boards stay as they are.
    fn after_edit(&mut self, edit: LineEdit, redraw: bool, cx: &mut Context<Self>) {
        self.sync_history();
        if !redraw {
            return;
        }
        let planned = redraw::plan(&self.live.lines, &self.edit_boards(), change_of(edit.kind), edit.line, &edit.lines, &edit.before, self.live.page(), self.session().meeting(), edit.edit);
        for req in planned {
            let nodes = if req.page == self.live.page() { req.traced.nodes.clone() } else { vec![] };
            let pending = Pending { edit: edit.edit, line: edit.line, page: req.page, scope: req.scope.clone(), nodes, items: req.traced.len(), before: req.board.clone() };
            if let Some(graph) = redraw::without_model(&req) {
                alcyo_diag::event("board.redraw", serde_json::json!({ "ok": true, "edit": req.edit, "page": req.page, "change": format!("{:?}", req.change), "scope": req.scope.name(), "traced": req.traced.len(), "model": false, "before": req.board, "after": graph }));
                self.board_followed(&pending, graph, false, cx);
                continue;
            }
            if req.scope == Scope::Board {
                self.edit.ask = Some(Ask::WholeBoard(pending, req));
                continue;
            }
            self.send_redraw(pending, req);
        }
        cx.notify();
    }

    /// Ask the model, unless it is a whole board on a local model while listening: that waits for listening to stop.
    fn send_redraw(&mut self, pending: Pending, req: Redraw) {
        if req.scope == Scope::Board && self.pipeline.listening() && self.session().local() {
            self.edit.deferred.push((pending, req));
            return;
        }
        self.edit.failed.retain(|p| p.edit != pending.edit || p.page != pending.page);
        self.edit.pending.push(pending);
        self.session().redraw(req);
    }

    /// The model's context, from the lines as they are now.
    pub(super) fn sync_history(&self) {
        let history: Vec<Utterance> = self.live.history.iter().filter_map(|h| serde_json::from_value(h.clone()).ok()).collect();
        self.session().set_history(history);
    }

    /// A board came back for an edit: put it on, joined to the edit's undo position, and say so.
    fn board_followed(&mut self, pending: &Pending, graph: Graph, by_model: bool, cx: &mut Context<Self>) {
        let on_screen = pending.page == self.live.page();
        let changed: Vec<String> = graph.nodes.iter().filter(|n| pending.before.nodes.iter().find(|b| b.id == n.id) != Some(n)).map(|n| n.id.clone()).collect();
        if on_screen {
            let graph = if graph.has_loose_anchors() {
                let before = self.ink.borrow().targets.clone();
                self.session().set_graph(graph);
                self.session().settle_anchors(&before)
            } else {
                graph
            };
            self.next_batch += 1;
            let mut ink = self.ink.borrow_mut();
            ink.update(&graph, self.next_batch);
            let memory = ink.board.snapshot();
            drop(ink);
            if !self.live.redrawn(pending.edit, pending.page, graph.clone(), Some(memory)) {
                // Undone meanwhile: the board stays as undo left it.
                self.session().set_graph(self.live.graph.clone());
                let mut ink = self.ink.borrow_mut();
                self.next_batch += 1;
                ink.update(&self.live.graph, self.next_batch);
                return;
            }
            if !by_model {
                self.session().edited_board(graph);
            }
            self.edit.fresh = Some((changed.clone(), Instant::now() + FRESH));
        } else if !self.live.redrawn(pending.edit, pending.page, graph, None) {
            return;
        }
        let text = match (&pending.scope, by_model) {
            (Scope::Section(tops), true) if tops.len() == 1 => {
                let name = pending.before.nodes.iter().find(|n| n.id == tops[0]).map(|n| n.label.clone()).unwrap_or_default();
                ta!("board-redrew-section", name = name)
            }
            _ => ta!("board-updated", items = changed.len().max(pending.items.min(1))),
        };
        self.edit.toast = Some((text, Instant::now() + TOAST));
        cx.notify();
    }

    /// The model's answer to a redraw.
    pub(super) fn redraw_answered(&mut self, edit: u64, page: usize, graph: Option<Graph>, message: Option<String>, cx: &mut Context<Self>) {
        let Some(at) = self.edit.pending.iter().position(|p| p.edit == edit && p.page == page) else { return };
        let pending = self.edit.pending.remove(at);
        match (graph, message) {
            (Some(graph), _) => self.board_followed(&pending, graph, true, cx),
            (None, Some(message)) => {
                eprintln!("redrawing after an edit failed: {message}");
                self.edit.failed.push(pending);
            }
            _ => {}
        }
        cx.notify();
    }

    /// An edit was undone or redone: the model hears the lines as they are. A redraw still on its way is refused
    /// when it comes ([`Live::redrawn`]), and a failed one is no longer to try.
    pub(super) fn edit_undone(&mut self, edit: Option<u64>) {
        self.sync_history();
        self.edit.toast = None;
        self.edit.failed.retain(|p| Some(p.edit) != edit);
        self.edit.deferred.retain(|(p, _)| Some(p.edit) != edit);
    }

    fn retry_redraw(&mut self, edit: u64, cx: &mut Context<Self>) {
        let Some(at) = self.edit.failed.iter().position(|p| p.edit == edit) else { return };
        let pending = self.edit.failed.remove(at);
        // Plan again from the line as it is: the board may have moved on.
        let line = pending.line;
        let before: Vec<serde_json::Value> = self.live.lines.get(line).map(|l| {
            let mut was = l.clone();
            if let Some((text, speaker)) = lines_edit::heard(l) {
                was["text"] = serde_json::json!(text);
                if let Some(s) = speaker {
                    was["speaker"] = serde_json::json!(s);
                }
            }
            was
        }).into_iter().collect();
        let planned = redraw::plan(&self.live.lines, &self.edit_boards(), Change::Words, line, &[line], &before, self.live.page(), self.session().meeting(), edit);
        for req in planned.into_iter().filter(|r| r.page == pending.page) {
            let pending = Pending { scope: req.scope.clone(), items: req.traced.len(), before: req.board.clone(), nodes: if req.page == self.live.page() { req.traced.nodes.clone() } else { vec![] }, ..pending.clone() };
            self.send_redraw(pending, req);
        }
        cx.notify();
    }

    /// What glows on the board: the items being redrawn, pulsing; the items just changed; or the items the line
    /// with its menu open or the lines picked drew.
    pub(super) fn edit_glow(&self) -> Option<(Vec<String>, bool)> {
        let page = self.live.page();
        let busy: Vec<String> = self.edit.pending.iter().filter(|p| p.page == page).flat_map(|p| p.nodes.clone()).collect();
        if !busy.is_empty() {
            return Some((busy, true));
        }
        if let Some((nodes, _)) = &self.edit.fresh {
            return Some((nodes.clone(), false));
        }
        let lines: Vec<usize> = if self.edit.selecting {
            self.picked()
        } else if let Some((i, _)) = self.edit.menu {
            vec![i]
        } else if let Some((i, _)) = &self.edit.editing {
            vec![*i]
        } else {
            return None;
        };
        let nodes = self.nodes_drawn(&lines);
        (!nodes.is_empty()).then_some((nodes, false))
    }

    // ---- what shows ----

    /// The status under a line: its redraw on its way, failed, or waiting for listening to stop.
    pub(super) fn line_status(&self, i: usize, cx: &mut Context<Self>) -> Option<AnyElement> {
        let p = pal();
        if let Some(pending) = self.edit.pending.iter().find(|x| x.line == i) {
            let text = match &pending.scope {
                Scope::Section(tops) if tops.len() == 1 => ta!("redrawing-section", name = pending.before.nodes.iter().find(|n| n.id == tops[0]).map(|n| n.label.clone()).unwrap_or_default()),
                Scope::Board => t("redrawing-board").to_string(),
                _ => ta!("redrawing-items", items = pending.items.max(1)),
            };
            return Some(div().pl(px(32.)).flex().items_center().gap(px(6.)).text_size(px(12.5)).text_color(rgb(p.accent)).child(Spinner::new().xsmall().color(rgb(p.accent).into())).child(text).into_any_element());
        }
        if self.edit.deferred.iter().any(|(x, _)| x.line == i) {
            return Some(div().pl(px(32.)).text_size(px(12.5)).text_color(rgb(p.faint)).child(t("redraw-waits")).into_any_element());
        }
        if let Some(pending) = self.edit.failed.iter().find(|x| x.line == i) {
            let edit = pending.edit;
            return Some(
                div()
                    .pl(px(32.))
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(6.))
                    .text_size(px(12.5))
                    .text_color(rgb(p.danger))
                    .child(t("redraw-failed"))
                    .child(div().id(("redraw-again", i)).text_color(rgb(p.accent)).font_weight(FontWeight::SEMIBOLD).cursor_pointer().child(t("redraw-try-again")).on_click(cx.listener(move |this, _, _, cx| this.retry_redraw(edit, cx))))
                    .into_any_element(),
            );
        }
        None
    }

    /// "Edited" after an edited line's words, which shows what was heard.
    pub(super) fn edited_tag(&self, i: usize, cx: &mut Context<Self>) -> Option<AnyElement> {
        let line = self.live.lines.get(i)?;
        lines_edit::heard(line)?;
        let p = pal();
        Some(
            div()
                .id(("line-edited", i))
                .flex_none()
                .flex()
                .items_center()
                .gap(px(3.))
                .text_size(px(11.5))
                .text_color(rgb(p.faint))
                .cursor_pointer()
                .child(icon("ipad/compose").size(px(12.)))
                .child(t("line-edited"))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.edit.heard_open = if this.edit.heard_open == Some(i) { None } else { Some(i) };
                    cx.stop_propagation();
                    cx.notify();
                }))
                .into_any_element(),
        )
    }

    /// What was heard, under an edited line whose tag was tapped, with the way back to it.
    pub(super) fn heard_note(&self, i: usize, cx: &mut Context<Self>) -> Option<AnyElement> {
        (self.edit.heard_open == Some(i)).then_some(())?;
        let line = self.live.lines.get(i)?;
        let (text, speaker) = lines_edit::heard(line)?;
        let p = pal();
        let heard = match speaker.filter(|s| Some(*s) != line["speaker"].as_str()) {
            Some(who) => format!("{}: {text}", speaker_name(who)),
            None => text.to_string(),
        };
        Some(
            div()
                .ml(px(32.))
                .mt(px(4.))
                .p(px(10.))
                .rounded(px(10.))
                .bg(ipad_dialogs::well())
                .flex()
                .flex_col()
                .gap(px(6.))
                .text_size(px(13.))
                .child(div().text_color(rgb(p.muted)).child(ta!("line-heard", text = heard)))
                .when(self.line_editable(i), |d| {
                    d.child(div().id(("line-restore", i)).text_color(rgb(p.accent)).font_weight(FontWeight::SEMIBOLD).cursor_pointer().child(t("line-restore")).on_click(cx.listener(move |this, _, _, cx| this.restore_line(i, cx))))
                })
                .into_any_element(),
        )
    }

    /// The field a line is edited in, in its place.
    pub(super) fn line_field(&self, i: usize) -> Option<AnyElement> {
        let (at, field) = self.edit.editing.as_ref()?;
        (*at == i).then_some(())?;
        let p = pal();
        Some(
            div()
                .ml(px(26.))
                .px(px(9.))
                .py(px(6.))
                .rounded(px(10.))
                .bg(rgb(p.surface))
                .border_2()
                .border_color(alpha(p.accent, 0.7))
                .child(Input::new(field).appearance(false))
                .into_any_element(),
        )
    }

    /// The circle before a line in Select mode.
    pub(super) fn pick_circle(&self, i: usize) -> Option<AnyElement> {
        if !self.edit.selecting {
            return None;
        }
        let p = pal();
        let on = self.edit.picked.contains(&i);
        let can = self.line_editable(i);
        Some(
            div()
                .absolute()
                .left(px(-30.))
                .top(px(2.))
                .size(px(22.))
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .when(on, |d| d.bg(rgb(p.accent)).text_color(rgb(0xff_ff_ff)).child(icon("ipad/check").size(px(13.))))
                .when(!on, |d| d.border_1().border_color(alpha(p.faint, if can { 0.6 } else { 0.25 })).bg(rgb(p.surface)))
                .into_any_element(),
        )
    }

    /// Whether the card's bar shows Cancel and Done rather than its tabs.
    pub(super) fn ipad_edit_header_shown(&self) -> bool {
        self.edit.editing.is_some() || self.edit.selecting
    }

    /// The card's bar while a line is edited or lines are picked: Cancel, what is going on, and Done or Select All.
    pub(super) fn ipad_edit_header(&self, cx: &mut Context<Self>) -> Option<gpui_kit::Div> {
        let p = pal();
        let (title, right): (String, String) = if self.edit.editing.is_some() {
            (t("line-edit-text").into(), t("done").into())
        } else if self.edit.selecting {
            (ta!("lines-selected", count = self.edit.picked.len()), t("select-all").into())
        } else {
            return None;
        };
        let editing = self.edit.editing.is_some();
        let side = |id: &'static str, text: String, bold: bool| div().id(id).min_w(px(80.)).text_size(px(15.5)).text_color(rgb(p.accent)).when(bold, |d| d.font_weight(FontWeight::SEMIBOLD)).cursor_pointer().child(text);
        Some(
            div()
                .flex_none()
                .h(px(48.))
                .px(px(16.))
                .mb(px(4.))
                .flex()
                .items_center()
                .justify_between()
                .child(side("edit-cancel", t("cancel").into(), false).on_click(cx.listener(move |this, _, _, cx| if editing { this.cancel_editing(cx) } else { this.leave_select(cx) })))
                .child(div().text_size(px(15.5)).font_weight(FontWeight::SEMIBOLD).child(title))
                .child(div().min_w(px(80.)).flex().justify_end().child(side("edit-done", right, editing).on_click(cx.listener(move |this, _, window, cx| if editing { this.finish_editing(window, cx) } else { this.pick_all(cx) })))),
        )
    }

    /// The bar at the card's foot in Select mode: Merge, Speaker and Delete.
    pub(super) fn ipad_select_bar(&self, cx: &mut Context<Self>) -> Option<gpui_kit::Div> {
        if !self.edit.selecting {
            return None;
        }
        let p = pal();
        let picked = self.picked();
        let can_merge = lines_edit::adjacent(&self.live.lines, &picked);
        let any = !picked.is_empty();
        let action = |id: &'static str, glyph: &'static str, label: &'static str, color: u32, on: bool| {
            div()
                .id(id)
                .w(px(76.))
                .flex()
                .flex_col()
                .items_center()
                .gap(px(2.))
                .text_size(px(11.5))
                .font_weight(FontWeight::MEDIUM)
                .text_color(if on { rgb(color).into() } else { alpha(p.faint, 0.5) })
                .when(on, |d| d.cursor_pointer())
                .child(icon(glyph).size(px(22.)))
                .child(label)
        };
        let names = self.live.speakers();
        let mut rows = vec![MenuRow::heading(ta!("lines-give-to", count = picked.len()))];
        rows.extend(names.iter().map(|name| {
            let to = name.clone();
            MenuRow::new(speaker_name(name), move |this, _, cx| this.give_picked(&to, cx))
        }));
        let new = alcyo_board::speakers::next_name(names.iter().map(String::as_str));
        rows.push(MenuRow::new(ta!("someone-new-name", name = new.clone()), move |this, _, cx| this.give_picked(&new, cx)).glyph("ipad/person-plus").apart());
        let speaker = action("select-speaker", "ipad/people", t("lines-speaker"), p.accent, any);
        let speaker = if any { self.ipad_menu("select-speaker-menu", speaker, rows, true, cx) } else { div().child(speaker) };
        Some(
            div()
                .flex_none()
                .h(px(62.))
                .flex()
                .items_center()
                .justify_around()
                .border_t_1()
                .border_color(rgb(p.line))
                .child(action("select-merge", "ipad/merge", t("lines-merge"), p.accent, can_merge).when(can_merge, |d| d.on_click(cx.listener(|this, _, window, cx| this.open_merge(window, cx)))))
                .child(speaker)
                .child(action("select-delete", "ipad/trash", t("lines-delete"), p.danger, any).when(any, |d| d.on_click(cx.listener(|this, _, _, cx| this.ask_delete(this.picked(), cx))))),
        )
    }

    /// The toast above the dock after a redraw: what changed, and Undo.
    pub(super) fn ipad_edit_toast(&self, bottom: f32, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (text, _) = self.edit.toast.as_ref()?;
        let p = pal();
        let dark = theme::is_dark();
        Some(
            div()
                .absolute()
                .left_0()
                .right_0()
                .bottom(px(bottom))
                .flex()
                .justify_center()
                .child(
                    super::ipad::glass(div(), true)
                        .id("edit-toast")
                        .occlude()
                        .h(px(46.))
                        .rounded(px(23.))
                        .pl(px(14.))
                        .pr(px(7.))
                        .flex()
                        .items_center()
                        .gap(px(12.))
                        .text_size(px(14.))
                        .font_weight(FontWeight::MEDIUM)
                        .child(div().size(px(22.)).rounded_full().bg(rgb(0x34_c7_59)).text_color(rgb(0xff_ff_ff)).flex().items_center().justify_center().child(icon("ipad/check").size(px(13.))))
                        .child(text.clone())
                        .child(
                            div()
                                .id("edit-toast-undo")
                                .h(px(32.))
                                .px(px(14.))
                                .rounded_full()
                                .flex()
                                .items_center()
                                .gap(px(6.))
                                .bg(alpha(p.accent, if dark { 0.22 } else { 0.12 }))
                                .text_color(rgb(p.accent))
                                .cursor_pointer()
                                .child(icon("ipad/undo").size(px(14.)))
                                .child(t("undo"))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.edit.toast = None;
                                    this.step(false, cx);
                                })),
                        ),
                )
                .into_any_element(),
        )
    }

    /// The merge sheet, and the questions asked before deleting lines with what they drew or redrawing the whole
    /// board: over a dimmed screen, as the iPad asks.
    pub(super) fn ipad_edit_dialog(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let p = pal();
        let dark = theme::is_dark();
        let surface: Hsla = if dark { rgb(0x2c_2c_2e).into() } else { rgb(p.surface).into() };
        let card = |id: &'static str, w: f32| {
            div()
                .id(id)
                .occlude()
                .w(px(w))
                .flex()
                .flex_col()
                .rounded(px(16.))
                .overflow_hidden()
                .bg(surface)
                .shadow(vec![BoxShadow { color: alpha(0, if dark { 0.6 } else { 0.25 }), offset: point(px(0.), px(18.)), blur_radius: px(50.), spread_radius: px(0.), inset: false }])
                .on_click(|_, _, cx| cx.stop_propagation())
        };
        let answer = |id: &'static str, text: String, color: u32, bold: bool| {
            div().id(id).flex_1().min_h(px(44.)).px(px(8.)).flex().items_center().justify_center().text_center().text_size(px(16.)).text_color(rgb(color)).when(bold, |d| d.font_weight(FontWeight::SEMIBOLD)).cursor_pointer().active(|d| d.bg(ipad_dialogs::well())).child(text)
        };
        let body = if let Some((lines, field, speaker)) = &self.edit.merge {
            let names = self.live.speakers();
            let rows: Vec<MenuRow> = names
                .iter()
                .map(|name| {
                    let to = name.clone();
                    MenuRow::new(speaker_name(name), move |this, _, cx| {
                        if let Some(merge) = &mut this.edit.merge {
                            merge.2 = Some(to.clone());
                        }
                        cx.notify();
                    })
                    .checked(speaker.as_deref() == Some(name.as_str()))
                })
                .collect();
            let who = speaker.clone().unwrap_or_default();
            let pull = div().id("merge-speaker").flex().items_center().gap(px(6.)).text_color(rgb(p.accent)).text_size(px(15.)).cursor_pointer().when(!who.is_empty(), |d| d.child(super::ipad::who(&names, &who, 22., true))).child(speaker_name(&who)).child(icon("ipad/updown").size(px(14.)));
            let joins: Vec<String> = lines.iter().filter_map(|i| self.live.lines.get(*i)).map(|l| l["text"].as_str().unwrap_or_default().to_string()).collect();
            let section = |label: String| div().px(px(16.)).pt(px(14.)).pb(px(6.)).text_size(px(12.5)).text_color(rgb(p.muted)).child(label);
            let group = || div().mx(px(16.)).rounded(px(12.)).bg(ipad_dialogs::well());
            card("merge-sheet", 460.)
                .child(
                    div()
                        .h(px(54.))
                        .px(px(18.))
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(div().id("merge-cancel").min_w(px(70.)).text_size(px(16.)).text_color(rgb(p.accent)).cursor_pointer().child(t("cancel")).on_click(cx.listener(|this, _, _, cx| {
                            this.edit.merge = None;
                            cx.notify();
                        })))
                        .child(div().text_size(px(17.)).font_weight(FontWeight::SEMIBOLD).child(ta!("merge-title", count = lines.len())))
                        .child(div().min_w(px(70.)).flex().justify_end().child(div().id("merge-go").text_size(px(16.)).font_weight(FontWeight::SEMIBOLD).text_color(rgb(p.accent)).cursor_pointer().child(t("lines-merge")).on_click(cx.listener(|this, _, _, cx| this.confirm_merge(cx))))),
                )
                .child(section(t("merge-line").into()))
                .child(group().px(px(14.)).py(px(10.)).child(Input::new(field).appearance(false)))
                .child(section(String::new()))
                .child(group().h(px(46.)).px(px(14.)).flex().items_center().justify_between().child(div().text_size(px(15.)).child(t("merge-said-by"))).child(self.ipad_menu("merge-speaker-menu", pull, rows, false, cx)))
                .child(section(t("merge-joins").into()))
                .child(group().p(px(10.)).flex().flex_wrap().gap(px(6.)).children(joins.into_iter().map(|j| div().px(px(9.)).py(px(3.)).rounded(px(7.)).bg(alpha(p.faint, 0.12)).text_size(px(13.)).text_color(rgb(p.muted)).child(j))))
                .child(div().px(px(30.)).pt(px(8.)).pb(px(20.)).text_size(px(12.5)).line_height(gpui_kit::relative(1.4)).text_color(rgb(p.muted)).child(t("merge-foot")))
        } else {
            let (title, message, buttons): (String, String, Vec<(&'static str, String, u32, bool)>) = match self.edit.ask.as_ref()? {
                Ask::Delete(lines, items) => (
                    ta!("delete-title", count = lines.len()),
                    ta!("delete-message", items = *items),
                    vec![("delete-line-only", ta!("delete-lines-only", count = lines.len()), p.accent, false), ("delete-with-items", ta!("delete-with-items", count = lines.len()), p.danger, true)],
                ),
                Ask::WholeBoard(_, req) => (
                    t("redraw-board-title").into(),
                    ta!("redraw-board-message", items = req.traced.len(), topics = alcyo_board::trace::sections(&req.board, &req.traced).len()),
                    vec![("redraw-only-text", t("redraw-only-text").into(), p.accent, false), ("redraw-board-go", t("redraw-board-go").into(), p.accent, true)],
                ),
            };
            let stacked = buttons.iter().any(|(_, text, _, _)| text.chars().count() > 14);
            let mut row = div().flex().border_t_1().border_color(rgb(p.line)).when(stacked, |d| d.flex_col());
            for (k, (id, text, color, bold)) in buttons.into_iter().enumerate() {
                if k > 0 {
                    row = row.child(if stacked { div().h(px(1.)).bg(rgb(p.line)) } else { div().w(px(1.)).bg(rgb(p.line)) });
                }
                let go = k == 1;
                row = row.child(answer(id, text, color, bold).on_click(cx.listener(move |this, _, _, cx| this.answer_ask(Some(go), cx))));
            }
            card("edit-ask", 300.)
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap(px(6.))
                        .px(px(18.))
                        .pt(px(20.))
                        .pb(px(16.))
                        .child(div().text_center().text_size(px(17.)).font_weight(FontWeight::SEMIBOLD).child(title))
                        .child(div().text_center().text_size(px(13.)).line_height(gpui_kit::relative(1.4)).text_color(rgb(p.muted)).child(message)),
                )
                .child(row)
        };
        Some(
            div()
                .id("edit-veil")
                .occlude()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(alpha(0, if dark { 0.45 } else { 0.2 }))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.edit.merge = None;
                    this.answer_ask(None, cx);
                }))
                .child(body)
                .into_any_element(),
        )
    }

    /// The answer to a question: the second button (`Some(true)`) deletes what the lines drew too, or redraws the
    /// whole board; the first deletes the lines only, or leaves the board; none (a tap beside it) cancels.
    fn answer_ask(&mut self, answer: Option<bool>, cx: &mut Context<Self>) {
        match (self.edit.ask.take(), answer) {
            (Some(Ask::Delete(lines, _)), Some(with_items)) => self.delete_now(&lines, with_items, cx),
            (Some(Ask::WholeBoard(pending, req)), Some(true)) => self.send_redraw(pending, req),
            _ => {}
        }
        cx.notify();
    }

    /// A line was tapped: in Select mode it is picked or let go.
    pub(super) fn line_tapped(&mut self, i: usize, cx: &mut Context<Self>) {
        if self.edit.selecting {
            self.toggle_pick(i, cx);
        }
    }
}

/// A speaker as the transcript names them: "Speaker A" for a letter.
fn speaker_name(name: &str) -> String {
    if name.chars().count() == 1 {
        ta!("speaker-letter", name = name.to_string())
    } else {
        name.to_string()
    }
}
