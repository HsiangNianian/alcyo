//! The iPad's transcript and circle cards (#341): the transcript as a floating card that can be thrown to any
//! corner and folds into a one-line pill, or upright and in a narrow window as a sheet along the bottom with
//! three heights; and the card that comes up under a Pencil circle with what was caught, what is being said
//! about it, and the common things to do to it. A child of `app` beside ipad.rs, whose frame and glass it uses.

use super::ipad::{arrive, glass, spring, who, Corner, Frame, Grab, Mode, DOCK_H, MOVE};
use super::*;
use gpui_kit::{AnyElement, BoxShadow, FontWeight};

/// The transcript pill folded away at the corner.
const PILL_H: f32 = 56.0;
const PILL_W: f32 = 250.0;
/// The circle card's width at most, and how far below the circle it comes up.
const CIRCLE_CARD_W: f32 = 612.0;
const CIRCLE_CARD_H: f32 = 182.0;

fn alpha(color: u32, a: f32) -> Hsla {
    let mut c: Hsla = rgb(color).into();
    c.a = a;
    c
}

fn soft(strength: f32) -> Hsla {
    if theme::is_dark() { alpha(0xff_ff_ff, 0.1 * strength) } else { alpha(0x00_00_00, 0.06 * strength) }
}

/// What glass the card is: the same as the card's own fill, for the fade over its scrolled lines.
fn card_fill() -> Hsla {
    if theme::is_dark() { alpha(0x18_18_1a, 0.88) } else { alpha(0xff_ff_ff, 0.88) }
}

impl Workbench {
    /// Where the transcript card is, open or folded: its left, top, width and height on the screen. Open it is
    /// at its corner, full height at the top and shorter at the bottom; folded it is the pill beside the dock.
    pub(super) fn ipad_card_rect(&self, f: &Frame, open: bool) -> (f32, f32, f32, f32) {
        let left = matches!(self.ipad.corner, Corner::TopLeft | Corner::BottomLeft);
        let bottom = matches!(self.ipad.corner, Corner::BottomLeft | Corner::BottomRight);
        if !open {
            let x = if left { f.margin } else { f.w - f.margin - PILL_W };
            return (x, f.h - f.dock_bottom() - (DOCK_H + PILL_H) / 2.0, PILL_W, PILL_H);
        }
        let w = f.card_w();
        // Beside the palette when it is on the card's side.
        let palette = if left != self.ipad.tools_right_side() { 56.0 + 12.0 } else { 0.0 };
        let x = if left { f.margin + palette } else { f.w - f.margin - w - palette };
        let h = if bottom { f.card_h().min(380.0) } else { f.card_h() };
        let y = if bottom { f.h - f.dock_bottom() - DOCK_H - 16.0 - h } else { f.card_top() };
        // Clear of the on-screen keyboard, so the box being typed in stays in sight.
        let y = if f.keyboard > 0.0 { y.min(f.h - f.keyboard - 12.0 - h).max(f.top() + 10.0) } else { y };
        (x, y, w, h)
    }

    /// The transcript sheet's height now: where it rests, or where a finger has it, stretched past its ends
    /// with some resistance.
    pub(super) fn ipad_sheet_height(&self, f: &Frame) -> f32 {
        let detents = f.detents();
        match self.ipad.drag.as_ref().filter(|d| d.grab == Grab::Sheet && d.moved) {
            Some(drag) => {
                let h = drag.start.0 - drag.delta().1;
                let (low, high) = (detents[0], detents[2]);
                if h > high {
                    high + (h - high) * 0.3
                } else if h < low {
                    low - (low - h) * 0.3
                } else {
                    h
                }
            }
            None => {
                let at = self.ipad.sheet;
                detents.into_iter().min_by(|a, b| (a - at).abs().total_cmp(&(b - at).abs())).unwrap_or(detents[1])
            }
        }
    }

    /// Transcript, Summary and Speakers as the system's segmented control.
    fn ipad_tabs(&self, wide: bool, cx: &mut Context<Self>) -> gpui_kit::Div {
        let side_panel = if self.panel.beside_board() { self.panel } else { Panel::Transcript };
        let dark = theme::is_dark();
        let count = self.live.speakers().len();
        div()
            .flex_1()
            .flex()
            .p(px(2.))
            .gap(px(2.))
            .rounded(px(9.))
            .bg(alpha(0x76_76_80, if dark { 0.24 } else { 0.12 }))
            .children([(Panel::Transcript, "ipad-tab-transcript", t("transcript-2").to_string()), (Panel::Summary, "ipad-tab-summary", t("summary-2").to_string()), (Panel::Speakers, "ipad-tab-speakers", if wide && count > 0 { format!("{} {count}", t("speakers")) } else { t("speakers").to_string() })].map(|(panel, id, label)| {
                let on = side_panel == panel;
                div()
                    .id(id)
                    .flex_1()
                    .h(px(30.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .px(px(12.))
                    .rounded(px(7.))
                    .text_ui()
                    .font_weight(FontWeight::MEDIUM)
                    .whitespace_nowrap()
                    .cursor_pointer()
                    .when(on, |d| {
                        if dark {
                            d.bg(rgb(0x5a_5a_5e))
                        } else {
                            d.bg(rgb(pal().surface)).shadow(vec![
                                BoxShadow { color: alpha(0, 0.12), offset: point(px(0.), px(1.)), blur_radius: px(3.), spread_radius: px(0.), inset: false },
                                BoxShadow { color: alpha(0, 0.04), offset: point(px(0.), px(0.)), blur_radius: px(0.), spread_radius: px(0.5), inset: false },
                            ])
                        }
                    })
                    .child(label)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.panel = panel;
                        this.transcript_open = true;
                        cx.notify();
                    }))
            }))
    }

    /// The box to type a line for the board, with who it goes in as beside it.
    fn ipad_typing(&self, named: bool, cx: &mut Context<Self>) -> gpui_kit::Div {
        let dark = theme::is_dark();
        let speakers = self.live.speakers();
        let chip = self.speaker.clone().map(|name| {
            let picked = name.clone();
            div()
                .id("ipad-typing-as")
                .flex_none()
                .h(px(32.))
                .flex()
                .items_center()
                .gap(px(6.))
                .pl(px(4.))
                .pr(px(10.))
                .rounded_full()
                .bg(rgb(pal().surface))
                .shadow(vec![BoxShadow { color: if dark { alpha(0xff_ff_ff, 0.12) } else { alpha(0, 0.08) }, offset: point(px(0.), px(0.)), blur_radius: px(0.), spread_radius: px(0.5), inset: false }])
                .text_color(rgb(pal().text))
                .text_size(px(12.5))
                .font_weight(FontWeight::MEDIUM)
                .cursor_pointer()
                .child(who(&speakers, &name, 22., true))
                .when(named, |d| d.child(ta!("typing-as", name = name.clone())))
                .child(icon("ipad/chevron-down").size(px(14.)))
                .on_click(cx.listener(move |this, _, window, cx| this.choose_speaker(picked.clone(), window, cx)))
        });
        div()
            .h(px(44.))
            .flex()
            .items_center()
            .gap(px(8.))
            .pl(px(14.))
            .pr(px(6.))
            .rounded(px(14.))
            .bg(alpha(0x76_76_80, 0.1))
            .child(div().flex_1().min_w_0().child(Input::new(&self.input).appearance(false)))
            .when_some(chip, |d, chip| d.child(chip))
    }

    /// The body of the transcript, summary or speakers, with the transcript's oldest lines fading under the
    /// top edge as they scroll away.
    fn ipad_card_body(&self, transcript: AnyElement, summary: Option<AnyElement>, speakers: Option<AnyElement>) -> Option<gpui_kit::Div> {
        let side_panel = if self.panel.beside_board() { self.panel } else { Panel::Transcript };
        let body = match side_panel {
            Panel::Summary => summary,
            Panel::Speakers => speakers,
            _ => Some(transcript),
        }?;
        let fill = card_fill();
        let mut clear = fill;
        clear.a = 0.0;
        Some(
            div()
                .relative()
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                .child(body)
                .when(side_panel == Panel::Transcript, |d| d.child(div().absolute().top_0().left_0().right_0().h(px(36.)).bg(gpui_kit::linear_gradient(180., gpui_kit::linear_color_stop(fill, 0.), gpui_kit::linear_color_stop(clear, 1.))))),
        )
    }

    /// The transcript: in landscape a card at its corner that a drag throws to another and that folds into a
    /// pill, the one surface morphing between the two; upright and in a narrow window a sheet along the bottom.
    pub(super) fn ipad_transcript(&self, f: &Frame, transcript: AnyElement, summary: Option<AnyElement>, speakers: Option<AnyElement>, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        if f.mode != Mode::Wide {
            return self.ipad_sheet(f, transcript, summary, speakers, window, cx);
        }
        let open = self.side_shown();
        let dragging = self.ipad.drag.as_ref().filter(|d| d.grab == Grab::Card && d.moved);
        let (mut x, mut y, w, h) = self.ipad_card_rect(f, open);
        if let Some(drag) = dragging {
            let (dx, dy) = drag.delta();
            (x, y) = (drag.start.0 + dx, drag.start.1 + dy);
        }
        let speakers_ = self.live.speakers();
        // The folded pill's face: what is being said now, or the last line said.
        let newest = self.draft.as_ref().map(|d| (self.speaker.clone(), d.text.clone())).or_else(|| {
            self.live.lines.iter().rev().find(|l| !l["manual"].as_bool().unwrap_or(false)).map(|l| (l["speaker"].as_str().filter(|s| !s.is_empty()).map(str::to_string), l["text"].as_str().unwrap_or_default().to_string()))
        });
        let (name, text) = newest.unwrap_or((None, t("transcript-2").to_string()));
        let pill = div()
            .id("ipad-transcript-pill")
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .gap(px(10.))
            .pl(px(12.))
            .pr(px(10.))
            .cursor_pointer()
            .text_size(px(14.))
            .when_some(name, |d, name| d.child(who(&speakers_, &name, 24., true)))
            .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().text_color(rgb(pal().muted)).child(text))
            .child(div().flex_none().child(icon("ipad/chevron-up").size(px(18.))))
            .on_click(cx.listener(|this, _, _, cx| this.toggle_side(cx)));
        let header = div()
            .relative()
            .flex_none()
            .flex()
            .items_center()
            .gap(px(8.))
            .pt(px(12.))
            .pr(px(10.))
            .pb(px(8.))
            .pl(px(14.))
            // The header carries the card: a drag on it throws the card to a corner.
            .child(self.ipad_drag_area(Grab::Card, cx))
            .child(self.ipad_tabs(false, cx))
            .child(
                div()
                    .id("ipad-card-collapse")
                    .flex_none()
                    .size(px(36.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .cursor_pointer()
                    .hover(|d| d.bg(soft(1.)))
                    .child(icon("ipad/chevron-down").size(px(18.)))
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_side(cx))),
            );
        let side_panel = if self.panel.beside_board() { self.panel } else { Panel::Transcript };
        let typing = (side_panel == Panel::Transcript && self.editable()).then(|| div().flex_none().p(px(12.)).child(self.ipad_typing(false, cx)));
        let card = div()
            .absolute()
            .top_0()
            .left_0()
            .w(px(f.card_w()))
            .h(px(f.card_h()))
            .flex()
            .flex_col()
            .child(header)
            .when_some(self.ipad_card_body(transcript, summary, speakers), |d, body| d.child(body))
            .when_some(typing, |d, typing| d.child(typing));
        // One surface, morphing: its size and the corner it sits at spring to where it is going, and the card's
        // face and the pill's cross-fade on it.
        let faces = div().relative().size_full().with_spring("ipad-card-open", SpringAnimation::new(MOVE).to(open), move |d, v| {
            let t = v.0.clamp(0., 1.);
            // Each face shows only near its own end of the morph, so neither is caught half under the other.
            let card_t = ((t - 0.35) / 0.65).clamp(0., 1.);
            let pill_t = ((0.5 - t) / 0.5).clamp(0., 1.);
            d.child(div().absolute().inset_0().opacity(card_t).when(card_t < 0.05, |d| d.invisible()).child(card))
                .child(div().absolute().inset_0().opacity(pill_t).when(pill_t < 0.05, |d| d.invisible()).child(pill))
        });
        let surface = glass(div(), true).id("ipad-card").occlude().h_full().overflow_hidden().rounded(px(26.)).child(faces).with_spring("ipad-card-w", spring(MOVE, w, false), |d, v| d.w(px(v)));
        let sized = div().child(surface).with_spring("ipad-card-h", spring(MOVE, h, false), |d, v| d.h(px(v)));
        let snap = dragging.is_some();
        div()
            .absolute()
            .top_0()
            .left_0()
            .child(div().child(arrive("ipad-card-in", div().child(sized), 24., 0., 0.12)).with_spring("ipad-card-y", spring(MOVE, y, snap), |d, v| d.relative().top(px(v))))
            .with_spring("ipad-card-x", spring(MOVE, x, snap), |d, v| d.relative().left(px(v)))
            .into_any_element()
    }

    /// Upright, and in a narrow window: the transcript as a sheet along the bottom with a grabber, resting at a
    /// one-line peek, half, or nearly the whole height, sprung between them and stretched with resistance
    /// past its ends. The dock rides its top edge.
    fn ipad_sheet(&self, f: &Frame, transcript: AnyElement, summary: Option<AnyElement>, speakers: Option<AnyElement>, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let compact = matches!(f.mode, Mode::Compact { .. });
        // In a narrow window the sheet comes out from the speech bubble, and otherwise the newest line shows.
        if compact && !self.side_shown() {
            return div().into_any_element();
        }
        let height = self.ipad_sheet_height(f) + f.keyboard;
        let dragging = self.ipad.drag.as_ref().is_some_and(|d| d.grab == Grab::Sheet && d.moved);
        let side_panel = if self.panel.beside_board() { self.panel } else { Panel::Transcript };
        let typing = (side_panel == Panel::Transcript && self.editable()).then(|| div().flex_none().pt(px(12.)).px(px(16.)).pb(px(f.dock_bottom().max(30.0) + f.keyboard)).child(self.ipad_typing(true, cx)));
        let sheet = glass(div(), true)
            .id("ipad-sheet")
            .occlude()
            .size_full()
            .rounded_t(px(30.))
            .overflow_hidden()
            .flex()
            .flex_col()
            .child(
                div()
                    .relative()
                    .flex_none()
                    .flex()
                    .flex_col()
                    .child(self.ipad_drag_area(Grab::Sheet, cx))
                    .child(div().mx_auto().mt(px(8.)).w(px(40.)).h(px(5.)).rounded(px(3.)).bg(soft(2.3)))
                    .child(div().flex().items_center().gap(px(10.)).pt(px(if compact { 14. } else { 30. })).pb(px(4.)).px(px(16.)).child(self.ipad_tabs(true, cx)).when(compact, |d| {
                        d.child(
                            div()
                                .id("ipad-sheet-close")
                                .flex_none()
                                .size(px(36.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded_full()
                                .cursor_pointer()
                                .child(icon("ipad/chevron-down").size(px(18.)))
                                .on_click(cx.listener(|this, _, _, cx| this.toggle_side(cx))),
                        )
                    })),
            )
            .when_some(self.ipad_card_body(transcript, summary, speakers), |d, body| d.child(div().flex_1().min_h_0().flex().flex_col().px(px(6.)).child(body)))
            .when_some(typing, |d, typing| d.child(typing));
        // In a narrow window it sits on the bottom bar.
        let above = if compact { 68.0 + (f.dock_bottom() - 16.0).max(0.0) } else { 0.0 };
        div()
            .absolute()
            .left_0()
            .right_0()
            .bottom(px(above))
            .child(arrive("ipad-sheet-in", div().size_full().child(sheet), 0., 40., 0.1))
            .with_spring("ipad-sheet-h", spring(MOVE, height, dragging), |d, v| d.h(px(v)))
            .into_any_element()
    }

    /// The card that comes up under a Pencil circle: how many were caught and which, what is being said now as
    /// the command for them, and the common things to do: highlight, group, a note, a board of their own, or
    /// away. It rises from the circle's lowest point, or opens above it where there is no room below.
    pub(super) fn ipad_circle_card(&self, f: &Frame, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (bounds, pointed) = {
            let ink = self.ink.borrow();
            if !ink.circling || ink.pointed.is_empty() {
                return None;
            }
            let v = ink.shown();
            let loop_ = ink.loops.last()?;
            let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
            for p in loop_ {
                let sx = f32::from(ink.canvas.origin.x) + (p[0] - v.x) * v.k;
                let sy = f32::from(ink.canvas.origin.y) + (p[1] - v.y) * v.k;
                (x0, y0, x1, y1) = (x0.min(sx), y0.min(sy), x1.max(sx), y1.max(sy));
            }
            ((x0, y0, x1, y1), ink.pointed.clone())
        };
        let names: Vec<String> = pointed.iter().filter_map(|id| self.live.graph.nodes.iter().find(|n| &n.id == id)).map(|n| n.label.clone()).collect();
        let w = CIRCLE_CARD_W.min(f.w - 2.0 * f.margin);
        let floor = f.h - f.dock_bottom() - DOCK_H - 12.0;
        let below = bounds.3 + 16.0;
        let (top, rises) = if below + CIRCLE_CARD_H <= floor { (below, 18.0) } else { ((bounds.1 - 16.0 - CIRCLE_CARD_H).max(f.top() + 70.0), -18.0) };
        let left = ((bounds.0 + bounds.2) / 2.0 - w / 2.0).clamp(f.margin, f.w - f.margin - w);
        let listening = self.pipeline.listening();
        let said = self.draft.as_ref().map(|d| d.text.clone());
        let accent = pal().accent;
        let level = self.ipad.level;
        let button = |id: &'static str, icon_name: &'static str, label: &'static str, danger: bool| {
            div()
                .id(id)
                .flex_none()
                .h(px(44.))
                .flex()
                .items_center()
                .gap(px(7.))
                .px(px(14.))
                .rounded_full()
                .bg(soft(1.))
                .text_size(px(14.))
                .font_weight(FontWeight::SEMIBOLD)
                .whitespace_nowrap()
                .cursor_pointer()
                .active(|d| d.bg(soft(2.2)))
                .when(danger, |d| d.text_color(rgb(pal().danger)).ml_auto())
                .child(icon(&format!("ipad/{icon_name}")).size(px(16.)))
                .child(label)
        };
        let ids = pointed.clone();
        // Over ink, so a touch more solid than the other glass: what is under it must not read through.
        let mut solid = card_fill();
        solid.a = 0.96;
        let card = glass(div(), true)
            .bg(solid)
            .id("ipad-circle-card")
            .occlude()
            .w(px(w))
            .p(px(16.))
            .rounded(px(24.))
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .text_size(px(14.))
                    .text_color(rgb(pal().muted))
                    .child(div().text_color(rgb(accent)).child(icon("ipad/circle").size(px(16.))))
                    .child(div().text_color(rgb(pal().text)).font_weight(FontWeight::SEMIBOLD).child(ta!("ipad-circled", n = pointed.len())))
                    .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().child(names.join(", "))),
            )
            // What is being said now is the command; it types in as the words arrive.
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .px(px(14.))
                    .py(px(12.))
                    .rounded(px(16.))
                    .bg(alpha(accent, 0.1))
                    .text_color(rgb(accent))
                    .text_size(px(16.))
                    .child(div().flex().items_center().gap(px(2.5)).h(px(16.)).children((0..4).map(|i| {
                        let h = if listening { 4.0 + level * 12.0 * (0.55 + 0.45 * ((i as f32) * 1.9).sin().abs()) } else { 4.0 };
                        div().w(px(3.)).h(px(h.min(16.))).rounded(px(2.)).bg(rgb(accent))
                    })))
                    .child(match said {
                        Some(text) => div().overflow_hidden().whitespace_nowrap().text_ellipsis().child(format!("“{text}”▍")).into_any_element(),
                        None => div().opacity(0.75).child(if listening { t("ipad-say-what-to-do") } else { t("ipad-listen-to-say") }).into_any_element(),
                    }),
            )
            .child(
                div()
                    .flex()
                    .gap(px(8.))
                    .child(button("ipad-circle-highlight", "mark", t("ipad-highlight"), false).on_click(cx.listener({
                        let ids = ids.clone();
                        move |this, _, _, cx| {
                            let ops: Vec<Op> = ids.iter().map(|id| Op::M(Ref::Id(id.clone()), "高亮".into(), None)).collect();
                            this.ipad_circle_done(&ops, cx);
                        }
                    })))
                    .child(button("ipad-circle-group", "tidy", t("ipad-group"), false).on_click(cx.listener({
                        let ids = ids.clone();
                        move |this, _, _, cx| {
                            let ops = vec![Op::G(t("ipad-group-name").to_string(), ids.iter().map(|id| Ref::Id(id.clone())).collect())];
                            this.ipad_circle_done(&ops, cx);
                        }
                    })))
                    .child(button("ipad-circle-note", "transcript", t("ipad-note"), false).on_click(cx.listener({
                        let ids = ids.clone();
                        move |this, _, window, cx| {
                            let Some(first) = ids.first().cloned() else { return };
                            let note = t("ipad-note-text").to_string();
                            this.ipad_circle_done(&[Op::K(Ref::Id(first.clone()), note.clone())], cx);
                            // The note is written straight away, in its own words.
                            let made = this.live.graph.nodes.iter().find(|n| n.label == note && n.parent.as_deref() == Some(first.as_str())).map(|n| n.id.clone());
                            if let Some(id) = made {
                                this.ink.borrow_mut().selected = Some(id.clone());
                                this.start_label(&id, window, cx);
                            }
                        }
                    })))
                    .child(button("ipad-circle-board", "crop", t("ipad-new-board"), false).on_click(cx.listener({
                        let ids = ids.clone();
                        move |this, _, _, cx| {
                            if !this.editable() || this.replay.is_some() {
                                return;
                            }
                            let label = ids.iter().filter_map(|id| this.live.graph.nodes.iter().find(|n| &n.id == id)).map(|n| n.label.clone()).collect::<Vec<_>>().join(", ");
                            this.ink.borrow_mut().clear_circles();
                            if this.live.split_nodes(&ids, &crate::note!("note-moved-to-new-board", t = label)) {
                                this.show_new_board(cx);
                            }
                            this.ink.borrow_mut().circling = true;
                            cx.notify();
                        }
                    })))
                    .child(button("ipad-circle-remove", "x", t("ipad-remove"), true).on_click(cx.listener({
                        let ids = ids.clone();
                        move |this, _, _, cx| {
                            let ops: Vec<Op> = ids.iter().map(|id| Op::D(Ref::Id(id.clone()))).collect();
                            this.ipad_circle_done(&ops, cx);
                        }
                    }))),
            );
        Some(
            div()
                .absolute()
                .left(px(left))
                .top(px(top))
                .child(arrive(gpui_kit::ElementId::Name(format!("ipad-circle-card-{}", pointed.join("-")).into()), div().child(card), 0., rises, 0.08))
                .into_any_element(),
        )
    }

    /// A button on the circle card did its work: the board changes, and the circle goes, the Pencil still
    /// holding the circle tool for the next.
    fn ipad_circle_done(&mut self, ops: &[Op], cx: &mut Context<Self>) {
        self.edit_board(ops, cx);
        let mut ink = self.ink.borrow_mut();
        ink.clear_circles();
        ink.circling = true;
        drop(ink);
        cx.notify();
    }
}
