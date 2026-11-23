//! The iPad's meeting screen, "canvas first" (#341): the board takes the whole screen and everything else
//! floats over it on glass. The meeting's card at the top left, how the board model is doing in a pill at the
//! top, what is done with the whole meeting at the top right, the Pencil's tools down the left, the transcript
//! as a card that folds into a pill, and one dock in reach of a thumb along the bottom. While listening, the
//! bars step back to a third after a few seconds without a touch; the listening capsule never does.
//!
//! Only the layout lives here. What each control does is the workbench's own, shared with the desktop, which
//! lays the same parts out in its own way (`platform::ipad_layout` chooses). A child of `app`, so it reaches
//! the workbench's state directly.

use super::*;
use gpui_kit::{AnyElement, BoxShadow, Edges, FontWeight};

/// How long without a touch, while listening, before the bars step back, and how far they go.
const FADE_AFTER: Duration = Duration::from_secs(3);
const FADED: f32 = 0.3;
/// Every touch target is at least this big.
const TARGET: f32 = 44.0;
/// The tool palette's width, and the dock's height.
const PALETTE_W: f32 = 56.0;
const DOCK_H: f32 = 60.0;
/// The meeting's card at the top left, and the room under it before the transcript card starts.
const CARD_H: f32 = 52.0;
/// The transcript pill folded away at the corner.
const PILL_H: f32 = 56.0;
const PILL_W: f32 = 250.0;

/// What the iPad's screen keeps for itself, beside the workbench's own state.
pub(super) struct Chrome {
    /// The meetings list, floating over the board's left edge, and what was open when it came out: once
    /// something else is chosen from it, it goes away again.
    sidebar: Option<(Panel, String)>,
    /// The tool palette is out, rather than folded into a puck.
    tools_open: bool,
    /// When the screen was last touched, and whether the bars stepped back at the last frame. Shared with the
    /// window's event handlers, which note touches before any view hears of them.
    touched: Rc<Cell<Instant>>,
    faded: Rc<Cell<bool>>,
}

impl Chrome {
    pub(super) fn new() -> Self {
        Self { sidebar: None, tools_open: true, touched: Rc::new(Cell::new(Instant::now())), faded: Rc::new(Cell::new(false)) }
    }

    /// Whether the bars should be stepped back now.
    fn fading(&self, listening: bool) -> bool {
        listening && self.touched.get().elapsed() >= FADE_AFTER
    }

    /// Whether the bars should step back or come forward since the last frame, so it is drawn again.
    pub(super) fn fade_changed(&self, listening: bool) -> bool {
        self.fading(listening) != self.faded.get()
    }
}

/// The sizes of the screen and what floats on it: an 11" iPad's, or a 13" one's, which has more room.
struct Frame {
    h: f32,
    safe: Edges<Pixels>,
    /// How far from the screen's sides the floating things are.
    margin: f32,
    big: bool,
}

impl Frame {
    fn of(window: &Window) -> Self {
        let size = window.viewport_size();
        let (w, h) = (f32::from(size.width), f32::from(size.height));
        let big = w >= 1300.0;
        Self { h, safe: crate::platform::safe_area(window), margin: if big { 24.0 } else { 20.0 }, big }
    }

    fn top(&self) -> f32 {
        f32::from(self.safe.top)
    }

    /// From the bottom of the screen to the dock: clear of the home indicator.
    fn dock_bottom(&self) -> f32 {
        (f32::from(self.safe.bottom) + if self.big { 6.0 } else { 2.0 }).max(16.0)
    }

    fn card_w(&self) -> f32 {
        if self.big { 372.0 } else { 330.0 }
    }

    fn card_top(&self) -> f32 {
        self.top() + if self.big { 86.0 } else { 76.0 }
    }

    /// The transcript card, as tall as the screen allows above the dock.
    fn card_h(&self) -> f32 {
        let room = self.h - self.card_top() - self.dock_bottom() - DOCK_H - 24.0;
        (if self.big { 700.0_f32 } else { 600.0 }).min(room).max(240.0)
    }

    fn palette_top(&self) -> f32 {
        (self.h * 0.29).round()
    }
}

/// A colour with some see-through.
fn alpha(color: u32, a: f32) -> Hsla {
    let mut c: Hsla = rgb(color).into();
    c.a = a;
    c
}

/// A soft fill for something chosen or pressed on glass, in the light or the dark.
fn soft(strength: f32) -> Hsla {
    if theme::is_dark() { alpha(0xff_ff_ff, 0.1 * strength) } else { alpha(0x00_00_00, 0.06 * strength) }
}

/// Glass: a see-through surface with a hairline of light inside its edge and a hairline of shade outside it,
/// standing off the board on a soft shadow. Dark glass in the dark, never a white slab. The card that holds
/// the transcript is a little more solid than the bars, as it is read.
pub(super) fn glass<E: gpui_kit::Styled>(el: E, card: bool) -> E {
    let dark = theme::is_dark();
    // GPUI has no backdrop blur yet, so the glass is a little more solid than the design's, to keep the
    // board's dots from showing through as sharply as they would through frosted glass.
    let fill = match (dark, card) {
        (false, false) => alpha(0xff_ff_ff, 0.82),
        (false, true) => alpha(0xff_ff_ff, 0.88),
        (true, false) => alpha(0x1c_1c_1e, 0.82),
        (true, true) => alpha(0x18_18_1a, 0.88),
    };
    let rim = BoxShadow { color: if dark { alpha(0xff_ff_ff, 0.10) } else { alpha(0xff_ff_ff, 0.85) }, offset: point(px(0.), px(0.)), blur_radius: px(0.), spread_radius: px(0.5), inset: true };
    let shade = |a: f32, y: f32, blur: f32| BoxShadow { color: alpha(0, a), offset: point(px(0.), px(y)), blur_radius: px(blur), spread_radius: px(0.), inset: false };
    let shadows = if dark { vec![rim, shade(0.3, 1., 1.), shade(0.5, 10., 32.)] } else { vec![rim, shade(0.04, 1., 1.), shade(0.10, 8., 28.)] };
    el.bg(fill).border(px(0.5)).border_color(if dark { alpha(0, 0.6) } else { alpha(0, 0.08) }).shadow(shadows)
}

/// A round 44 pt button with a line icon, filled in ink when `on`.
fn round_button(id: impl Into<gpui_kit::ElementId>, name: &str, on: bool) -> gpui_kit::Stateful<gpui_kit::Div> {
    let p = pal();
    div()
        .id(id)
        .flex_none()
        .size(px(TARGET))
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .cursor_pointer()
        .map(|d| if on { d.bg(rgb(p.text)).text_color(rgb(p.paper)) } else { d.text_color(rgb(p.text)).hover(|d| d.bg(soft(1.))).active(|d| d.bg(soft(2.))) })
        .child(icon(&format!("ipad/{name}")).size(px(22.)))
}

/// A hairline between groups on glass.
fn rule() -> gpui_kit::Div {
    div().flex_none().w(px(1.)).h(px(22.)).bg(if theme::is_dark() { alpha(0xff_ff_ff, 0.12) } else { alpha(0, 0.1) })
}

/// The colours of a speaker's letter, which follow the order people first spoke in; deeper in the dark.
fn speaker_tones(speakers: &[String], name: &str) -> (Hsla, Hsla) {
    let (background, ink) = speaker_colors(speakers, name);
    if theme::is_dark() {
        (motion::mix(pal().paper, ink, 0.28), motion::mix(ink, 0xff_ff_ff, 0.6))
    } else {
        (rgb(background).into(), rgb(ink).into())
    }
}

/// A speaker as a round letter, `size` across: filled once their line is on the board, in outline before.
pub(super) fn who(speakers: &[String], name: &str, size: f32, drawn: bool) -> gpui_kit::Div {
    let (background, ink) = speaker_tones(speakers, name);
    div()
        .flex_none()
        .size(px(size))
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .text_size(px(size / 2.0))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(ink)
        .map(|d| if drawn { d.bg(background) } else { d.border_1().border_color(ink) })
        .child(initial(name))
}

/// What the layout takes from the workbench's render: the parts it builds the same way for both layouts.
pub(super) struct Parts {
    pub board: AnyElement,
    pub ground: AnyElement,
    pub welcome: Option<AnyElement>,
    pub tray: Option<AnyElement>,
    pub offers: AnyElement,
    pub notices: Option<AnyElement>,
    /// Meetings or Settings, over the board.
    pub page: Option<AnyElement>,
    /// Export, Share or Import, over everything.
    pub dialog: Option<AnyElement>,
    pub preparing: Option<AnyElement>,
    pub replay_bar: Option<AnyElement>,
    pub shortcuts: Option<AnyElement>,
    pub transcript: AnyElement,
    pub summary: Option<AnyElement>,
    pub speakers: Option<AnyElement>,
    /// Lines waiting for the board, and those it could not draw.
    pub waiting: usize,
    pub not_drawn: Vec<usize>,
}

impl Workbench {
    /// What floats over each side of the board on the iPad, for the board to fit between: the palette, the
    /// meeting's card, the transcript card when it is out, and the dock.
    pub(super) fn ipad_insets(&self, window: &Window) -> [f32; 4] {
        let f = Frame::of(window);
        let palette = if self.ipad.tools_open { PALETTE_W } else { TARGET };
        let right = if self.side_shown() { f.card_w() + f.margin } else { f.margin };
        [f.margin + palette + BOARD_MARGIN, f.top() + 10.0 + CARD_H + BOARD_MARGIN, right + BOARD_MARGIN, f.dock_bottom() + DOCK_H + BOARD_MARGIN]
    }

    pub(super) fn ipad_sidebar_shown(&self) -> bool {
        self.ipad.sidebar.is_some()
    }

    pub(super) fn ipad_toggle_sidebar(&mut self) {
        self.ipad.sidebar = match self.ipad.sidebar {
            Some(_) => None,
            None => Some((self.panel, self.live.id().to_string())),
        };
    }

    /// The board model, by a name to say it is drawing.
    fn model_name(&self) -> String {
        match self.session().provider() {
            Ok(Provider::Remote { base_url, model, .. }) if base_url.contains("deepseek") || model.contains("deepseek") => "DeepSeek".into(),
            Ok(Provider::Remote { model, .. }) if !model.is_empty() => model,
            _ => alcyo_models::current(&self.models)
                .and_then(|path| {
                    let file = path.file_name()?.to_str()?.to_string();
                    alcyo_models::catalog().into_iter().find(|m| m.file == file).map(|m| m.label.to_string())
                })
                .unwrap_or_else(|| t("board").to_string()),
        }
    }

    /// The whole screen, on the iPad.
    pub(super) fn ipad_screen(&mut self, parts: Parts, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let f = Frame::of(window);
        let listening = self.pipeline.listening();
        // A page or another meeting chosen from the list puts the list away.
        if self.ipad.sidebar.as_ref().is_some_and(|(panel, id)| *panel != self.panel || id != self.live.id()) {
            self.ipad.sidebar = None;
        }
        let fading = self.ipad.fading(listening) && self.panel.beside_board() && self.ipad.sidebar.is_none();
        self.ipad.faded.set(fading);
        // The bars step back together; what is faded is drawn through this.
        let fade = |id: &'static str, el: gpui_kit::Div| el.with_spring(id, SpringAnimation::new(motion::SETTLE).to(fading), |d, v| d.opacity(1. - (1. - FADED) * v.0.clamp(0., 1.)));
        // A page over the board (Meetings, Settings) keeps the meeting's card for the way back, and nothing else.
        let page_open = parts.page.is_some();
        let Parts { board, ground, welcome, tray, offers, notices, page, dialog, preparing, replay_bar, shortcuts, transcript, summary, speakers, waiting, not_drawn } = parts;
        let replaying = replay_bar.is_some();

        // Every touch anywhere counts, before any element hears of it, so the bars come back at once.
        let (touched, faded) = (self.ipad.touched.clone(), self.ipad.faded.clone());
        let watch = canvas(
            |_, _, _| (),
            move |_, (), window: &mut Window, _| {
                let note = |touched: &Rc<Cell<Instant>>, faded: &Rc<Cell<bool>>, window: &mut Window| {
                    touched.set(Instant::now());
                    if faded.get() {
                        window.refresh();
                    }
                };
                let (t1, f1) = (touched.clone(), faded.clone());
                window.on_mouse_event(move |_: &MouseDownEvent, phase, window, _| {
                    if phase == gpui_kit::DispatchPhase::Capture {
                        note(&t1, &f1, window);
                    }
                });
                let (t2, f2) = (touched.clone(), faded.clone());
                window.on_mouse_event(move |_: &MouseMoveEvent, phase, window, _| {
                    if phase == gpui_kit::DispatchPhase::Capture {
                        note(&t2, &f2, window);
                    }
                });
                let (t3, f3) = (touched.clone(), faded.clone());
                window.on_mouse_event(move |_: &gpui_kit::TouchDragEvent, phase, window, _| {
                    if phase == gpui_kit::DispatchPhase::Capture {
                        note(&t3, &f3, window);
                    }
                });
                let (t4, f4) = (touched.clone(), faded.clone());
                window.on_mouse_event(move |_: &ScrollEvent, phase, window, _| {
                    if phase == gpui_kit::DispatchPhase::Capture {
                        note(&t4, &f4, window);
                    }
                });
            },
        )
        .absolute()
        .size_full();

        let meeting_card = self.ipad_meeting_card(&f, cx);
        let status = (!page_open && !replaying).then(|| self.ipad_status(&f, waiting, &not_drawn, cx)).flatten();
        let status_shown = status.is_some();
        let actions = (!page_open).then(|| self.ipad_actions(&f, cx));
        let palette = (!page_open && !replaying && self.editable()).then(|| self.ipad_palette(&f, cx));
        let card = (!page_open).then(|| self.ipad_card(&f, transcript, summary, speakers, cx));
        let dock = (!page_open).then(|| self.ipad_dock(&f, replay_bar, fading, cx));
        let sidebar = self.ipad.sidebar.is_some().then(|| {
            let top = f.top() + 10.0 + CARD_H + 10.0;
            div()
                .absolute()
                .size_full()
                // A tap beside the list puts it away.
                .child(div().id("ipad-sidebar-veil").absolute().size_full().on_click(cx.listener(|this, _, _, cx| {
                    this.ipad.sidebar = None;
                    cx.notify();
                })))
                .child(
                    glass(div(), true)
                        .id("ipad-sidebar")
                        .occlude()
                        .absolute()
                        .left(px(f.margin))
                        .top(px(top))
                        .bottom(px(f.dock_bottom()))
                        .w(px(SIDEBAR_W))
                        .rounded(px(22.))
                        .overflow_hidden()
                        .child(self.sidebar(listening, cx).size_full()),
                )
        });
        // Under the status pill: the way back to the newest ink, and a board the talk moved to.
        let offers_top = f.top() + 20.0 + if status_shown { 32.0 + 10.0 } else { 0.0 };
        // The notices place themselves 12 pt down, or under the offers when there are some.
        let notices_top = offers_top - 12.0;
        // Kept between the palette and the transcript card, never over either.
        let (left_room, right_room) = (f.margin + PALETTE_W + 8.0, if self.side_shown() { f.margin + f.card_w() + 8.0 } else { f.margin });

        div()
            .id("workbench")
            .track_focus(&self.focus)
            .font(theme::interface_font())
            .on_key_down(cx.listener(Self::on_key))
            .on_drop(cx.listener(|this, paths: &gpui_kit::ExternalPaths, _, cx| this.import_paths(paths.paths(), cx)))
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(rgb(pal().paper))
            .text_color(rgb(pal().text))
            .child(watch)
            .child(board)
            .child(div().absolute().size_full().child(ground))
            .when_some(welcome, |d, welcome| d.child(div().absolute().top_0().bottom_0().left(px(left_room)).right(px(right_room)).child(welcome)))
            .when_some(tray, |d, tray| d.child(tray))
            .child(div().absolute().top(px(offers_top)).left(px(left_room)).right(px(right_room)).flex().justify_center().child(offers))
            .when_some(notices, |d, notices| d.child(div().absolute().top(px(notices_top)).left(px(left_room)).right(px(right_room)).child(notices)))
            .when_some(palette, |d, palette| d.child(fade("ipad-fade-palette", palette)))
            .when_some(card, |d, card| d.child(card))
            .when_some(dock, |d, dock| d.child(dock))
            .when_some(status, |d, status| d.child(fade("ipad-fade-status", status)))
            .when_some(actions, |d, actions| d.child(fade("ipad-fade-actions", actions)))
            // Meetings and Settings lie over the board and under the meeting's card, which leads back.
            .when_some(page, |d, page| d.child(div().absolute().size_full().pt(px(f.top() + 10.0 + CARD_H)).bg(rgb(pal().paper)).child(div().relative().size_full().child(page))))
            .when_some(sidebar, |d, sidebar| d.child(sidebar))
            .child(fade("ipad-fade-meeting", meeting_card))
            .when_some(preparing, |d, page| d.child(page))
            .when_some(dialog, |d, dialog| d.child(dialog))
            .when_some(shortcuts, |d, window| d.child(window))
            .into_any_element()
    }

    /// The meeting's card at the top left: the meetings list, and the meeting's name over when it began and
    /// how many spoke. A page over the board is named there instead.
    fn ipad_meeting_card(&self, f: &Frame, cx: &mut Context<Self>) -> gpui_kit::Div {
        let page_name = match self.panel {
            Panel::Library => Some(t("meetings")),
            Panel::Settings => Some(t("settings")),
            _ => None,
        };
        let started = self.live.created_at().and_then(|ms| {
            use chrono::TimeZone;
            let at = chrono::Local.timestamp_millis_opt(ms as i64).single()?;
            let days = (chrono::Local::now().date_naive() - at.date_naive()).num_days();
            let day = match days {
                0 => t("today-2").to_string(),
                1 => t("yesterday-2").to_string(),
                _ => at.format("%m-%d").to_string(),
            };
            Some(format!("{day} {}", at.format("%H:%M")))
        });
        let speakers = self.live.speakers().len();
        let editable = self.editable();
        let title: AnyElement = if let Some(name) = page_name {
            div().pl_1().text_title().font_weight(FontWeight::SEMIBOLD).child(name).into_any_element()
        } else if self.editing_title {
            div().w(px(240.)).child(Input::new(&self.title_input).appearance(false)).into_any_element()
        } else {
            div()
                .id("ipad-meeting-title")
                .flex()
                .items_center()
                .min_w_0()
                .when(editable, |d| d.cursor_pointer().on_click(cx.listener(|this, _, window, cx| this.edit_title(window, cx))))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .min_w_0()
                        .pl_1()
                        .line_height(gpui_kit::relative(1.15))
                        .child(div().max_w(px(260.)).overflow_hidden().whitespace_nowrap().text_ellipsis().text_title().font_weight(FontWeight::SEMIBOLD).child(self.live.title().to_string()))
                        .when_some(started, |d, when| {
                            let line = if speakers > 0 { ta!("ipad-meeting-started", when = when, n = speakers) } else { when };
                            d.child(div().text_size(px(11.5)).text_color(rgb(pal().faint)).whitespace_nowrap().child(line))
                        }),
                )
                .child(div().ml(px(6.)).text_color(rgb(pal().faint)).child(icon("ipad/chevron-down").size(px(14.))))
                .into_any_element()
        };
        glass(div(), false)
            .occlude()
            .absolute()
            .left(px(f.margin))
            .top(px(f.top() + 10.0))
            .h(px(CARD_H))
            .flex()
            .items_center()
            .gap(px(4.))
            .pl(px(4.))
            .pr(px(14.))
            .rounded_full()
            .child(round_button("ipad-sidebar-toggle", "sidebar", false).on_click(cx.listener(|this, _, _, cx| {
                this.ipad_toggle_sidebar();
                cx.notify();
            })))
            .child(title)
    }

    /// How the board stands, in a pill at the top: the model drawing, and the lines it could not draw, which
    /// a tap shows in the transcript. Nothing while all is drawn and the model is ready.
    fn ipad_status(&self, f: &Frame, waiting: usize, not_drawn: &[usize], cx: &mut Context<Self>) -> Option<gpui_kit::Div> {
        if !self.editable() {
            return None;
        }
        let (text, busy): (Option<String>, bool) = match () {
            _ if waiting > 0 && self.preparing.is_some() => (Some(ta!("count-waiting-for-the-model", n = waiting)), true),
            _ if waiting > 0 => (Some(ta!("ipad-model-drawing", model = self.model_name())), true),
            _ if self.needs_model => (Some(t("choose-a-board-model").to_string()), false),
            _ => (self.status.text().map(str::to_string), matches!(self.status, ModelStatus::Starting | ModelStatus::Loading)),
        };
        let missed = not_drawn.last().copied().map(|line| (ta!("not-drawn-count-show", n = not_drawn.len()), line));
        if text.is_none() && missed.is_none() {
            return None;
        }
        let needs_model = self.needs_model;
        let sparkle = div().text_color(rgb(if needs_model { pal().warning } else { pal().accent })).child(icon("ipad/sparkle").size(px(18.)));
        Some(
            div().absolute().top(px(f.top() + 20.0)).left_0().right_0().flex().justify_center().child(
                glass(div(), false)
                    .id("ipad-status")
                    .occlude()
                    .h(px(32.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .pl(px(12.))
                    .pr(px(if missed.is_some() { 6. } else { 14. }))
                    .rounded_full()
                    .text_size(px(12.5))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(rgb(pal().muted))
                    .whitespace_nowrap()
                    .child(if busy { sparkle.with_animation("ipad-status-busy", motion::pulse(), |d, t| d.opacity(t)).into_any_element() } else { sparkle.into_any_element() })
                    .when_some(text, |d, text| {
                        d.child(div().id("ipad-status-text").when(needs_model, |d| d.cursor_pointer().on_click(cx.listener(|this, _, _, cx| {
                            this.panel = Panel::Settings;
                            cx.notify();
                        }))).child(text))
                    })
                    .when_some(missed, |d, (label, line)| {
                        d.child(
                            div()
                                .id("ipad-not-drawn")
                                .h(px(24.))
                                .flex()
                                .items_center()
                                .px(px(10.))
                                .rounded_full()
                                .bg(soft(1.))
                                .text_color(rgb(pal().text))
                                .cursor_pointer()
                                .active(|d| d.bg(soft(2.)))
                                .child(label)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.panel = Panel::Transcript;
                                    this.transcript_open = true;
                                    this.jumped_line = Some((line, Instant::now()));
                                    cx.notify();
                                })),
                        )
                    }),
            ),
        )
    }

    /// What is done with the whole meeting, at the top right: replay, share live, export, and the transcript.
    fn ipad_actions(&self, f: &Frame, cx: &mut Context<Self>) -> gpui_kit::Div {
        let replaying = self.replay.is_some();
        let can_replay = !self.live.steps.is_empty();
        glass(div(), false)
            .occlude()
            .absolute()
            .right(px(f.margin))
            .top(px(f.top() + 14.0))
            .h(px(TARGET))
            .flex()
            .items_center()
            .gap(px(2.))
            .px(px(4.))
            .rounded_full()
            .child(
                round_button("ipad-replay", "replay", replaying)
                    .when(!can_replay && !replaying, |d| d.opacity(0.35))
                    .when(can_replay || replaying, |d| {
                        d.on_click(cx.listener(|this, _, window, cx| {
                            if this.replay.is_some() {
                                this.exit_replay(cx);
                            } else {
                                this.start_replay(window, cx);
                            }
                        }))
                    }),
            )
            .child(round_button("ipad-share", "live", self.panel == Panel::Share || self.sharing.is_some()).on_click(cx.listener(|this, _, _, cx| this.toggle_panel(Panel::Share, cx))))
            .child(round_button("ipad-present", "airplay", false).on_click(cx.listener(|this, _, window, cx| this.enter_presenter(window, cx))))
            .child(round_button("ipad-export", "export", self.panel == Panel::Export).on_click(cx.listener(|this, _, _, cx| this.toggle_panel(Panel::Export, cx))))
            .child(rule().mx(px(4.)))
            .child(round_button("ipad-transcript", "transcript", false).when(self.side_shown(), |d| d.bg(soft(1.2))).on_click(cx.listener(|this, _, _, cx| this.toggle_side(cx))))
    }

    /// The Pencil's tools down the left: circle, mark, crop and the eraser, then the marker's colours. It folds
    /// into a 44 pt puck showing the tool in hand. A tool the board cannot take yet is dimmed: marks and crops
    /// go on an imported page or a clip.
    fn ipad_palette(&self, f: &Frame, cx: &mut Context<Self>) -> gpui_kit::Div {
        let (circling, cropping, marking, mark_tool) = {
            let ink = self.ink.borrow();
            (ink.circling, ink.cropping, ink.marking, ink.mark_tool)
        };
        let on_page = self.live.graph.underlay().is_some();
        let can_mark = on_page || self.live.graph.nodes.iter().any(|n| n.kind == Kind::Clip);
        let erasing = marking && mark_tool == ERASER;
        let place = |d: gpui_kit::Div| d.occlude().absolute().left(px(f.margin)).top(px(f.palette_top()));
        if !self.ipad.tools_open {
            let (name, on) = match () {
                _ if cropping => ("crop", true),
                _ if erasing => ("eraser", true),
                _ if marking => ("mark", true),
                _ => ("circle", circling),
            };
            return place(glass(div(), false)).rounded_full().p(px(0.)).child(round_button("ipad-tools-puck", name, on).on_click(cx.listener(|this, _, _, cx| {
                this.ipad.tools_open = true;
                cx.notify();
            })));
        }
        // Taking up a tool puts the others down; tapping the one in hand puts it down too.
        let tool = |id: &'static str, name: &'static str, on: bool, enabled: bool, take: fn(&mut Ink)| {
            round_button(id, name, on).when(!enabled, |d| d.opacity(0.35)).when(enabled, |d| {
                d.on_click(cx.listener(move |this, _, _, cx| {
                    let mut ink = this.ink.borrow_mut();
                    if on {
                        ink.put_tools_down();
                    } else {
                        ink.put_tools_down();
                        take(&mut ink);
                    }
                    cx.notify();
                }))
            })
        };
        let divider = || div().flex_none().w(px(24.)).h(px(1.)).my(px(6.)).bg(soft(2.3));
        place(glass(div(), false))
            .w(px(PALETTE_W))
            .p(px(6.))
            .flex()
            .flex_col()
            .items_center()
            .gap(px(4.))
            .rounded(px(28.))
            // The grip folds the palette away.
            .child(
                div()
                    .id("ipad-tools-grip")
                    .flex_none()
                    .w(px(TARGET))
                    .h(px(12.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .child(div().w(px(18.)).h(px(4.)).rounded(px(2.)).bg(soft(2.3)))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.ipad.tools_open = false;
                        cx.notify();
                    })),
            )
            .child(tool("ipad-circle", "circle", circling, true, |ink| ink.circling = true))
            .child(tool("ipad-mark", "mark", marking && !erasing, can_mark, |ink| ink.marking = true))
            .child(tool("ipad-crop", "crop", cropping, on_page, |ink| ink.cropping = true))
            .child(tool("ipad-eraser", "eraser", erasing, can_mark, |ink| {
                ink.marking = true;
                ink.mark_tool = ERASER;
            }))
            .child(divider())
            // The marker's colours: the red pen and the highlighters.
            .children(["pen", "yellow", "green", "pink"].into_iter().map(|name| {
                let (color, _, _) = annotation_look(name);
                let chosen = mark_tool == name;
                div()
                    .id(gpui_kit::ElementId::Name(format!("ipad-ink-{name}").into()))
                    .flex_none()
                    .size(px(38.))
                    .my(px(-1.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .when(chosen, |d| d.border_2().border_color(rgb(pal().text)))
                    .child(div().size(px(28.)).rounded_full().bg(rgb(pal().ink(color))))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let mut ink = this.ink.borrow_mut();
                        ink.mark_tool = name;
                        // Where marks can go, choosing a colour takes the marker up in it; elsewhere it waits.
                        if can_mark {
                            ink.put_tools_down();
                            ink.marking = true;
                        }
                        cx.notify();
                    }))
            }))
            .child(div().h(px(4.)))
    }

    /// The transcript as a card at the right, with the summary and the speakers a tab away, and a box to type
    /// a line for the board; folded, a pill at the corner with the newest line.
    fn ipad_card(&self, f: &Frame, transcript: AnyElement, summary: Option<AnyElement>, speakers_panel: Option<AnyElement>, cx: &mut Context<Self>) -> AnyElement {
        let speakers = self.live.speakers();
        if !self.side_shown() {
            // The newest line: what is being said now, or the last one said.
            let newest = self.draft.as_ref().map(|d| (self.speaker.clone(), d.text.clone())).or_else(|| {
                self.live.lines.iter().rev().find(|l| !l["manual"].as_bool().unwrap_or(false)).map(|l| (l["speaker"].as_str().filter(|s| !s.is_empty()).map(str::to_string), l["text"].as_str().unwrap_or_default().to_string()))
            });
            let (name, text) = newest.unwrap_or((None, t("transcript-2").to_string()));
            return glass(div(), true)
                .id("ipad-transcript-pill")
                .occlude()
                .absolute()
                .right(px(f.margin))
                .bottom(px(f.dock_bottom() + (DOCK_H - PILL_H) / 2.0))
                .h(px(PILL_H))
                .max_w(px(PILL_W))
                .flex()
                .items_center()
                .gap(px(10.))
                .pl(px(12.))
                .pr(px(10.))
                .rounded_full()
                .cursor_pointer()
                .text_size(px(14.))
                .when_some(name, |d, name| d.child(who(&speakers, &name, 24., true)))
                .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().text_color(rgb(pal().muted)).child(text))
                .child(div().flex_none().child(icon("ipad/chevron-up").size(px(18.))))
                .on_click(cx.listener(|this, _, _, cx| this.toggle_side(cx)))
                .into_any_element();
        }
        let side_panel = if self.panel.beside_board() { self.panel } else { Panel::Transcript };
        let dark = theme::is_dark();
        // Transcript, Summary, Speakers, as the system's segmented control.
        let tabs = div()
            .flex_1()
            .flex()
            .p(px(2.))
            .gap(px(2.))
            .rounded(px(9.))
            .bg(alpha(0x76_76_80, if dark { 0.24 } else { 0.12 }))
            .children([(Panel::Transcript, "ipad-tab-transcript", t("transcript-2")), (Panel::Summary, "ipad-tab-summary", t("summary-2")), (Panel::Speakers, "ipad-tab-speakers", t("speakers"))].map(|(panel, id, label)| {
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
            }));
        let header = div()
            .flex_none()
            .flex()
            .items_center()
            .gap(px(8.))
            .pt(px(12.))
            .pr(px(10.))
            .pb(px(8.))
            .pl(px(14.))
            .child(tabs)
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
        let body = match side_panel {
            Panel::Summary => summary,
            Panel::Speakers => speakers_panel,
            _ => Some(transcript),
        };
        // The box to type a line in, with who it goes in as beside it.
        let typing = (side_panel == Panel::Transcript && self.editable()).then(|| {
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
                    .cursor_pointer()
                    .child(who(&speakers, &name, 22., true))
                    .child(icon("ipad/chevron-down").size(px(14.)))
                    .on_click(cx.listener(move |this, _, window, cx| this.choose_speaker(picked.clone(), window, cx)))
            });
            div().flex_none().p(px(12.)).child(
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
                    .when_some(chip, |d, chip| d.child(chip)),
            )
        });
        glass(div(), true)
            .id("ipad-card")
            .occlude()
            .absolute()
            .right(px(f.margin))
            .top(px(f.card_top()))
            .w(px(f.card_w()))
            .h(px(f.card_h()))
            .flex()
            .flex_col()
            .overflow_hidden()
            .rounded(px(26.))
            .child(header)
            .when_some(body, |d, body| d.child(div().relative().flex_1().min_h_0().flex().flex_col().child(body)))
            .when_some(typing, |d, typing| d.child(typing))
            .into_any_element()
    }

    /// The dock along the bottom, in reach of a thumb: listening with its level and time, undo, redo and tidy,
    /// and the boards. A replay takes it over for its own controls.
    fn ipad_dock(&self, f: &Frame, replay_bar: Option<AnyElement>, fading: bool, cx: &mut Context<Self>) -> gpui_kit::Div {
        let place = |d: gpui_kit::Div| div().absolute().left_0().right_0().bottom(px(f.dock_bottom())).flex().justify_center().child(d);
        if let Some(bar) = replay_bar {
            return place(glass(div(), false).occlude().w(px(620.)).max_w(gpui_kit::relative(0.94)).h(px(DOCK_H)).flex().items_center().px(px(12.)).rounded(px(30.)).child(bar));
        }
        let editable = self.editable();
        let listening = self.pipeline.listening();
        let (page, pages) = (self.live.page(), self.live.pages());
        let recorded = self.recorded();
        let p = pal();
        // Listening: a red capsule with its level and how long it has listened, the one thing that never fades.
        let capsule = editable.then(|| {
            let (fill, ink) = if listening { (p.recording, 0xff_ff_ff) } else { (p.button, p.on_button) };
            let level = self.level;
            div()
                .id("ipad-listen")
                .flex_none()
                .h(px(48.))
                .flex()
                .items_center()
                .gap(px(10.))
                .pl(px(8.))
                .pr(px(16.))
                .rounded_full()
                .bg(rgb(fill))
                .text_color(rgb(ink))
                .text_size(px(15.))
                .font_weight(FontWeight::SEMIBOLD)
                .whitespace_nowrap()
                .cursor_pointer()
                .child(
                    div()
                        .flex_none()
                        .size(px(30.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_full()
                        .bg(alpha(0xff_ff_ff, 0.22))
                        .child(div().size(px(10.)).map(|d| if listening { d.rounded(px(2.)).bg(rgb(0xff_ff_ff)) } else { d.rounded_full().bg(rgb(p.recording)) })),
                )
                .child(if listening { t("listening") } else { t("start-listening") })
                .when(listening, |d| {
                    d.child(div().flex().items_center().gap(px(2.5)).h(px(18.)).children((0..6).map(|i| {
                        let h = 4.0 + level * 14.0 * (0.5 + 0.5 * ((i as f32) * 1.7 + 0.6).sin().abs());
                        div().w(px(3.)).h(px(h.min(18.))).rounded(px(2.)).bg(rgb(0xff_ff_ff))
                    })))
                })
                .when(listening || !recorded.is_zero(), |d| d.child(div().font_family(theme::MONO).text_size(px(13.)).font_weight(FontWeight::MEDIUM).child(clock(recorded.as_secs_f64() * 1000.0))))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.toggle_mic();
                    cx.notify();
                }))
        });
        let dim = |d: gpui_kit::Stateful<gpui_kit::Div>, off: bool| d.when(off, |d| d.opacity(0.35));
        let rest = div()
            .flex()
            .items_center()
            .gap(px(2.))
            .when(editable, |d| {
                let (undo, redo) = (self.live.can_undo(), self.live.can_redo());
                d.child(dim(round_button("ipad-undo", "undo", false), !undo).when(undo, |d| d.on_click(cx.listener(|this, _, _, cx| this.step(false, cx)))))
                    .child(dim(round_button("ipad-redo", "redo", false), !redo).when(redo, |d| d.on_click(cx.listener(|this, _, _, cx| this.step(true, cx)))))
                    .child(round_button("ipad-tidy", "tidy", false).on_click(cx.listener(|this, _, _, cx| this.tidy(cx))))
                    .child(rule().mx(px(8.)))
            })
            .child(dim(round_button("ipad-previous", "chevron-left", false), page == 0).when(page > 0, |d| d.on_click(cx.listener(move |this, _, _, cx| this.go_to_page(page - 1, cx)))))
            .child(div().px(px(2.)).font_family(theme::MONO).text_size(px(14.)).whitespace_nowrap().child(format!("{} / {pages}", page + 1)))
            .child(dim(round_button("ipad-next", "chevron-right", false), page + 1 >= pages).when(page + 1 < pages, |d| d.on_click(cx.listener(move |this, _, _, cx| this.go_to_page(page + 1, cx)))))
            .when(editable, |d| d.child(round_button("ipad-new-board", "plus", false).on_click(cx.listener(|this, _, _, cx| this.new_board(cx)))));
        // The glass and the buttons step back while listening goes on untouched; the capsule stays.
        let faded = |id: &'static str, el: gpui_kit::Div| el.with_spring(id, SpringAnimation::new(motion::SETTLE).to(fading), |d, v| d.opacity(1. - (1. - FADED) * v.0.clamp(0., 1.)));
        place(
            div()
                .occlude()
                .relative()
                .h(px(DOCK_H))
                .flex()
                .items_center()
                .px(px(8.))
                .child(div().absolute().top_0().bottom_0().left_0().right_0().child(faded("ipad-fade-dock-glass", glass(div(), false).size_full().rounded(px(30.)))))
                .when_some(capsule, |d, capsule| d.child(div().relative().child(capsule)).child(div().relative().child(faded("ipad-fade-dock-rule", rule().ml(px(10.)).mr(px(8.))))))
                .child(div().relative().child(faded("ipad-fade-dock", rest))),
        )
    }
}
