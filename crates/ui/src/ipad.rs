//! The iPad's meeting screen, "canvas first" (#341): the board takes the whole screen and everything else
//! floats over it on glass. The meeting's card at the top left, how the board model is doing in a pill at the
//! top, what is done with the whole meeting at the top right, the Pencil's tools down one side, the transcript
//! as a card that folds into a pill, and one dock in reach of a thumb along the bottom. Turned upright the
//! transcript becomes a sheet the dock rides on; in a narrow window (Stage Manager, Split View) the bars fold
//! into one compact window. While listening, the bars step back after a few seconds without a touch; the
//! listening capsule never does.
//!
//! Only the layout lives here (and the transcript's card, sheet and the circle's card in ipad_card.rs). What
//! each control does is the workbench's own, shared with the desktop, which lays the same parts out in its
//! own way (`platform::ipad_layout` chooses). A child of `app`, so it reaches the workbench's state directly.

use super::*;
use alcyo_board::graph::PENS;
use gpui_kit::{AnyElement, BoxShadow, Edges, FontWeight, SpringConfig, SpringPlayback};
use std::collections::VecDeque;

/// How long without a touch, while listening, before the bars step back, and how far they go.
const FADE_AFTER: Duration = Duration::from_secs(3);
const FADED: f32 = 0.3;
/// Every touch target is at least this big.
pub(super) const TARGET: f32 = 44.0;
/// The tool palette's width, and the dock's height.
const PALETTE_W: f32 = 56.0;
pub(super) const DOCK_H: f32 = 60.0;
/// The meeting's card at the top left.
pub(super) const CARD_H: f32 = 52.0;
/// The meetings list's width, past the left margin it shares with the meeting's card.
const SIDEBAR_PANEL_W: f32 = 296.0;
/// The puck the palette folds into, and the same in portrait.
const PUCK: f32 = 44.0;
const PUCK_PORTRAIT: f32 = 52.0;
/// The height of the palette's column: padding, grip, three tools (four with crop, on an imported page), a
/// rule, three inks and a little room.
fn palette_h(crop: bool) -> f32 {
    let tools = if crop { 4.0 } else { 3.0 };
    12.0 + 12.0 + tools * TARGET + 13.0 + 3.0 * 36.0 + 4.0 + (tools + 5.0) * 4.0
}
/// How far a drag must go before it moves anything, so a tap stays a tap.
const DRAG_SLOP: f32 = 6.0;

/// A spring for what steps back: slow and soft, so the bars fade as if dimmed, not switched.
const DIM: SpringConfig = SpringConfig::new(40., 12.6, 1.);
/// A spring for what moves across the screen: quick, with the faintest overshoot, as UIKit's do.
pub(super) const MOVE: SpringConfig = SpringConfig::new(300., 30., 1.);

/// Where the transcript card sits.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum Corner {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

/// What a finger or the mouse is dragging.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum Grab {
    Card,
    Sheet,
    Palette,
    Map,
}

/// A drag under way: where it started and where it is, and the last few moments of it, for the velocity
/// that throws the card to a corner or the sheet past a detent.
pub(super) struct Drag {
    pub(super) grab: Grab,
    pub(super) from: Point<Pixels>,
    /// What was being dragged, where it was when the drag began: the card's corner, the sheet's height, the
    /// palette's left edge.
    pub(super) start: (f32, f32),
    pub(super) now: Point<Pixels>,
    pub(super) moved: bool,
    pub(super) trail: VecDeque<(Instant, Point<Pixels>)>,
}

impl Drag {
    pub(super) fn delta(&self) -> (f32, f32) {
        (f32::from(self.now.x - self.from.x), f32::from(self.now.y - self.from.y))
    }

    /// Points a second, over the last tenth of a second.
    pub(super) fn velocity(&self) -> (f32, f32) {
        let (Some(first), Some(last)) = (self.trail.front(), self.trail.back()) else { return (0., 0.) };
        let dt = last.0.duration_since(first.0).as_secs_f32();
        if dt < 0.01 {
            return (0., 0.);
        }
        (f32::from(last.1.x - first.1.x) / dt, f32::from(last.1.y - first.1.y) / dt)
    }
}

/// The transcript sheet's heights, upright and in a narrow window: a one-line peek, half, and nearly all.
pub(super) const DETENTS: [f32; 3] = [96.0, 462.0, 1100.0];

/// What the iPad's screen keeps for itself, beside the workbench's own state.
pub(super) struct Chrome {
    /// The meetings list, floating over the board's left edge, and what was open when it came out: once
    /// something else is chosen from it, it goes away again.
    sidebar: Option<(Panel, String)>,
    /// The tool palette is out, rather than folded into a puck, and on which side it is docked.
    tools_open: bool,
    tools_right: bool,
    /// Upright, the palette is a puck until opened.
    upright_tools: bool,
    /// The pen the inks choose: black, red or blue.
    pen: &'static str,
    /// Where the transcript card is, and a drag under way.
    pub(super) corner: Corner,
    pub(super) drag: Option<Drag>,
    /// The last word search where search by meaning cannot run, and the meetings it found.
    word_search: (String, Vec<String>),
    /// The corner the board's map sits in while zoomed in.
    map_corner: Corner,
    /// The transcript sheet's height when upright or narrow.
    pub(super) sheet: f32,
    /// The meeting, and whether its transcript was out, when the transcript last went to its newest line.
    pub(super) newest_for: Option<(String, bool)>,
    /// When the screen was last touched, and whether the bars stepped back at the last frame. Shared with the
    /// window's event handlers, which note touches before any view hears of them.
    touched: Rc<Cell<Instant>>,
    faded: Rc<Cell<bool>>,
    /// The level meter's level, eased so it moves calmly.
    pub(super) level: f32,
    /// The Pencil was handed the circle tool, as the iPad opens with it in hand.
    primed: bool,
    /// When the screen last looked for an external display.
    looked: Instant,
}

impl Chrome {
    pub(super) fn new() -> Self {
        Self {
            sidebar: None,
            tools_open: true,
            tools_right: false,
            upright_tools: false,
            pen: "pen-black",
            corner: Corner::TopRight,
            drag: None,
            sheet: DETENTS[1],
            map_corner: Corner::TopLeft,
            word_search: (String::new(), vec![]),
            newest_for: None,
            touched: Rc::new(Cell::new(Instant::now())),
            faded: Rc::new(Cell::new(false)),
            level: 0.0,
            primed: false,
            looked: Instant::now(),
        }
    }

    /// Whether the palette is docked on the right.
    pub(super) fn tools_right_side(&self) -> bool {
        self.tools_right
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

/// How the screen is laid out at its size.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum Mode {
    /// Landscape, the board between floating bars and the transcript card.
    Wide,
    /// Upright: the transcript a sheet along the bottom, the dock riding it, the palette a puck.
    Upright,
    /// A narrow window (Stage Manager, Split View, Slide Over): one compact window. Below 700 pt the
    /// listening capsule drops its word; below 500 pt the circle goes into the ⋯ menu too.
    Compact { label: bool, circle: bool },
}

/// The sizes of the screen and what floats on it.
pub(super) struct Frame {
    pub(super) w: f32,
    pub(super) h: f32,
    pub(super) safe: Edges<Pixels>,
    /// How far from the screen's sides the floating things are.
    pub(super) margin: f32,
    pub(super) big: bool,
    pub(super) mode: Mode,
    /// What the on-screen keyboard covers at the bottom.
    pub(super) keyboard: f32,
    /// Where the bars along the top start from the left: past iPadOS's window controls in a windowed scene.
    pub(super) top_left: f32,
}

impl Frame {
    pub(super) fn of(window: &Window) -> Self {
        let size = window.viewport_size();
        let (w, h) = (f32::from(size.width), f32::from(size.height));
        let big = w >= 1300.0;
        let mode = if w >= 900.0 && w >= h {
            Mode::Wide
        } else if w >= 700.0 && h > w {
            Mode::Upright
        } else {
            Mode::Compact { label: w >= 700.0, circle: w >= 500.0 }
        };
        let keyboard = if cfg!(target_os = "ios") { f32::from(window.insets().ime.bottom) } else { 0.0 };
        let safe = crate::platform::safe_area(window);
        let margin = if big { 24.0 } else { 20.0 };
        let top = f32::from(safe.top);
        let top_left = crate::platform::clear_of_controls(window, margin, top, top + 10.0 + CARD_H + 10.0);
        Self { w, h, safe, margin, big, mode, keyboard, top_left }
    }

    pub(super) fn top(&self) -> f32 {
        f32::from(self.safe.top)
    }

    /// From the bottom of the screen to the dock: clear of the home indicator.
    pub(super) fn dock_bottom(&self) -> f32 {
        (f32::from(self.safe.bottom) + if self.big { 6.0 } else { 2.0 }).max(16.0)
    }

    pub(super) fn card_w(&self) -> f32 {
        if self.big { 372.0 } else { 330.0 }
    }

    pub(super) fn card_top(&self) -> f32 {
        self.top() + if self.big { 86.0 } else { 76.0 }
    }

    /// The transcript card, as tall as the screen allows above the dock.
    pub(super) fn card_h(&self) -> f32 {
        let room = self.h - self.card_top() - self.dock_bottom() - DOCK_H - 24.0;
        (if self.big { 700.0_f32 } else { 600.0 }).min(room).max(240.0)
    }

    fn palette_top(&self) -> f32 {
        (self.h * 0.29).round()
    }

    /// The sheet's detents that fit this screen, the tallest kept below the top bars.
    pub(super) fn detents(&self) -> [f32; 3] {
        let most = (self.h - self.top() - 10.0).min(DETENTS[2]);
        let half = DETENTS[1].min(self.h * 0.45);
        [DETENTS[0], half, most]
    }
}

/// A spring for `id` toward `target`, snapping there at once while `snap` (a drag following the finger) or
/// with reduced motion.
pub(super) fn spring(config: SpringConfig, target: f32, snap: bool) -> SpringAnimation<f32> {
    SpringAnimation::new(config).to(target).playback(if snap { SpringPlayback::Completed } else { SpringPlayback::Running })
}

/// Something coming onto the screen: from `dx`, `dy` points away, fading in, once when it first appears.
pub(super) fn arrive<E: gpui_kit::Styled + IntoElement + 'static>(id: impl Into<gpui_kit::ElementId>, el: E, dx: f32, dy: f32, delay: f32) -> impl IntoElement {
    let total = 0.42 + delay;
    el.with_animation(
        id,
        gpui_kit::Animation::new(Duration::from_secs_f32(total)).with_easing(move |t: f32| {
            let t = ((t * total - delay) / 0.42).clamp(0., 1.);
            1. - (1. - t).powi(4)
        }),
        move |d, t| d.relative().left(px(dx * (1. - t))).top(px(dy * (1. - t))).opacity(t),
    )
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
    /// The board in miniature, while zoomed in.
    pub minimap: Option<AnyElement>,
    pub zoomed_in: bool,
    /// Lines waiting for the board, and those it could not draw.
    pub waiting: usize,
    pub not_drawn: Vec<usize>,
}

impl Workbench {
    /// What floats over each side of the board on the iPad, for the board to fit between.
    pub(super) fn ipad_insets(&self, window: &Window) -> [f32; 4] {
        let f = Frame::of(window);
        match f.mode {
            Mode::Wide => {
                let palette = if self.ipad.tools_open { PALETTE_W } else { PUCK };
                let card = if self.side_shown() && matches!(self.ipad.corner, Corner::TopLeft | Corner::TopRight) { f.card_w() + f.margin } else { 0.0 };
                let (left, right) = match (self.ipad.tools_right, self.ipad.corner) {
                    (false, Corner::TopLeft | Corner::BottomLeft) => (f.margin + palette.max(card), f.margin),
                    (false, _) => (f.margin + palette, f.margin + card),
                    (true, Corner::TopLeft | Corner::BottomLeft) => (f.margin + card, f.margin + palette),
                    (true, _) => (f.margin, f.margin + palette.max(card)),
                };
                [left + BOARD_MARGIN, f.top() + 10.0 + CARD_H + BOARD_MARGIN, right + BOARD_MARGIN, f.dock_bottom() + DOCK_H + BOARD_MARGIN]
            }
            Mode::Upright => [BOARD_MARGIN, f.top() + 10.0 + CARD_H + BOARD_MARGIN, BOARD_MARGIN, self.ipad.sheet.min(f.detents()[1]) + 40.0 + BOARD_MARGIN],
            Mode::Compact { .. } => [16.0, f.top() + 60.0 + 16.0, 16.0, f.dock_bottom() + 68.0 + 76.0 + 16.0],
        }
    }

    /// The meetings whose transcript, boards or summary hold `query`, for search where search by meaning
    /// cannot run: read once for each query, case left aside.
    pub(super) fn word_hits(&mut self, query: &str) -> Vec<String> {
        if self.ipad.word_search.0 == query {
            return self.ipad.word_search.1.clone();
        }
        let Some(store) = self.store.as_ref() else { return vec![] };
        let hits: Vec<String> = self
            .library
            .iter()
            .filter(|m| {
                store.load(&m.id).ok().flatten().is_some_and(|loaded| serde_json::Value::Object(loaded.raw).to_string().to_lowercase().contains(query))
            })
            .map(|m| m.id.clone())
            .collect();
        self.ipad.word_search = (query.to_string(), hits.clone());
        hits
    }

    /// Where the board's map goes on the iPad: at the corner of the frame the iPad springs to its corner.
    pub(super) fn ipad_map_corner(&self, _window: &Window) -> (bool, bool, f32, f32) {
        (false, false, 0.0, 0.0)
    }

    /// The map's place at its corner: clear of the bars at the top and the dock at the bottom.
    fn ipad_map_rect(&self, f: &Frame, corner: Corner) -> (f32, f32) {
        let (w, h) = (MAP_PANEL_W, MAP_PANEL_H);
        let left = f.margin + if self.ipad.tools_right || f.mode != Mode::Wide { 0.0 } else { PALETTE_W + 12.0 };
        let right = f.w - f.margin - w - if self.ipad.tools_right && f.mode == Mode::Wide { PALETTE_W + 12.0 } else { 0.0 };
        let top = f.top() + 10.0 + CARD_H + 16.0;
        let bottom = f.h - f.dock_bottom() - DOCK_H - 16.0 - h;
        match corner {
            Corner::TopLeft => (left, top),
            Corner::TopRight => (right, top),
            Corner::BottomLeft => (left, bottom),
            Corner::BottomRight => (right, bottom),
        }
    }

    /// The board's map, while zoomed in, in a frame that springs to its corner and is dragged by its grip.
    fn ipad_map(&self, f: &Frame, map: AnyElement, cx: &mut Context<Self>) -> AnyElement {
        let dragging = self.ipad.drag.as_ref().filter(|d| d.grab == Grab::Map && d.moved);
        let (x, y) = match dragging {
            Some(drag) => (drag.start.0 + drag.delta().0, drag.start.1 + drag.delta().1),
            None => self.ipad_map_rect(f, self.ipad.map_corner),
        };
        let snap = dragging.is_some();
        // The map draws itself 4 pt in from the corner it is given.
        let frame = div()
            .relative()
            .w(px(MAP_PANEL_W + 8.0))
            .h(px(MAP_PANEL_H + 8.0))
            .child(map)
            .child(div().absolute().top_0().left_0().right_0().h(px(MAP_GRIP + 4.0)).child(self.ipad_drag_area(Grab::Map, cx)));
        div()
            .absolute()
            .top_0()
            .left_0()
            .child(div().child(arrive("ipad-map-in", frame, 0., -8., 0.)).with_spring("ipad-map-y", spring(MOVE, y - 4.0, snap), |d, v| d.relative().top(px(v))))
            .with_spring("ipad-map-x", spring(MOVE, x - 4.0, snap), |d, v| d.relative().left(px(v)))
            .into_any_element()
    }

    /// Whether only the Pencil marks the board, fingers panning and zooming.
    pub(super) fn ipad_pencil_only(&self) -> bool {
        self.form.saved.pencil_only
    }

    /// The Pencil was double-tapped or squeezed: undo, the eraser and back, the palette out and in, or nothing,
    /// as Settings says for each.
    pub(super) fn ipad_pencil_action(&mut self, action: crate::platform::PencilAction, cx: &mut Context<Self>) {
        let choice = match action {
            crate::platform::PencilAction::DoubleTap => self.form.saved.pencil_double_tap.clone(),
            crate::platform::PencilAction::Squeeze => self.form.saved.pencil_squeeze.clone(),
        };
        match choice.as_str() {
            "undo" => self.step(false, cx),
            "eraser" => {
                let mut ink = self.ink.borrow_mut();
                let erasing = ink.marking && ink.mark_tool == ERASER;
                ink.put_tools_down();
                if erasing {
                    ink.circling = true;
                } else {
                    ink.marking = true;
                    ink.mark_tool = ERASER;
                }
            }
            "tools" => {
                self.ipad.tools_open = !self.ipad.tools_open;
                self.ipad.upright_tools = !self.ipad.upright_tools;
            }
            _ => {}
        }
        cx.notify();
    }

    /// Whether a second has gone by since this last said so, for what is looked at now and then.
    pub(super) fn ipad_second_passed(&mut self) -> bool {
        if self.ipad.looked.elapsed() < Duration::from_secs(1) {
            return false;
        }
        self.ipad.looked = Instant::now();
        true
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

    /// A drag began on something that can be dragged: the card, the sheet or the palette.
    pub(super) fn ipad_drag_begin(&mut self, grab: Grab, at: Point<Pixels>, window: &Window) {
        let f = Frame::of(window);
        let start = match grab {
            Grab::Card => {
                let (x, y, _, _) = self.ipad_card_rect(&f, self.side_shown());
                (x, y)
            }
            Grab::Sheet => (self.ipad.sheet, 0.0),
            Grab::Palette => (if self.ipad.tools_right { f.w - f.margin - PALETTE_W } else { f.margin }, 0.0),
            Grab::Map => self.ipad_map_rect(&f, self.ipad.map_corner),
        };
        self.ipad.drag = Some(Drag { grab, from: at, start, now: at, moved: false, trail: VecDeque::from([(Instant::now(), at)]) });
    }

    pub(super) fn ipad_drag_move(&mut self, at: Point<Pixels>) -> bool {
        let Some(drag) = self.ipad.drag.as_mut() else { return false };
        drag.now = at;
        drag.trail.push_back((Instant::now(), at));
        while drag.trail.len() > 2 && drag.trail.front().is_some_and(|(t, _)| t.elapsed() > Duration::from_millis(100)) {
            drag.trail.pop_front();
        }
        let (dx, dy) = drag.delta();
        if !drag.moved && dx.hypot(dy) > DRAG_SLOP {
            drag.moved = true;
        }
        drag.moved
    }

    /// The drag let go: the card snaps to the corner it was thrown toward, the sheet to a detent, the palette to
    /// a side. A drag that never moved was a tap: on the palette's grip it folds the palette.
    pub(super) fn ipad_drag_end(&mut self, window: &Window, cx: &mut Context<Self>) {
        let Some(drag) = self.ipad.drag.take() else { return };
        let f = Frame::of(window);
        let (dx, dy) = drag.delta();
        let (vx, vy) = drag.velocity();
        match drag.grab {
            Grab::Card if drag.moved => {
                let open = self.side_shown();
                let (_, _, w, h) = self.ipad_card_rect(&f, open);
                // Where it would come to rest, thrown: a fifth of a second more at the speed it left the finger.
                let cx_ = drag.start.0 + dx + vx * 0.2 + w / 2.0;
                let cy_ = drag.start.1 + dy + vy * 0.2 + h / 2.0;
                let right = cx_ > f.w / 2.0;
                let bottom = open && cy_ > f.h / 2.0 + 60.0;
                self.ipad.corner = match (right, bottom) {
                    (false, false) => Corner::TopLeft,
                    (true, false) => Corner::TopRight,
                    (false, true) => Corner::BottomLeft,
                    (true, true) => Corner::BottomRight,
                };
                // The palette keeps to the other side from the card.
                if self.ipad.tools_right == right {
                    self.ipad.tools_right = !right;
                }
            }
            Grab::Sheet if drag.moved => {
                let detents = f.detents();
                let h = drag.start.0 - dy - vy * 0.18;
                self.ipad.sheet = detents.into_iter().min_by(|a, b| (a - h).abs().total_cmp(&(b - h).abs())).unwrap_or(detents[1]);
            }
            Grab::Palette if drag.moved => {
                self.ipad.tools_right = drag.start.0 + dx + vx * 0.2 + PALETTE_W / 2.0 > f.w / 2.0;
            }
            Grab::Map if drag.moved => {
                let cx_ = drag.start.0 + dx + vx * 0.2 + MAP_PANEL_W / 2.0;
                let cy_ = drag.start.1 + dy + vy * 0.2 + MAP_PANEL_H / 2.0;
                self.ipad.map_corner = match (cx_ > f.w / 2.0, cy_ > f.h / 2.0) {
                    (false, false) => Corner::TopLeft,
                    (true, false) => Corner::TopRight,
                    (false, true) => Corner::BottomLeft,
                    (true, true) => Corner::BottomRight,
                };
            }
            Grab::Palette if f.mode == Mode::Upright => self.ipad.upright_tools = false,
            Grab::Palette => self.ipad.tools_open = !self.ipad.tools_open,
            _ => {}
        }
        cx.notify();
    }

    /// Something that can be dragged by mouse or finger: drawn over what it is put in, it takes a press that
    /// starts inside, claims a finger's drag before the board can, and follows it to the release. Taps pass
    /// through untouched.
    pub(super) fn ipad_drag_area(&self, grab: Grab, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.entity().downgrade();
        canvas(
            |_, _, _| (),
            move |bounds: Bounds<Pixels>, (), window: &mut Window, _| {
                let (down, moved, up, touch) = (this.clone(), this.clone(), this.clone(), this.clone());
                window.on_mouse_event(move |e: &MouseDownEvent, phase, window, cx| {
                    if phase == gpui_kit::DispatchPhase::Capture && e.button == MouseButton::Left && bounds.contains(&e.position) {
                        let _ = down.update(cx, |w, _| w.ipad_drag_begin(grab, e.position, window));
                    }
                });
                window.on_mouse_event(move |e: &MouseMoveEvent, phase, _, cx| {
                    if phase == gpui_kit::DispatchPhase::Capture && e.pressed_button == Some(MouseButton::Left) {
                        let _ = moved.update(cx, |w, cx| {
                            if w.ipad.drag.as_ref().is_some_and(|d| d.grab == grab) && w.ipad_drag_move(e.position) {
                                cx.notify();
                            }
                        });
                    }
                });
                window.on_mouse_event(move |_: &MouseUpEvent, phase, window, cx| {
                    if phase == gpui_kit::DispatchPhase::Capture {
                        let _ = up.update(cx, |w, cx| {
                            if w.ipad.drag.as_ref().is_some_and(|d| d.grab == grab) {
                                w.ipad_drag_end(window, cx);
                            }
                        });
                    }
                });
                window.on_mouse_event(move |e: &gpui_kit::TouchDragEvent, phase, window, cx| {
                    if phase != gpui_kit::DispatchPhase::Bubble {
                        return;
                    }
                    match e.phase {
                        gpui_kit::TouchPhase::Started if bounds.contains(&e.start_position) => {
                            window.prevent_default();
                            let _ = touch.update(cx, |w, cx| {
                                w.ipad_drag_begin(grab, e.start_position, window);
                                w.ipad_drag_move(e.position);
                                cx.notify();
                            });
                        }
                        gpui_kit::TouchPhase::Moved => {
                            let _ = touch.update(cx, |w, cx| {
                                if w.ipad.drag.as_ref().is_some_and(|d| d.grab == grab) && w.ipad_drag_move(e.position) {
                                    cx.notify();
                                }
                            });
                        }
                        gpui_kit::TouchPhase::Ended | gpui_kit::TouchPhase::Cancelled => {
                            let _ = touch.update(cx, |w, cx| {
                                if w.ipad.drag.as_ref().is_some_and(|d| d.grab == grab) {
                                    w.ipad_drag_end(window, cx);
                                }
                            });
                        }
                        _ => {}
                    }
                });
            },
        )
        .absolute()
        .size_full()
    }

    /// The whole screen, on the iPad.
    pub(super) fn ipad_screen(&mut self, parts: Parts, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let f = Frame::of(window);
        let listening = self.pipeline.listening();
        // A page or another meeting chosen from the list puts the list away.
        if self.ipad.sidebar.as_ref().is_some_and(|(panel, id)| *panel != self.panel || id != self.live.id()) {
            self.ipad.sidebar = None;
        }
        // The transcript opens at its newest line: when the meeting changes, or the transcript comes out.
        let newest = (self.live.id().to_string(), self.side_shown());
        if self.ipad.newest_for.as_ref() != Some(&newest) {
            self.ipad.newest_for = Some(newest);
            self.transcript_scroll.scroll_to_bottom();
        }
        // The Pencil starts with the circle in hand: the one tool a meeting needs most.
        if !self.ipad.primed && self.editable() {
            self.ipad.primed = true;
            let mut ink = self.ink.borrow_mut();
            if !(ink.marking || ink.cropping) {
                ink.circling = true;
            }
        }
        // The meter eases toward the level heard, so it moves with the voice without twitching.
        self.ipad.level += (self.level - self.ipad.level) * 0.35;
        let circling_card = self.ink.borrow().circling && !self.ink.borrow().pointed.is_empty();
        let fading = self.form.saved.fade_tools && self.ipad.fading(listening) && self.panel.beside_board() && self.ipad.sidebar.is_none() && !circling_card && self.ipad.drag.is_none();
        self.ipad.faded.set(fading);
        // The bars step back together, slowly; a touch brings them back at once.
        let fade = move |id: &'static str, el: gpui_kit::Div| {
            el.with_spring(id, SpringAnimation::new(DIM).to(fading).playback(if fading { SpringPlayback::Running } else { SpringPlayback::Completed }), |d, v| d.opacity(1. - (1. - FADED) * v.0.clamp(0., 1.)))
        };
        // A page over the board (Meetings, Settings) keeps the meeting's card for the way back, and nothing else.
        let page_open = parts.page.is_some();
        let Parts { board, ground, welcome, tray, offers, notices, page, dialog, preparing, replay_bar, shortcuts, transcript, summary, speakers, minimap, zoomed_in, waiting, not_drawn } = parts;
        let replaying = replay_bar.is_some();

        // Every touch anywhere counts, before any element hears of it, so the bars come back at once.
        let (touched, faded) = (self.ipad.touched.clone(), self.ipad.faded.clone());
        let this = cx.entity().downgrade();
        let watch = canvas(
            |_, _, _| (),
            move |_, (), window: &mut Window, _| {
                // A drag ends with the press that began it, wherever that is let go. What began it may be gone by
                // then (the card folded into its pill under the finger), and a drag left behind would carry the
                // card off with the next press anywhere: a press far from where it began is not part of it.
                let (begun, ended, lifted) = (this.clone(), this.clone(), this.clone());
                window.on_mouse_event(move |e: &MouseDownEvent, phase, _, cx| {
                    if phase == gpui_kit::DispatchPhase::Capture {
                        let _ = begun.update(cx, |w, _| {
                            let far = |d: &Drag| {
                                let off = e.position - d.from;
                                f32::from(off.x).hypot(f32::from(off.y)) > DRAG_SLOP * 4.0
                            };
                            if w.ipad.drag.as_ref().is_some_and(far) {
                                w.ipad.drag = None;
                            }
                        });
                    }
                });
                window.on_mouse_event(move |_: &MouseUpEvent, phase, window, cx| {
                    if phase == gpui_kit::DispatchPhase::Capture {
                        let _ = ended.update(cx, |w, cx| {
                            if w.ipad.drag.is_some() {
                                w.ipad_drag_end(window, cx);
                            }
                        });
                    }
                });
                window.on_mouse_event(move |e: &gpui_kit::TouchDragEvent, phase, window, cx| {
                    if phase == gpui_kit::DispatchPhase::Capture && matches!(e.phase, gpui_kit::TouchPhase::Ended | gpui_kit::TouchPhase::Cancelled) {
                        let _ = lifted.update(cx, |w, cx| {
                            if w.ipad.drag.is_some() {
                                w.ipad_drag_end(window, cx);
                            }
                        });
                    }
                });
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

        let compact = matches!(f.mode, Mode::Compact { .. });
        // The meetings list, out, has the card's place at its head.
        let meeting_card = ((!compact || page_open) && self.ipad.sidebar.is_none()).then(|| self.ipad_meeting_card(&f, cx));
        let status = (!page_open && !replaying && !compact).then(|| if zoomed_in { Some(self.ipad_zoom_chip(&f, cx)) } else { self.ipad_status(&f, waiting, &not_drawn, cx) }).flatten();
        let status_shown = status.is_some();
        let actions = (!page_open && !compact).then(|| self.ipad_actions(&f, cx));
        let palette = (!page_open && !replaying && !compact && self.editable()).then(|| self.ipad_palette(&f, cx));
        let card = (!page_open).then(|| self.ipad_transcript(&f, transcript, summary, speakers, window, cx));
        let dock = (!page_open).then(|| self.ipad_dock(&f, replay_bar, fading, window, cx));
        let compact_bars = (compact && !page_open).then(|| self.ipad_compact_nav(&f, cx));
        let circle_card = (!page_open && !replaying).then(|| self.ipad_circle_card(&f, cx)).flatten();
        let sidebar = self.ipad.sidebar.is_some().then(|| self.ipad_sidebar(&f, listening, cx));
        // Under the status pill: the way back to the newest ink, and a board the talk moved to.
        let offers_top = f.top() + 20.0 + if status_shown { 32.0 + 10.0 } else { 0.0 } + if compact { 50.0 } else { 0.0 };
        // The notices place themselves 12 pt down, or under the offers when there are some.
        let notices_top = offers_top - 12.0;
        // Kept between the palette and the transcript card, never over either.
        let card_room = if self.side_shown() && f.mode == Mode::Wide { f.margin + f.card_w() + 8.0 } else { f.margin };
        let palette_room = if f.mode == Mode::Wide { f.margin + PALETTE_W + 8.0 } else { f.margin };
        let card_left = matches!(self.ipad.corner, Corner::TopLeft | Corner::BottomLeft);
        let (left_room, right_room) = if card_left { (card_room, palette_room) } else { (palette_room, card_room) };

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
            .child(div().absolute().inset_0().child(ground))
            .when_some(welcome, |d, welcome| d.child(div().absolute().top_0().bottom_0().left(px(left_room)).right(px(right_room)).child(welcome)))
            .when_some(tray, |d, tray| d.child(tray))
            .when_some(minimap.filter(|_| !page_open).map(|map| self.ipad_map(&f, map, cx)), |d, map| d.child(map))
            // Centred on the screen under the status pill, as that is, wherever the transcript card is.
            .child(div().absolute().top(px(offers_top)).left_0().right_0().flex().justify_center().child(offers))
            .when_some(notices, |d, notices| d.child(div().absolute().top(px(notices_top)).left(px(left_room)).right(px(right_room)).child(notices)))
            .when_some(compact_bars, |d, bars| d.child(fade("ipad-fade-compact", bars)))
            .when_some(palette, |d, palette| d.child(fade("ipad-fade-palette", div().absolute().inset_0().child(palette))))
            .when_some(circle_card, |d, card| d.child(card))
            .when_some(card, |d, card| d.child(card))
            .when_some(dock, |d, dock| d.child(dock))
            .when_some(status, |d, status| d.child(fade("ipad-fade-status", status)))
            .when_some(actions, |d, actions| d.child(fade("ipad-fade-actions", actions)))
            // Meetings and Settings lie over the board and under the meeting's card, which leads back.
            .when_some(page, |d, page| d.child(div().absolute().inset_0().pt(px(f.top() + 10.0 + CARD_H)).bg(rgb(pal().paper)).child(div().relative().size_full().child(page))))
            .when_some(sidebar, |d, sidebar| d.child(sidebar))
            .when_some(meeting_card, |d, card| d.child(fade("ipad-fade-meeting", card)))
            .when_some(preparing, |d, page| d.child(page))
            .when_some(dialog, |d, dialog| d.child(dialog))
            .when_some(shortcuts, |d, window| d.child(window))
            .into_any_element()
    }

    /// Export, Share or Import on the iPad: a rounded card floating over the whole screen, status bar and all,
    /// on a scrim a tap puts away. It is pinned to the screen's corner, never to where the flow before it ends,
    /// so the transcript card being out cannot push it down.
    pub(super) fn ipad_dialog(&self, title: &str, body: gpui_kit::Stateful<gpui_kit::Div>, cx: &mut Context<Self>) -> gpui_kit::Stateful<gpui_kit::Div> {
        let p = pal();
        let dark = theme::is_dark();
        let close = |this: &mut Self, cx: &mut Context<Self>| {
            this.panel = Panel::Transcript;
            cx.notify();
        };
        let shade = |a: f32, y: f32, blur: f32| BoxShadow { color: alpha(0, a), offset: point(px(0.), px(y)), blur_radius: px(blur), spread_radius: px(0.), inset: false };
        let hairline = BoxShadow { color: if dark { alpha(0xff_ff_ff, 0.1) } else { alpha(0, 0.08) }, offset: point(px(0.), px(0.)), blur_radius: px(0.), spread_radius: px(0.5), inset: false };
        let card = div()
            .id("dialog")
            .occlude()
            .w(px(560.))
            .max_w_full()
            .max_h(gpui_kit::relative(0.86))
            .flex()
            .flex_col()
            .rounded(px(26.))
            .overflow_hidden()
            .bg(rgb(p.surface))
            .shadow(vec![shade(if dark { 0.6 } else { 0.26 }, 24., 70.), hairline])
            .on_click(|_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .flex_none()
                    .h(px(64.))
                    .flex()
                    .items_center()
                    .justify_between()
                    .pl(px(24.))
                    .pr(px(14.))
                    .child(div().text_size(px(19.)).font_weight(FontWeight::BOLD).text_color(rgb(p.text)).child(title.to_string()))
                    .child(
                        div()
                            .id("close-dialog")
                            .size(px(TARGET))
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .child(div().size(px(30.)).flex().items_center().justify_center().rounded_full().bg(soft(1.3)).text_color(rgb(p.muted)).child(icon("x").size(px(13.))))
                            .active(|d| d.opacity(0.6))
                            .on_click(cx.listener(move |this, _, _, cx| close(this, cx))),
                    ),
            )
            .child(body.px(px(24.)).pb(px(24.)).pt(px(4.)).text_size(px(14.5)))
            .with_animation("ipad-dialog-in", gpui_kit::Animation::new(Duration::from_millis(260)).with_easing(|t: f32| 1. - (1. - t).powi(3)), |d, t| d.opacity(t).mt(px(18. * (1. - t))));
        div()
            .id("dialog-veil")
            .occlude()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .p(px(24.))
            .flex()
            .items_center()
            .justify_center()
            .bg(alpha(0x0a_0a_0a, if dark { 0.5 } else { 0.3 }))
            .on_click(cx.listener(move |this, _, _, cx| close(this, cx)))
            .child(card)
    }

    /// The meetings list, as iPadOS's own sidebars: the whole height of the screen along its left edge, sliding
    /// in from it over a dimmed board. Its head is where the meeting's card was, the button that brought it out
    /// in the same place to put it away.
    fn ipad_sidebar(&self, f: &Frame, listening: bool, cx: &mut Context<Self>) -> gpui_kit::Div {
        let w = f.top_left + SIDEBAR_PANEL_W;
        let dark = theme::is_dark();
        let close = cx.listener(|this, _, _, cx| {
            this.ipad.sidebar = None;
            cx.notify();
        });
        let head = div()
            .flex_none()
            .mt(px(f.top() + 10.0))
            .h(px(CARD_H))
            .pl(px(f.top_left + 4.0))
            .pr(px(12.))
            .flex()
            .items_center()
            .gap(px(4.))
            .child(round_button("ipad-sidebar-close", "sidebar", false).on_click(close))
            .child(div().flex_1().min_w_0().pl_1().text_title().font_weight(FontWeight::SEMIBOLD).child(t("meetings")));
        let list = self.sidebar(listening, cx).w_full().flex_1().min_h_0().bg(gpui_kit::transparent_black()).border_r(px(0.));
        let panel = div()
            .id("ipad-sidebar")
            .occlude()
            .absolute()
            .top_0()
            .bottom_0()
            .left_0()
            .w(px(w))
            .flex()
            .flex_col()
            .pb(px(f32::from(f.safe.bottom)))
            .bg(rgb(pal().chrome))
            .border_r_1()
            .border_color(rgb(pal().line))
            .shadow(vec![BoxShadow { color: alpha(0, if dark { 0.5 } else { 0.12 }), offset: point(px(0.), px(0.)), blur_radius: px(24.), spread_radius: px(0.), inset: false }])
            .child(head)
            .child(div().flex_1().min_h_0().flex().flex_col().pl(px((f.top_left - 8.0).max(0.0))).child(list))
            .with_animation("ipad-sidebar-in", gpui_kit::Animation::new(Duration::from_millis(320)).with_easing(|t: f32| 1. - (1. - t).powi(4)), move |d, t| d.left(px(-w * (1. - t))));
        // Pinned to the screen's corner: left to its static place it would start below the board, which is in
        // the flow before it, and so it did on the iPad, half way down.
        div()
            .absolute()
            .inset_0()
            // The board dims under it, and a tap on it puts the list away.
            .child(
                div()
                    .id("ipad-sidebar-veil")
                    .absolute()
                    .inset_0()
                    .bg(alpha(0, if dark { 0.4 } else { 0.22 }))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.ipad.sidebar = None;
                        cx.notify();
                    }))
                    .with_animation("ipad-sidebar-veil-in", gpui_kit::Animation::new(Duration::from_millis(240)), |d, t| d.opacity(t)),
            )
            .child(panel)
    }

    /// While zoomed in, the status pill gives way to how far, and how to come back.
    fn ipad_zoom_chip(&self, f: &Frame, cx: &mut Context<Self>) -> gpui_kit::Div {
        let k = self.ink.borrow().shown().k;
        let map_on = !self.form.saved.minimap_hidden;
        div().absolute().top(px(f.top() + 20.0)).left_0().right_0().flex().justify_center().child(arrive(
            "ipad-zoom-chip-in",
            glass(div(), false)
                .id("ipad-zoom-chip")
                .occlude()
                .h(px(32.))
                .flex()
                .items_center()
                .gap(px(6.))
                .px(px(14.))
                .rounded_full()
                .font_family(theme::MONO)
                .text_size(px(12.5))
                .text_color(rgb(pal().muted))
                .cursor_pointer()
                .child(icon("ipad/zoom").size(px(14.)))
                .child(ta!("ipad-zoom-fit", percent = (k * 100.0).round() as i64))
                .on_click(cx.listener(|this, _, _, cx| this.fit_board(cx)))
                // The map comes and goes from here, while zoomed in.
                .child(
                    div()
                        .id("ipad-map-toggle")
                        .ml(px(4.))
                        .mr(px(-8.))
                        .size(px(28.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_full()
                        .when(map_on, |d| d.bg(soft(1.5)).text_color(rgb(pal().text)))
                        .child(icon("map").size(px(13.)))
                        .on_click(cx.listener(|this, _, _, cx| {
                            cx.stop_propagation();
                            this.toggle_map(cx);
                        })),
                ),
            0.,
            -8.,
            0.,
        ))
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

    /// The meeting's card at the top left: the meetings list, and the meeting's name over when it began and
    /// how many spoke. A page over the board is named there instead.
    fn ipad_meeting_card(&self, f: &Frame, cx: &mut Context<Self>) -> gpui_kit::Div {
        let page_name = match self.panel {
            Panel::Library => Some(t("meetings")),
            Panel::Settings => Some(t("settings")),
            _ => None,
        };
        let page_open = page_name.is_some();
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
            .left(px(f.top_left))
            .top(px(f.top() + 10.0))
            .h(px(CARD_H))
            .flex()
            .items_center()
            .gap(px(4.))
            .pl(px(4.))
            // Over a page, which has its own title, only the way to the list stays.
            .pr(px(if page_open { 4. } else { 14. }))
            .rounded_full()
            .child(round_button("ipad-sidebar-toggle", "sidebar", false).on_click(cx.listener(|this, _, _, cx| {
                this.ipad_toggle_sidebar();
                cx.notify();
            })))
            .when(!page_open, |d| d.child(title))
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
            .when(f.mode == Mode::Wide, |d| {
                d.child(rule().mx(px(4.)))
                    .child(round_button("ipad-transcript", "transcript", false).when(self.side_shown(), |d| d.bg(soft(1.2))).on_click(cx.listener(|this, _, _, cx| this.toggle_side(cx))))
            })
    }

    /// The Pencil's tools: circle, mark, crop and the eraser, then the black, red and blue pens. Docked to
    /// either side in landscape, where its grip folds it into a 44 pt puck and drags it across; upright it is a
    /// 52 pt puck over the sheet that opens upward. The column folds into the puck and back, its tools
    /// following one after another.
    fn ipad_palette(&self, f: &Frame, cx: &mut Context<Self>) -> AnyElement {
        let (circling, cropping, marking, mark_tool) = {
            let ink = self.ink.borrow();
            (ink.circling, ink.cropping, ink.marking, ink.mark_tool)
        };
        let on_page = self.live.graph.underlay().is_some();
        let erasing = marking && mark_tool == ERASER;
        let upright = f.mode == Mode::Upright;
        let open = if upright { self.ipad.upright_tools } else { self.ipad.tools_open };
        let puck = if upright { PUCK_PORTRAIT } else { PUCK };
        let (w, h) = if open { (PALETTE_W, palette_h(on_page)) } else { (puck, puck) };
        let pen = self.ipad.pen;
        let (name, on) = match () {
            _ if cropping => ("crop", true),
            _ if erasing => ("eraser", true),
            _ if marking => ("mark", true),
            _ => ("circle", circling),
        };
        let grip_drag = open.then(|| self.ipad_drag_area(Grab::Palette, cx));
        // Taking up a tool puts the others down; tapping the one in hand puts it down too. Each says what it
        // is under a pointer.
        let tool = |id: &'static str, name: &'static str, tip: &'static str, on: bool, take: fn(&mut Ink, &'static str)| {
            round_button(id, name, on).tooltip(move |window, cx| Tooltip::new(tip).build(window, cx)).on_click(cx.listener(move |this, _, _, cx| {
                let pen = this.ipad.pen;
                let mut ink = this.ink.borrow_mut();
                ink.put_tools_down();
                if !on {
                    take(&mut ink, pen);
                }
                cx.notify();
            }))
        };
        let content: AnyElement = if open {
            let divider = div().flex_none().w(px(24.)).h(px(1.)).my(px(6.)).bg(soft(2.3));
            let grip = div()
                .id("ipad-tools-grip")
                .relative()
                .flex_none()
                .w(px(TARGET))
                .h(px(12.))
                .flex()
                .items_center()
                .justify_center()
                .cursor_grab()
                .child(div().w(px(18.)).h(px(4.)).rounded(px(2.)).bg(soft(2.3)))
                .when_some(grip_drag, |d, area| d.child(area));
            let rise = if upright { 10. } else { -10. };
            let mut items: Vec<AnyElement> = vec![
                grip.into_any_element(),
                tool("ipad-circle", "circle", t("circle"), circling, |ink, _| ink.circling = true).into_any_element(),
                // Marking draws in the pen chosen below: black, red or blue.
                tool("ipad-mark", "mark", t("mark"), marking && !erasing, |ink, pen| {
                    ink.marking = true;
                    ink.mark_tool = pen;
                })
                .into_any_element(),
            ];
            // Cropping cuts a piece out of an imported page, so it is offered only on one.
            if on_page {
                items.push(tool("ipad-crop", "crop", t("cut-a-part-of-the"), cropping, |ink, _| ink.cropping = true).into_any_element());
            }
            items.extend([
                tool("ipad-eraser", "eraser", t("ipad-pencil-eraser"), erasing, |ink, _| {
                    ink.marking = true;
                    ink.mark_tool = ERASER;
                })
                .into_any_element(),
                divider.into_any_element(),
            ]);
            // The pens: black, red and blue. Choosing one takes it up.
            items.extend(PENS.into_iter().map(|name| {
                let (color, _, _) = annotation_look(name);
                let chosen = pen == name;
                let drawing = marking && mark_tool == name;
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
                    .child(div().size(px(28.)).rounded_full().bg(rgb(pal().ink(color))).when(drawing, |d| d.shadow(vec![BoxShadow { color: alpha(pal().ink(color), 0.35), offset: point(px(0.), px(0.)), blur_radius: px(6.), spread_radius: px(1.), inset: false }])))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.ipad.pen = name;
                        let mut ink = this.ink.borrow_mut();
                        ink.put_tools_down();
                        ink.mark_tool = name;
                        ink.marking = true;
                        cx.notify();
                    }))
                    .into_any_element()
            }));
            items.push(div().h(px(4.)).into_any_element());
            let count = items.len();
            div()
                .w(px(PALETTE_W))
                .p(px(6.))
                .flex()
                .flex_col()
                .items_center()
                .gap(px(4.))
                // Upright the column opens upward from the puck, so its tools arrive from the bottom.
                .children(items.into_iter().enumerate().map(|(i, item)| {
                    let order = if upright { count - 1 - i } else { i };
                    arrive(("ipad-tool-in", i), div().child(item), 0., rise, order as f32 * 0.025).into_any_element()
                }))
                .into_any_element()
        } else {
            // The puck shows the tool in hand, and the pen's ink in a dot at its edge.
            let (color, _, _) = annotation_look(pen);
            div()
                .id("ipad-tools-puck")
                .relative()
                .size(px(puck))
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .cursor_pointer()
                .text_color(rgb(pal().text))
                .child(icon(&format!("ipad/{name}")).size(px(22.)))
                .when(upright || on, |d| d.child(div().absolute().right(px(2.)).bottom(px(2.)).size(px(14.)).rounded_full().bg(rgb(pal().ink(color))).border_2().border_color(rgb(pal().surface))))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if upright {
                        this.ipad.upright_tools = true;
                    } else {
                        this.ipad.tools_open = true;
                    }
                    cx.notify();
                }))
                .into_any_element()
        };
        // The column and the puck are one surface that grows and shrinks between them.
        let surface = glass(div(), false)
            .occlude()
            .rounded_full()
            .overflow_hidden()
            .h_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_end()
            .child(content)
            .with_spring("ipad-palette-w", spring(MOVE, w, false), |d, v| d.w(px(v)));
        let sized = div().child(surface).with_spring("ipad-palette-h", spring(MOVE, h, false), |d, v| d.h(px(v)));
        if upright {
            let dragging_sheet = self.ipad.drag.as_ref().is_some_and(|d| d.grab == Grab::Sheet && d.moved);
            let bottom = self.ipad_sheet_height(f) + 52.0;
            return div()
                .absolute()
                .right(px(22.))
                .child(sized)
                .with_spring("ipad-palette-bottom", spring(MOVE, bottom, dragging_sheet), |d, v| d.bottom(px(v)))
                .into_any_element();
        }
        let dragging = self.ipad.drag.as_ref().filter(|d| d.grab == Grab::Palette && d.moved);
        let x = match dragging {
            Some(drag) => drag.start.0 + drag.delta().0,
            None if self.ipad.tools_right => f.w - f.margin - w,
            None => f.margin,
        };
        div()
            .absolute()
            .top(px(f.palette_top()))
            .left_0()
            .child(arrive("ipad-palette-in", div().child(sized), -20., 0., 0.05))
            .with_spring("ipad-palette-x", spring(MOVE, x, dragging.is_some()), |d, v| d.relative().left(px(v)))
            .into_any_element()
    }

    /// The listening capsule: red with its level and how long it has listened, or ink asking to start. Its
    /// words rise into place as it changes; below 700 pt it keeps only the meter and the time.
    fn ipad_capsule(&self, big: bool, label: bool, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let listening = self.pipeline.listening();
        let recorded = self.recorded();
        let p = pal();
        let ink = if listening { 0xff_ff_ff } else { p.on_button };
        let level = self.ipad.level;
        let now = self.listening_since.map_or(0.0, |s| s.elapsed().as_secs_f32());
        if listening && level > 0.02 {
            window.request_animation_frame();
        }
        let (height, stop, text) = if big { (56., 38., 17.) } else { (48., 30., 15.) };
        let face = div()
            .flex()
            .items_center()
            .gap(px(10.))
            .when(label, |d| d.child(if listening { t("listening") } else { t("start-listening") }))
            .when(listening, |d| {
                // Six bars that breathe with the voice, each at its own pace, so the meter never ticks in step.
                d.child(div().flex().items_center().gap(px(2.5)).h(px(if big { 22. } else { 18. })).children((0..6).map(|i| {
                    let sway = 0.55 + 0.45 * ((now * (5.1 + i as f32 * 0.9)) + i as f32 * 1.7).sin().abs();
                    let h = 4.0 + level * (if big { 18.0 } else { 14.0 }) * sway;
                    div().w(px(3.)).h(px(h.min(if big { 22. } else { 18. }))).rounded(px(2.)).bg(rgb(0xff_ff_ff))
                })))
            })
            .when(listening || !recorded.is_zero(), |d| d.child(div().font_family(theme::MONO).text_size(px(if big { 15. } else { 13. })).font_weight(FontWeight::MEDIUM).child(clock(recorded.as_secs_f64() * 1000.0))));
        div()
            .id("ipad-listen")
            .flex_none()
            .h(px(height))
            .flex()
            .items_center()
            .gap(px(10.))
            .pl(px(8.))
            .pr(px(if label || listening { 16. } else { 8. }))
            .rounded_full()
            .text_color(rgb(ink))
            .text_size(px(text))
            .font_weight(FontWeight::SEMIBOLD)
            .whitespace_nowrap()
            .cursor_pointer()
            .child(
                div()
                    .flex_none()
                    .size(px(stop))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .bg(alpha(0xff_ff_ff, 0.22))
                    .child(div().size(px(10.)).map(|d| if listening { d.rounded(px(2.)).bg(rgb(0xff_ff_ff)) } else { d.rounded_full().bg(rgb(p.recording)) })),
            )
            .child(arrive(gpui_kit::ElementId::Name(format!("ipad-listen-face-{listening}").into()), face, 0., 8., 0.))
            .on_click(cx.listener(|this, _, _, cx| {
                this.toggle_mic();
                cx.notify();
            }))
            // Ink glides to red as listening starts, and back.
            .with_spring("ipad-listen-fill", SpringAnimation::new(motion::SETTLE).to(listening), move |d, v| d.bg(motion::mix(p.button, p.recording, v.0.clamp(0., 1.))))
            .into_any_element()
    }

    /// The dock in reach of a thumb: listening, undo, redo and tidy, and the boards. Upright it rides the
    /// transcript sheet's top edge; a replay takes it over for its own controls.
    fn ipad_dock(&self, f: &Frame, replay_bar: Option<AnyElement>, fading: bool, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let upright = f.mode == Mode::Upright;
        let dragging_sheet = self.ipad.drag.as_ref().is_some_and(|d| d.grab == Grab::Sheet && d.moved);
        let place = |d: AnyElement| -> AnyElement {
            if upright {
                let bottom = self.ipad_sheet_height(f) - 32.0;
                div().absolute().left_0().right_0().flex().justify_center().child(d).with_spring("ipad-dock-bottom", spring(MOVE, bottom, dragging_sheet), |d, v| d.bottom(px(v))).into_any_element()
            } else {
                div().absolute().left_0().right_0().bottom(px(f.dock_bottom())).flex().justify_center().child(arrive("ipad-dock-in", div().child(d), 0., 24., 0.08)).into_any_element()
            }
        };
        if let Some(bar) = replay_bar {
            return place(glass(div(), false).occlude().w(px(620.)).max_w(gpui_kit::relative(0.94)).h(px(DOCK_H)).flex().items_center().px(px(12.)).rounded(px(30.)).child(bar).into_any_element());
        }
        let editable = self.editable();
        let (page, pages) = (self.live.page(), self.live.pages());
        let capsule = editable.then(|| self.ipad_capsule(false, true, window, cx));
        let dim = |d: gpui_kit::Stateful<gpui_kit::Div>, off: bool| d.when(off, |d| d.opacity(0.35));
        let rest = div()
            .flex()
            .items_center()
            .gap(px(2.))
            .when(editable, |d| {
                let (undo, redo) = (self.live.can_undo(), self.live.can_redo());
                d.child(dim(round_button("ipad-undo", "undo", false), !undo).when(undo, |d| d.on_click(cx.listener(|this, _, _, cx| this.step(false, cx)))))
                    .when(!upright, |d| d.child(dim(round_button("ipad-redo", "redo", false), !redo).when(redo, |d| d.on_click(cx.listener(|this, _, _, cx| this.step(true, cx))))))
                    .child(round_button("ipad-tidy", "tidy", false).on_click(cx.listener(|this, _, _, cx| this.tidy(cx))))
                    .child(rule().mx(px(if upright { 6. } else { 8. })))
            })
            .child(dim(round_button("ipad-previous", "chevron-left", false), page == 0).when(page > 0, |d| d.on_click(cx.listener(move |this, _, _, cx| this.go_to_page(page - 1, cx)))))
            .child(div().px(px(2.)).font_family(theme::MONO).text_size(px(14.)).whitespace_nowrap().child(format!("{} / {pages}", page + 1)))
            .child(dim(round_button("ipad-next", "chevron-right", false), page + 1 >= pages).when(page + 1 < pages, |d| d.on_click(cx.listener(move |this, _, _, cx| this.go_to_page(page + 1, cx)))))
            .when(editable, |d| d.child(round_button("ipad-new-board", "plus", false).on_click(cx.listener(|this, _, _, cx| this.new_board(cx)))));
        if let Mode::Compact { label, circle } = f.mode {
            return self.ipad_compact_bar(f, capsule.is_some().then(|| self.ipad_capsule(false, label, window, cx)), circle, cx);
        }
        // The glass and the buttons step back while listening goes on untouched; the capsule stays.
        let faded = |id: &'static str, el: gpui_kit::Div| {
            el.with_spring(id, SpringAnimation::new(DIM).to(fading).playback(if fading { SpringPlayback::Running } else { SpringPlayback::Completed }), |d, v| d.opacity(1. - (1. - FADED) * v.0.clamp(0., 1.)))
        };
        let height = if upright { 64. } else { DOCK_H };
        place(
            div()
                .occlude()
                .relative()
                .h(px(height))
                .flex()
                .items_center()
                .px(px(8.))
                .child(div().absolute().top_0().bottom_0().left_0().right_0().child(faded("ipad-fade-dock-glass", glass(div(), false).size_full().rounded(px(height / 2.0)))))
                .when_some(capsule, |d, capsule| d.child(div().relative().child(capsule)).child(div().relative().child(faded("ipad-fade-dock-rule", rule().ml(px(10.)).mr(px(8.))))))
                .child(div().relative().child(faded("ipad-fade-dock", rest)))
                .into_any_element(),
        )
    }

    /// A narrow window's own bars: the meeting along the top with the transcript and ⋯; the newest line and
    /// listening, circle, undo and the boards along the bottom. What has no room goes into ⋯, in the order the
    /// desktop's dock gives way.
    fn ipad_compact_nav(&self, f: &Frame, cx: &mut Context<Self>) -> gpui_kit::Div {
        let circle_in_bar = matches!(f.mode, Mode::Compact { circle: true, .. });
        let this = cx.entity().downgrade();
        let more = Button::new("ipad-more").ghost().child(icon("ipad/more").size(px(22.))).w(px(TARGET)).h(px(TARGET)).rounded_full().dropdown_menu_with_anchor(gpui_kit::Anchor::TopRight, move |menu, _, _| {
            let act = |label: &'static str, f: fn(&mut Workbench, &mut Context<Workbench>)| {
                let this = this.clone();
                PopupMenuItem::new(label).on_click(move |_, _, cx| {
                    let _ = this.update(cx, |w, cx| f(w, cx));
                })
            };
            let menu = menu.min_w(px(220.));
            let menu = if circle_in_bar {
                menu
            } else {
                menu.item(act(t("circle"), |w, cx| {
                    let mut ink = w.ink.borrow_mut();
                    ink.put_tools_down();
                    ink.circling = true;
                    drop(ink);
                    cx.notify();
                }))
            };
            menu.item(act(t("mark"), |w, cx| {
                let mut ink = w.ink.borrow_mut();
                ink.put_tools_down();
                ink.marking = true;
                ink.mark_tool = w.ipad.pen;
                drop(ink);
                cx.notify();
            }))
            .item(act(t("crop"), |w, cx| {
                let mut ink = w.ink.borrow_mut();
                ink.put_tools_down();
                ink.cropping = true;
                drop(ink);
                cx.notify();
            }))
            .item(act(t("tidy-the-layout"), |w, cx| w.tidy(cx)))
            .separator()
            .item(act(t("export"), |w, cx| w.toggle_panel(Panel::Export, cx)))
            .item(act(t("share"), |w, cx| w.toggle_panel(Panel::Share, cx)))
        });
        glass(div(), false)
            .occlude()
            .absolute()
            .top(px(f.top()))
            .left_0()
            .right_0()
            .h(px(60.))
            .flex()
            .items_center()
            .gap(px(4.))
            .pt(px(6.))
            .pl(px(8.0 + f.top_left - f.margin))
            .pr(px(8.))
            .rounded_b(px(0.))
            .child(round_button("ipad-sidebar-toggle", "sidebar", false).on_click(cx.listener(|this, _, _, cx| {
                this.ipad_toggle_sidebar();
                cx.notify();
            })))
            .child(div().flex_1().min_w_0().pl_1().overflow_hidden().whitespace_nowrap().text_ellipsis().text_title().font_weight(FontWeight::SEMIBOLD).child(self.live.title().to_string()))
            .child(round_button("ipad-transcript", "transcript", self.side_shown()).on_click(cx.listener(|this, _, _, cx| this.toggle_side(cx))))
            .child(more)
    }

    fn ipad_compact_bar(&self, f: &Frame, capsule: Option<AnyElement>, circle: bool, cx: &mut Context<Self>) -> AnyElement {
        let circling = self.ink.borrow().circling;
        let (page, pages) = (self.live.page(), self.live.pages());
        let undo = self.live.can_undo();
        let speakers = self.live.speakers();
        // The newest line, above the bar, when the transcript is away.
        let newest = (!self.side_shown()).then(|| {
            self.draft.as_ref().map(|d| (self.speaker.clone(), d.text.clone(), true)).or_else(|| {
                self.live.lines.iter().rev().find(|l| !l["manual"].as_bool().unwrap_or(false)).map(|l| (l["speaker"].as_str().filter(|s| !s.is_empty()).map(str::to_string), l["text"].as_str().unwrap_or_default().to_string(), false))
            })
        });
        div()
            .absolute()
            .left_0()
            .right_0()
            .bottom_0()
            .flex()
            .flex_col()
            .when_some(newest.flatten(), |d, (who_, text, now)| {
                d.child(
                    div()
                        .id("ipad-compact-last")
                        .occlude()
                        .mx(px(12.))
                        .mb(px(10.))
                        .px(px(12.))
                        .py(px(10.))
                        .rounded(px(16.))
                        .bg(rgb(pal().chrome))
                        .border_1()
                        .border_color(rgb(pal().line))
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .cursor_pointer()
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_side(cx)))
                        .when_some(who_.clone(), |d, name| {
                            d.child(div().flex().items_center().gap(px(8.)).text_size(px(12.5)).font_weight(FontWeight::SEMIBOLD).child(who(&speakers, &name, 24., true)).child(name).when(now, |d| d.child(div().font_weight(FontWeight::NORMAL).text_color(rgb(pal().recording)).child(t("now")))))
                        })
                        .child(div().pl(px(if who_.is_some() { 32. } else { 0. })).text_size(px(14.)).text_color(rgb(pal().muted)).overflow_hidden().whitespace_nowrap().text_ellipsis().child(if now { format!("{text}▍") } else { text })),
                )
            })
            .child(
                div()
                    .occlude()
                    .h(px(68.0 + (f.dock_bottom() - 16.0).max(0.0)))
                    .pb(px((f.dock_bottom() - 16.0).max(0.0)))
                    .flex()
                    .items_center()
                    .gap(px(2.))
                    .px(px(10.))
                    .border_t_1()
                    .border_color(rgb(pal().line))
                    .bg(rgb(pal().chrome))
                    .when_some(capsule, |d, c| d.child(c))
                    .child(div().flex_1())
                    .when(circle, |d| {
                        d.child(round_button("ipad-circle", "circle", circling).on_click(cx.listener(move |this, _, _, cx| {
                            let mut ink = this.ink.borrow_mut();
                            ink.put_tools_down();
                            if !circling {
                                ink.circling = true;
                            }
                            cx.notify();
                        })))
                    })
                    .child(round_button("ipad-undo", "undo", false).when(!undo, |d| d.opacity(0.35)).when(undo, |d| d.on_click(cx.listener(|this, _, _, cx| this.step(false, cx)))))
                    .child(rule().mx(px(4.)))
                    .child(round_button("ipad-previous", "chevron-left", false).when(page == 0, |d| d.opacity(0.35)).when(page > 0, |d| d.on_click(cx.listener(move |this, _, _, cx| this.go_to_page(page - 1, cx)))))
                    .child(div().font_family(theme::MONO).text_size(px(13.)).child(format!("{}/{pages}", page + 1)))
                    .child(round_button("ipad-next", "chevron-right", false).when(page + 1 >= pages, |d| d.opacity(0.35)).when(page + 1 < pages, |d| d.on_click(cx.listener(move |this, _, _, cx| this.go_to_page(page + 1, cx))))),
            )
            .into_any_element()
    }
}

/// What the circles took in, tinted in the accent one after another, and the circles themselves dashed in it,
/// the newest closing its gap with a short settle. Drawn over the board's ink on the iPad.
pub(super) fn paint_circles(ink: &Ink, origin: Point<Pixels>, k: f32, window: &mut Window) {
    let accent = pal().accent;
    let since = ink.closed_at.map_or(10.0, |t| t.elapsed().as_secs_f32());
    let mut moving = false;
    for (i, id) in ink.pointed.iter().enumerate() {
        let Some(t) = ink.targets.iter().find(|t| &t.id == id) else { continue };
        let p = ((since - 0.06 - i as f32 * 0.05) / 0.16).clamp(0., 1.);
        moving |= p < 1.0;
        let ease = 1. - (1. - p).powi(3);
        let pad = 8.0 * k.min(1.0);
        let area = Bounds::new(origin + point(px(t.x as f32 * k - pad), px(t.y as f32 * k - pad)), size(px(t.w as f32 * k + 2. * pad), px(t.h as f32 * k + 2. * pad)));
        window.paint_quad(gpui_kit::fill(area, alpha(accent, 0.09 * ease)).corner_radii(px(10.)));
    }
    let at = |p: &[f32; 2]| origin + point(px(p[0] * k), px(p[1] * k));
    let last = ink.loops.len().saturating_sub(1);
    let mut pb = PathBuilder::stroke(px(1.6));
    for (n, shape) in ink.loops.iter().enumerate() {
        let mut points: Vec<Point<Pixels>> = shape.iter().map(at).collect();
        // The gap between where the Pencil lifted and where it began closes, quickly, after it lifts.
        if let (Some(first), Some(end)) = (points.first().copied(), points.last().copied()) {
            let p = if n == last { (since / 0.16).clamp(0., 1.) } else { 1.0 };
            moving |= p < 1.0;
            let e = 1. - (1. - p).powi(3);
            points.push(end + (first - end) * e);
        }
        dashes(&mut pb, &points);
    }
    if let Some(shape) = ink.drawing.as_ref() {
        dashes(&mut pb, &shape.iter().map(at).collect::<Vec<_>>());
    }
    if let Ok(path) = pb.build() {
        window.paint_path(path, Background::from(alpha(accent, 0.95)));
    }
    if moving {
        window.request_animation_frame();
    }
}

/// A dashed line along `points`: dashes of 6 pt with gaps of 4.5, as UIKit draws a lasso.
fn dashes(pb: &mut PathBuilder, points: &[Point<Pixels>]) {
    const ON: f32 = 6.0;
    const OFF: f32 = 4.5;
    let mut phase = 0.0_f32;
    for pair in points.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let (dx, dy) = (f32::from(b.x - a.x), f32::from(b.y - a.y));
        let len = dx.hypot(dy);
        let mut s = 0.0;
        while s < len {
            let in_dash = phase < ON;
            let left = if in_dash { ON - phase } else { ON + OFF - phase };
            let step = left.min(len - s);
            if in_dash {
                let p0 = a + point(px(dx * s / len), px(dy * s / len));
                let p1 = a + point(px(dx * (s + step) / len), px(dy * (s + step) / len));
                pb.move_to(p0);
                pb.line_to(p1);
            }
            s += step;
            phase = (phase + step) % (ON + OFF);
        }
    }
}
