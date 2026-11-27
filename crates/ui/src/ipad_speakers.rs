//! The Speakers tab as the iPad shows it (round 6, C): one list, each row with a ⋯ menu that holds Mark on the board,
//! Use for next line, Rename… (an alert with a field) and Merge into ▸ (a list of the others, then a confirmation).
//! Come from a transcript line, a card on top shows that line and moves just it through a pull-down, someone new
//! included. A child of `app` beside ipad.rs.

use super::ipad_dialogs::{self, MenuRow};
use super::*;
use gpui_kit::{AnyElement, BoxShadow, FontWeight};

/// The ⋯ menu of each row, and the list of whom to merge into, by the row's place: a menu's id is static.
const ROW_MENUS: [&str; 24] = [
    "speaker-menu-0", "speaker-menu-1", "speaker-menu-2", "speaker-menu-3", "speaker-menu-4", "speaker-menu-5", "speaker-menu-6", "speaker-menu-7",
    "speaker-menu-8", "speaker-menu-9", "speaker-menu-10", "speaker-menu-11", "speaker-menu-12", "speaker-menu-13", "speaker-menu-14", "speaker-menu-15",
    "speaker-menu-16", "speaker-menu-17", "speaker-menu-18", "speaker-menu-19", "speaker-menu-20", "speaker-menu-21", "speaker-menu-22", "speaker-menu-23",
];
const MERGE_MENUS: [&str; 24] = [
    "speaker-merge-0", "speaker-merge-1", "speaker-merge-2", "speaker-merge-3", "speaker-merge-4", "speaker-merge-5", "speaker-merge-6", "speaker-merge-7",
    "speaker-merge-8", "speaker-merge-9", "speaker-merge-10", "speaker-merge-11", "speaker-merge-12", "speaker-merge-13", "speaker-merge-14", "speaker-merge-15",
    "speaker-merge-16", "speaker-merge-17", "speaker-merge-18", "speaker-merge-19", "speaker-merge-20", "speaker-merge-21", "speaker-merge-22", "speaker-merge-23",
];

/// An alert over the Speakers tab.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum SpeakerAlert {
    /// Rename this speaker, the new name typed in `speaker_name`.
    Rename(String),
    /// Merge the first into the second, once confirmed.
    Merge(String, String),
}

/// What the Rename alert's button does with the name typed: nothing yet (empty, or the same name), a rename, or a
/// merge into someone who already has that name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RenameAnswer {
    Nothing,
    Rename,
    Merge,
}

pub(crate) fn rename_answer(names: &[String], from: &str, typed: &str) -> RenameAnswer {
    let to = typed.trim();
    match () {
        _ if to.is_empty() || to == from => RenameAnswer::Nothing,
        _ if names.iter().any(|n| n == to) => RenameAnswer::Merge,
        _ => RenameAnswer::Rename,
    }
}

/// Whom `from` can be merged into: everyone else, in the order they first spoke.
pub(crate) fn merge_targets(names: &[String], from: &str) -> Vec<String> {
    names.iter().filter(|n| *n != from).cloned().collect()
}

/// A name as the tab says it: a speaker known only by a letter is "Speaker A".
fn shown(name: &str) -> String {
    if name.chars().count() == 1 { ta!("speaker-letter", name = name.to_string()) } else { name.to_string() }
}

impl Workbench {
    /// Give `from`'s lines, voice and board marks to `to`: a rename, or a merge when `to` is someone already there.
    pub(super) fn rename_speaker_to(&mut self, from: &str, to: &str, window: &mut Window, cx: &mut Context<Self>) {
        let to = to.trim().to_string();
        if !self.editable() || to.is_empty() || to == from {
            return;
        }
        self.session().rename_speaker(from, &to);
        self.live.rename_speaker(from, &to);
        self.speaker_hovered = None;
        if self.speaker.as_deref() == Some(from) {
            self.speaker = Some(to.clone());
        }
        for name in self.slots.values_mut().filter(|name| **name == from) {
            *name = to.clone();
        }
        self.choose_speaker(to, window, cx);
    }

    /// Mark a speaker's lines on the board, or stop marking them.
    fn mark_speaker(&mut self, name: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.speaker_chosen.as_deref() == Some(name.as_str()) {
            self.speaker_chosen = None;
            cx.notify();
        } else {
            self.choose_speaker(name, window, cx);
        }
    }

    fn open_speaker_alert(&mut self, alert: SpeakerAlert, window: &mut Window, cx: &mut Context<Self>) {
        if let SpeakerAlert::Rename(from) = &alert {
            let from = from.clone();
            self.speaker_name.update(cx, |state, cx| {
                state.set_value(from, window, cx);
                state.focus(window, cx);
            });
        }
        self.speaker_alert = Some(alert);
        cx.notify();
    }

    /// The tab: the line come from, if any, then everyone.
    pub(super) fn ipad_speakers_panel(&self, editable: bool, cx: &mut Context<Self>) -> gpui_kit::Stateful<gpui_kit::Div> {
        let p = pal();
        let names = self.live.speakers();
        let rows: Vec<AnyElement> = names
            .iter()
            .enumerate()
            .map(|(i, name)| {
                let chosen = self.speaker_chosen.as_deref() == Some(name.as_str());
                let next = self.speaker.as_deref() == Some(name.as_str());
                let (lines, items, boards) = self.live.speaker_impact(name);
                let hovered = name.clone();
                let mut row = div()
                    .id(("ipad-speaker", i))
                    .min_h(px(48.))
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .pl(px(14.))
                    .pr(px(6.))
                    .when(chosen, |d| d.bg(alpha_of(p.accent, if theme::is_dark() { 0.18 } else { 0.08 })))
                    .when(i > 0, |d| d.border_t_1().border_color(rgb(p.line)))
                    .on_hover(cx.listener(move |this, over: &bool, _, cx| this.hover_speaker(&hovered, *over, cx)))
                    .child(ipad::who(&names, name, 26., true))
                    .child(div().flex_1().min_w_0().text_size(px(15.)).overflow_hidden().text_ellipsis().whitespace_nowrap().child(name.clone()))
                    .when(next, |d| {
                        d.child(div().flex_none().flex().items_center().gap(px(4.)).px(px(7.)).h(px(22.)).rounded_full().bg(ipad_dialogs::well()).text_size(px(11.5)).text_color(rgb(p.muted)).child(icon("ipad/keyboard").size(px(13.))).child(t("next")))
                    })
                    .child(div().flex_none().text_size(px(13.5)).text_color(rgb(p.faint)).child(ta!("line-count", n = lines)));
                if editable && i < ROW_MENUS.len() {
                    let (menu_id, merge_id) = (ROW_MENUS[i], MERGE_MENUS[i]);
                    let merging = self.ipad.menu == Some(merge_id);
                    let menu_rows = if merging {
                        let mut rows = vec![MenuRow::heading(ta!("speaker-merge-heading", name = shown(name)))];
                        rows.extend(merge_targets(&names, name).into_iter().map(|other| {
                            let (from, to) = (name.clone(), other.clone());
                            MenuRow::new(shown(&other), move |this, window, cx| this.open_speaker_alert(SpeakerAlert::Merge(from.clone(), to.clone()), window, cx))
                        }));
                        rows
                    } else {
                        let (mark, use_next, rename) = (name.clone(), name.clone(), name.clone());
                        let mut rows = vec![
                            MenuRow::heading(format!("{} · {}", shown(name), ta!("affects-short", lines = lines, items = items, boards = boards))),
                            MenuRow::new(t("speaker-mark-on-board"), move |this, window, cx| this.mark_speaker(mark.clone(), window, cx)).glyph("ipad/mark").checked(chosen),
                            MenuRow::new(t("use-for-next-line"), move |this, _, cx| {
                                this.speaker = Some(use_next.clone());
                                cx.notify();
                            })
                            .glyph("ipad/keyboard")
                            .checked(next),
                            MenuRow::new(t("speaker-rename-dots"), move |this, window, cx| this.open_speaker_alert(SpeakerAlert::Rename(rename.clone()), window, cx)).glyph("ipad/pen").apart(),
                        ];
                        if names.len() > 1 {
                            rows.push(MenuRow::new(t("speaker-merge-into"), move |this, _, cx| {
                                // The same button now opens the list of whom to merge into.
                                this.ipad.menu = Some(merge_id);
                                cx.notify();
                            }).glyph("ipad/chevron-right"));
                        }
                        rows
                    };
                    let id = if merging { merge_id } else { menu_id };
                    let dots = div()
                        .id(id)
                        .flex_none()
                        .size(px(36.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_full()
                        .cursor_pointer()
                        .text_color(rgb(p.accent))
                        .active(|d| d.opacity(0.6))
                        .child(icon("ipad/more").size(px(18.)));
                    row = row.child(self.ipad_menu(id, dots, menu_rows, false, cx));
                }
                row.into_any_element()
            })
            .collect();
        let list = div().flex().flex_col().rounded(px(14.)).overflow_hidden().bg(ipad_dialogs::well()).children(rows);
        let caption = |text: &'static str| div().px(px(16.)).text_size(px(12.5)).line_height(gpui_kit::relative(1.4)).text_color(rgb(p.muted)).child(text);
        div()
            .id("speakers-panel")
            .flex_1()
            .overflow_y_scroll()
            .p(px(14.))
            .flex()
            .flex_col()
            .gap(px(8.))
            .when(names.is_empty(), |d| d.child(div().pt(px(24.)).text_center().text_size(px(14.)).text_color(rgb(p.faint)).child(t("nobody-has-spoken-yet"))))
            .when_some(self.ipad_line_card(&names, editable, cx), |d, card| d.child(card))
            .when(!names.is_empty(), |d| {
                d.child(div().px(px(16.)).pt(px(4.)).font_family(theme::MONO).text_size(px(10.5)).text_color(rgb(p.faint)).child(t("speakers").to_uppercase()))
                    .child(list)
                    .child(caption(t("speaker-list-foot")))
            })
    }

    /// The line come from the transcript, when it is still its speaker's: who said it and when, what was said, and a
    /// pull-down to give just this line to someone else, or to someone new.
    fn ipad_line_card(&self, names: &[String], editable: bool, cx: &mut Context<Self>) -> Option<AnyElement> {
        let p = pal();
        let i = self.speaker_line.filter(|_| editable)?;
        let line = self.live.lines.get(i)?;
        let from = line["speaker"].as_str().filter(|s| Some(*s) == self.speaker_chosen.as_deref())?.to_string();
        let said = line["text"].as_str().unwrap_or_default().to_string();
        let when = line["at"].as_f64().map(|ms| local_time(ms).rsplit(' ').next().unwrap_or_default().to_string());
        let someone_new = alcyo_board::speakers::next_name(names.iter().map(String::as_str));
        let mut rows = vec![MenuRow::heading(t("its-voice-moves-too-so"))];
        rows.extend(names.iter().filter(|n| **n != from).map(|name| {
            let to = name.clone();
            MenuRow::new(shown(name), move |this, window, cx| this.move_line(i, to.clone(), window, cx))
        }));
        let to = someone_new.clone();
        rows.push(MenuRow::new(ta!("someone-new-name", name = someone_new), move |this, window, cx| this.move_line(i, to.clone(), window, cx)).glyph("ipad/person-plus").apart());
        let pull = div()
            .id("move-line")
            .flex_none()
            .h(px(34.))
            .px(px(14.))
            .flex()
            .items_center()
            .gap(px(6.))
            .rounded_full()
            .bg(alpha_of(p.accent, if theme::is_dark() { 0.2 } else { 0.1 }))
            .text_color(rgb(p.accent))
            .text_size(px(14.))
            .font_weight(FontWeight::MEDIUM)
            .cursor_pointer()
            .child(t("speaker-move-line"))
            .child(icon("ipad/chevron-down").size(px(14.)));
        Some(
            div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .p(px(14.))
                .rounded(px(14.))
                .bg(ipad_dialogs::well())
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(ipad::who(names, &from, 22., true))
                        .child(div().text_size(px(13.)).text_color(rgb(p.muted)).child(match when {
                            Some(when) => format!("{} · {when}", shown(&from)),
                            None => shown(&from),
                        })),
                )
                .child(div().text_size(px(15.)).line_height(gpui_kit::relative(1.45)).child(said))
                .child(div().flex().child(self.ipad_menu("speaker-move", pull, rows, false, cx)))
                .into_any_element(),
        )
    }

    /// Rename… and Merge as iOS alerts: a small card over a dimmed screen, the title, what it does, a field for the
    /// new name, and the two answers side by side under a hairline.
    pub(super) fn ipad_speaker_alert(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let alert = self.speaker_alert.clone()?;
        let p = pal();
        let dark = theme::is_dark();
        let names = self.live.speakers();
        let close = cx.listener(|this, _, _, cx| {
            this.speaker_alert = None;
            cx.notify();
        });
        let (title, message, field, action, danger, enabled): (String, String, bool, &'static str, bool, bool) = match &alert {
            SpeakerAlert::Rename(from) => {
                let to = self.speaker_name.read(cx).value().trim().to_string();
                let (lines, items, boards) = self.live.speaker_impact(from);
                let affects = ta!("affects-lines-transcript-lines-and", lines = lines, items = items, boards = boards);
                // A name someone already has makes it a merge, and says so.
                let answer = rename_answer(&names, from, &to);
                let merge = answer == RenameAnswer::Merge;
                let message = if merge { ta!("to-already-exists-so-from", from = from.clone(), to = to.clone()) } else { affects };
                (ta!("speaker-rename-title", name = shown(from)), message, true, if merge { t("merge") } else { t("rename") }, merge, answer != RenameAnswer::Nothing)
            }
            SpeakerAlert::Merge(from, to) => {
                let (lines, items, boards) = self.live.speaker_impact(from);
                let affects = ta!("affects-lines-transcript-lines-and", lines = lines, items = items, boards = boards);
                (
                    ta!("speaker-merge-title", from = shown(from), to = shown(to)),
                    format!("{}\n\n{affects}", ta!("speaker-merge-message", from = shown(from), to = shown(to))),
                    false,
                    t("merge"),
                    true,
                    true,
                )
            }
        };
        let go = alert.clone();
        let answer = |id: &'static str, text: &'static str, color: u32, bold: bool| {
            div().id(id).flex_1().h(px(44.)).flex().items_center().justify_center().text_size(px(16.)).text_color(rgb(color)).when(bold, |d| d.font_weight(FontWeight::SEMIBOLD)).cursor_pointer().active(|d| d.bg(ipad_dialogs::well()))
                .child(text)
        };
        let card = div()
            .id("speaker-alert")
            .occlude()
            .w(px(290.))
            .flex()
            .flex_col()
            .rounded(px(16.))
            .overflow_hidden()
            .bg(if dark { rgb(0x2c_2c_2e).into() } else { gpui_kit::Hsla::from(rgb(p.surface)) })
            .shadow(vec![BoxShadow { color: alpha_of(0, if dark { 0.6 } else { 0.25 }), offset: point(px(0.), px(18.)), blur_radius: px(50.), spread_radius: px(0.), inset: false }])
            .on_click(|_, _, cx| cx.stop_propagation())
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
                    .child(div().text_center().text_size(px(13.)).line_height(gpui_kit::relative(1.4)).text_color(rgb(p.muted)).child(message))
                    .when(field, |d| d.child(div().mt(px(6.)).w_full().h(px(34.)).px(px(8.)).flex().items_center().rounded(px(8.)).bg(ipad_dialogs::well()).child(div().flex_1().min_w_0().child(Input::new(&self.speaker_name).appearance(false))))),
            )
            .child(
                div()
                    .flex()
                    .border_t_1()
                    .border_color(rgb(p.line))
                    .child(answer("speaker-alert-cancel", t("cancel"), p.accent, false).on_click(close))
                    .child(div().w(px(1.)).bg(rgb(p.line)))
                    .child(answer("speaker-alert-go", action, if danger { p.danger } else { p.accent }, true).when(!enabled, |d| d.opacity(0.35)).when(enabled, |d| {
                        d.on_click(cx.listener(move |this, _, window, cx| {
                            this.speaker_alert = None;
                            match &go {
                                SpeakerAlert::Rename(from) => {
                                    let to = this.speaker_name.read(cx).value().to_string();
                                    this.rename_speaker_to(from, &to, window, cx);
                                }
                                SpeakerAlert::Merge(from, to) => this.rename_speaker_to(from, to, window, cx),
                            }
                            cx.notify();
                        }))
                    })),
            );
        Some(
            div()
                .id("speaker-alert-veil")
                .occlude()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(alpha_of(0, if dark { 0.45 } else { 0.2 }))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.speaker_alert = None;
                    cx.notify();
                }))
                .child(card)
                .into_any_element(),
        )
    }
}

fn alpha_of(color: u32, a: f32) -> gpui_kit::Hsla {
    let mut c: gpui_kit::Hsla = rgb(color).into();
    c.a = a;
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    /// Rename… renames to a new name, merges into a name someone has, and does nothing for an empty or the same one.
    #[test]
    fn a_typed_name_renames_or_merges() {
        let all = names(&["A", "B", "Li Wei"]);
        assert_eq!(rename_answer(&all, "A", "  "), RenameAnswer::Nothing);
        assert_eq!(rename_answer(&all, "A", "A"), RenameAnswer::Nothing);
        assert_eq!(rename_answer(&all, "A", " Mia "), RenameAnswer::Rename);
        assert_eq!(rename_answer(&all, "A", "Li Wei"), RenameAnswer::Merge);
    }

    /// Merge into lists everyone but the speaker, in the order they spoke.
    #[test]
    fn merge_into_lists_the_others() {
        let all = names(&["A", "B", "C"]);
        assert_eq!(merge_targets(&all, "B"), names(&["A", "C"]));
        assert!(merge_targets(&names(&["A"]), "A").is_empty());
    }
}
