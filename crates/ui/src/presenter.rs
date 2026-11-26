//! Presenter mode on the iPad (#341, direction C): the iPad on the lectern, cabled or AirPlayed to the room's
//! projector. The room sees a clean 16:9 board with two lines of captions; the iPad shows that same picture
//! in a frame on a dark surround, with a few big controls around it: listening, undo, the boards as
//! thumbnails, and Follow live.
//!
//! What the room sees is drawn by one function, [`Workbench::room_picture`], wherever it is shown: inside the
//! frame on the iPad, and as the whole of a separate window on the external display ([`RoomView`]). Both
//! paint the same board from the same textures, in the frame's space scaled to whatever holds them:
//!
//! - The board being drawn on is painted from the board's own ink, as its canvas paints it: the finished
//!   strokes, the ink sliding into place and the stroke the pen is writing, read from [`Ink`] every frame, so
//!   the room sees the pen write live. The finished strokes go onto a tier of the room's own ([`LiveTier`]),
//!   filled from `Ink::finished` as strokes arrive, at a sharpness a 1080p projector needs (the board's
//!   textures follow the iPad's zoom instead). The room keeps its own view too ([`RoomGlide`]): fitted to the
//!   16:9 frame, kept while new ink fits, gliding to a new fit when not, so the room never sees the iPad zoom.
//!   A circle drawn on the frame is handed to the board's own tools at the matching point.
//! - A board the room is shown that is not being drawn on is still: it is laid out and rasterized once, off
//!   the main thread, into textures of its own ([`Art`]), at the room's sharpness and at the thumbnails'.
//!   Thumbnails get their own small rendering, as the board's textures shrunk that far break thin strokes
//!   into specks; the live board's thumbnail is filled again from its finished strokes now and then.
//!
//! A child of `app`, so it reaches the workbench's state directly.

use super::*;
use gpui_kit::{linear_color_stop, linear_gradient, AnyElement, AnyWindowHandle, BoxShadow, FontWeight, WeakEntity};

/// The surround, dark whatever the theme, so the iPad never glares beside the projector.
const SURROUND: u32 = 0x0d_0d_0e;
const ON_DARK: u32 = 0xf2_f2_f2;
const MUTED_ON_DARK: u32 = 0xc4_c4_c4;
const FAINT_ON_DARK: u32 = 0x8f_8f_8f;
/// The accent on the dark surround, and the green of a connected display.
const ACCENT_ON_DARK: u32 = 0x9f_b4_ff;
const CONNECTED: u32 = 0x34_c7_59;
const CONNECTED_TEXT: u32 = 0x7e_e2_9a;
/// The top bar and the bottom bar, and how far from the screen's sides everything is.
const TOP_H: f32 = 52.0;
const BAR_H: f32 = 76.0;
const SIDE: f32 = 24.0;
/// The frame's width on an 11" iPad, which the room's captions are sized against.
const DESIGN_W: f32 = 1146.0;
/// A board as a thumbnail.
const THUMB_W: f32 = 96.0;
const THUMB_H: f32 = 58.0;
/// The controls one stands and taps.
const BIG: f32 = 52.0;
/// The live board's thumbnail is filled again at most this often while ink arrives.
const THUMB_EVERY: Duration = Duration::from_millis(600);
/// A new picture for the room cross-fades in over this long.
const SWITCH: Duration = Duration::from_millis(420);
/// How much a small board is blown up for the room, at most, over its size on the 11" frame.
const ROOM_ZOOM: f32 = 1.5;

fn alpha(color: u32, a: f32) -> Hsla {
    let mut c: Hsla = rgb(color).into();
    c.a = a;
    c
}

/// The external display the room watches.
#[derive(Clone, Debug)]
pub(super) struct RoomDisplay {
    pub name: String,
    pub size: gpui_kit::Size<Pixels>,
}

/// The external display, if one is connected.
///
/// INTEGRATION POINT (lead): replace the body with `crate::platform::room_display(cx)` once the platform
/// reports external displays. A desktop build pretends one is connected when started with
/// `ALCYO_PRESENTER_DISPLAY=<name>`, so the presenter's screen can be worked on without one.
pub(super) fn room_display(cx: &App) -> Option<RoomDisplay> {
    if let Some(display) = crate::platform::room_display(cx).filter(|_| cfg!(target_os = "ios")) {
        return Some(RoomDisplay { name: display.name, size: display.size });
    }
    std::env::var("ALCYO_PRESENTER_DISPLAY").ok().filter(|n| !n.is_empty()).map(|name| RoomDisplay { name, size: size(px(1920.), px(1080.)) })
}

/// Which board the room is shown.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Shown {
    /// The board being drawn on, painted live from its ink.
    Live,
    /// Another board, still.
    Page(usize),
}

/// What presenter mode keeps beside the workbench's own state.
pub(super) struct Presenter {
    /// The iPad shows the presenter's screen.
    pub(super) on: bool,
    /// The room sees two lines of captions under the board; off, the board only.
    captions: bool,
    /// The board sent to the room by hand; None follows the board being drawn on.
    room_page: Option<usize>,
    /// The ⋯ menu is out.
    menu: bool,
    /// The external display, when one is connected.
    display: Option<RoomDisplay>,
    /// The room's own window on the external display, once one is open.
    room_window: Option<AnyWindowHandle>,
    /// Opening it was tried for the display connected now: a failure is not tried again every frame.
    window_tried: bool,
    /// What the room shows, by board number, and what it showed before with when that changed: the new
    /// picture cross-fades in over the old one. Counted, so each change starts its own fade.
    shown: Option<(Shown, usize)>,
    before: Option<(Shown, usize, Instant)>,
    switches: usize,
    /// The frame on the iPad, which the board is fitted to; the room's own window shows it scaled. Where it
    /// is on the screen, for a circle drawn on it.
    frame: Rc<Cell<(f32, f32)>>,
    frame_at: Rc<Cell<Point<Pixels>>>,
    /// The room's view of the live board.
    glide: Rc<Cell<Option<RoomGlide>>>,
    /// Pictures of the boards for the thumbnails and the room.
    art: Rc<RefCell<Art>>,
}

impl Presenter {
    pub(super) fn new() -> Self {
        // A desktop build laid out for the iPad starts in presenter mode with ALCYO_PRESENTER=1, so it can be
        // screenshotted off-screen.
        let on = crate::platform::ipad_layout() && !cfg!(target_os = "ios") && std::env::var("ALCYO_PRESENTER").is_ok_and(|v| v == "1");
        Self {
            on,
            captions: true,
            room_page: None,
            menu: false,
            display: None,
            room_window: None,
            window_tried: false,
            shown: None,
            before: None,
            switches: 0,
            frame: Rc::new(Cell::new((DESIGN_W, DESIGN_W * 9.0 / 16.0))),
            frame_at: Rc::new(Cell::new(point(px(0.), px(0.)))),
            glide: Rc::new(Cell::new(None)),
            art: Rc::new(RefCell::new(Art::default())),
        }
    }
}

/// A board's pictures, made once.
struct PageArt {
    /// The room's sharpness, and the thumbnail's.
    room: Tier,
    thumb: Tier,
    extent: BoardBounds,
    /// The imported page under the board, with its size on the board, and each clip's picture where its card is.
    page: Option<(Arc<RenderImage>, f32, f32)>,
    pictures: Vec<(Arc<RenderImage>, [f32; 4])>,
}

/// The boards' pictures, kept while presenter mode is on.
#[derive(Default)]
struct Art {
    /// Which meeting they are of, how many boards it had and which was being drawn on: when any of that
    /// changes, the boards not drawn on are made again.
    key: Option<(String, usize, usize)>,
    pages: HashMap<usize, PageArt>,
    making: bool,
    /// The live board's thumbnail: its strokes, and what it was made from (the board's generation and how many
    /// finished strokes) and when.
    live_thumb: Option<(Tier, (u64, usize), Instant)>,
    /// The live board at the room's sharpness.
    live_room: Option<LiveTier>,
    /// Textures given up, dropped at the next paint.
    retired: Vec<Arc<RenderImage>>,
}

impl Art {
    fn retire(&mut self, art: PageArt) {
        self.retired.extend(art.room.release());
        self.retired.extend(art.thumb.release());
        if let Some((image, _, _)) = art.page {
            self.retired.push(image);
        }
        self.retired.extend(art.pictures.into_iter().map(|(image, _)| image));
    }
}

/// The view that fits a board into the frame, as `Ink::fit` fits it to the board's canvas, except that a
/// small board is blown up to half again its size: the room reads it from the back.
fn fit_into(e: &BoardBounds, w: f32, h: f32, insets: [f32; 4]) -> View {
    let [left, top, right, bottom] = insets;
    let (free_w, free_h) = ((w - left - right).max(1.0), (h - top - bottom).max(1.0));
    let bw = (e.x1 - e.x0) as f32 + 120.0;
    let bh = (e.y1 - e.y0) as f32 + 120.0;
    let k = (free_w / bw).min(free_h / bh).min(ROOM_ZOOM * w / DESIGN_W);
    View { k, x: (e.x0 + e.x1) as f32 / 2.0 - (left + free_w / 2.0) / k, y: (e.y0 + e.y1) as f32 / 2.0 - (top + free_h / 2.0) / k }
}

/// How the room's view of the live board moves: from one view to another, starting then. Kept apart from the
/// board's own view, so the room never sees someone zooming about on the iPad.
#[derive(Clone, Copy)]
struct RoomGlide {
    from: View,
    to: View,
    at: Instant,
    /// The board it is a view of: another board is shown at once, as the picture cross-fades anyway.
    page: usize,
}

impl RoomGlide {
    /// The view now, eased out: the scale changes by the same ratio each step and the middle moves straight,
    /// as the board's own view glides.
    fn now(&self, frame: (f32, f32)) -> (View, bool) {
        let p = self.at.elapsed().as_secs_f32() / GLIDE.as_secs_f32();
        if p >= 1.0 || self.from.k <= 0.0 || self.to.k <= 0.0 {
            return (self.to, false);
        }
        let e = 1.0 - (1.0 - p).powi(3);
        let (hw, hh) = (frame.0 / 2.0, frame.1 / 2.0);
        let centre = |v: View| (v.x + hw / v.k, v.y + hh / v.k);
        let (a, b) = (centre(self.from), centre(self.to));
        let k = self.from.k * (self.to.k / self.from.k).powf(e);
        let (cx, cy) = (a.0 + (b.0 - a.0) * e, a.1 + (b.1 - a.1) * e);
        (View { k, x: cx - hw / k, y: cy - hh / k }, true)
    }
}

/// The room's view of the live board, in the frame's space: kept while what is on the board still fits in
/// it, so the board does not rescale with every line, and gliding to a new fit when it does not.
fn room_view(glide: &Cell<Option<RoomGlide>>, extent: &BoardBounds, page: usize, frame: (f32, f32), insets: [f32; 4]) -> (View, bool) {
    let fit = fit_into(extent, frame.0, frame.1, insets);
    let Some(g) = glide.get().filter(|g| g.page == page) else {
        let g = RoomGlide { from: fit, to: fit, at: Instant::now() - GLIDE, page };
        glide.set(Some(g));
        return (fit, false);
    };
    let cur = g.to;
    let [left, top, right, bottom] = insets;
    let inside = extent.x0 as f32 >= cur.x + left / cur.k
        && extent.y0 as f32 >= cur.y + top / cur.k
        && extent.x1 as f32 <= cur.x + (frame.0 - right) / cur.k
        && extent.y1 as f32 <= cur.y + (frame.1 - bottom) / cur.k;
    // A board that shrank a lot, as after an undo, is fitted again too.
    if inside && fit.k <= cur.k * 1.25 {
        return g.now(frame);
    }
    let (shown, _) = g.now(frame);
    glide.set(Some(RoomGlide { from: shown, to: fit, at: Instant::now(), page }));
    (shown, true)
}

/// The live board's strokes at the room's sharpness: filled from the board's finished strokes as they come,
/// and made again when the board repaints what remains or the room needs another sharpness.
struct LiveTier {
    tier: Tier,
    generation: u64,
    filled: usize,
}

fn sync_live_tier(art: &mut Art, b: &Ink, level: f32) {
    let stale = art.live_room.as_ref().is_none_or(|l| l.generation != b.generation || l.tier.level != level || l.filled > b.finished.len());
    if stale {
        if let Some(old) = art.live_room.take() {
            art.retired.extend(old.tier.release());
        }
        art.live_room = Some(LiveTier { tier: Tier::new(level), generation: b.generation, filled: 0 });
    }
    let live = art.live_room.as_mut().expect("made above");
    for stroke in &b.finished[live.filled..] {
        live.tier.fill(stroke);
    }
    live.filled = b.finished.len();
}

/// What floats over the frame's edges, which the board keeps clear of: the captions along the bottom.
fn frame_insets(w: f32, captions: bool) -> [f32; 4] {
    let s = w / DESIGN_W;
    [24.0 * s, 28.0 * s, 24.0 * s, if captions { 96.0 } else { 24.0 } * s]
}

/// Fill a tier with a board drawn for export.
fn fill_tier(tier: &mut Tier, drawn: &alcyo_export::ink::Drawn) {
    for shape in &drawn.strokes {
        tier.fill(&Stroke { key: String::new(), shape: shape.outline.clone(), style: shape.style, area: [0.0; 4] });
    }
}

/// Faint dots under the board, as on the board's canvas, `gap` apart.
fn paint_dots(window: &mut Window, bounds: Bounds<Pixels>, gap: f32, size_: f32) {
    let dot: Hsla = rgb(pal().line).into();
    let mut gy = bounds.origin.y + px(gap);
    while gy < bounds.origin.y + bounds.size.height {
        let mut gx = bounds.origin.x + px(gap);
        while gx < bounds.origin.x + bounds.size.width {
            window.paint_quad(gpui_kit::fill(Bounds::new(point(gx - px(size_ / 2.), gy - px(size_ / 2.)), size(px(size_), px(size_))), dot));
            gx += px(gap);
        }
        gy += px(gap);
    }
}

/// A filled outline on the board, at `origin` and `k` screen pixels per board unit.
fn paint_shape(window: &mut Window, shape: &[Pt], origin: Point<Pixels>, k: f32, d: [f32; 2], fill: Hsla) {
    let Some(first) = shape.first() else { return };
    let at = |p: &Pt| origin + point(px((p[0] + d[0]) * k), px((p[1] + d[1]) * k));
    let mut pb = fill_builder();
    pb.move_to(at(first));
    shape[1..].iter().for_each(|p| pb.line_to(at(p)));
    pb.close();
    if let Ok(path) = pb.build() {
        window.paint_path(path, Background::from(fill));
    }
}

/// The live board as the room sees it, in `bounds`: the room's view in the iPad's frame, scaled to fit, with
/// the ink sliding into place and the stroke the pen is writing over the finished ink.
fn paint_live(window: &mut Window, bounds: Bounds<Pixels>, b: &Ink, art: &mut Art, glide: &Cell<Option<RoomGlide>>, page: usize, frame: (f32, f32), insets: [f32; 4]) {
    let r = f32::from(bounds.size.width) / frame.0.max(1.0);
    let (v, moving) = room_view(glide, &b.extent, page, frame, insets);
    if moving {
        window.request_animation_frame();
    }
    // As sharp as the iPad's frame on its dense screen, or a 1080p projector, whichever asks more.
    let target = glide.get().map_or(v, |g| g.to);
    sync_live_tier(art, b, raster_level(target.k, (1920.0 / frame.0).max(window.scale_factor())).min(4.0));
    let k = v.k * r;
    let origin = bounds.origin + point(px(-v.x * k), px(-v.y * k));
    paint_dots(window, bounds, GRID * r, 2.0 * r.max(1.0));
    if let Some((image, w, h)) = b.page.clone().filter(|_| b.graph.as_ref().is_some_and(|g| g.underlay().is_some())) {
        let area = Bounds::new(origin, size(px(w * k), px(h * k)));
        window.paint_drop_shadows(area, gpui_kit::Corners::default(), &theme::shadow(theme::Lift::Sheet));
        let _ = window.paint_image(area, area, gpui_kit::Corners::default(), image, 0, false);
    }
    for (card, clip) in &b.clips {
        let src = if clip.src.is_empty() { b.page_src.as_ref() } else { Some(&clip.src) };
        let Some(Some(image)) = src.and_then(|src| b.clip_images.get(src).cloned()) else { continue };
        let at = |x: f64, y: f64| origin + point(px(x as f32 * k), px(y as f32 * k));
        let card_area = Bounds::new(at(card.x, card.y), size(px(card.w as f32 * k), px(card.h as f32 * k)));
        let s = card.w / clip.w;
        let whole = Bounds::new(at(card.x - clip.x * s, card.y - clip.y * s), size(px((clip.pw * s) as f32 * k), px((clip.ph * s) as f32 * k)));
        window.with_content_mask(Some(gpui_kit::ContentMask { bounds: card_area.intersect(&bounds) }), |window| {
            let _ = window.paint_image(whole, whole, gpui_kit::Corners::default(), image, 0, false);
        });
    }
    if let Some(live) = art.live_room.as_mut() {
        live.tier.under.paint(window, origin, k, Some(bounds));
        live.tier.layer.paint(window, origin, k, Some(bounds));
    }
    if let Some(left) = b.slide_left() {
        for (stroke, from) in &b.sliding {
            paint_shape(window, &stroke.shape, origin, k, [from[0] * left, from[1] * left], color(stroke.style));
        }
    }
    if let Some(front) = b.queue.front().filter(|_| b.live > 0.0) {
        paint_shape(window, &outline(&front.inked, b.live), origin, k, [0.0, 0.0], color(front.style));
    }
    // The circles: drawn for the room to see what is pointed at.
    for shape in b.loops.iter().chain(b.drawing.iter()) {
        let mut pb = PathBuilder::stroke(px(2.5 * r.max(1.0)));
        let at = |p: &[f32; 2]| origin + point(px(p[0] * k), px(p[1] * k));
        if let Some(first) = shape.first() {
            pb.move_to(at(first));
            shape[1..].iter().for_each(|p| pb.line_to(at(p)));
            if b.drawing.as_ref() != Some(shape) {
                pb.close();
            }
        }
        if let Ok(path) = pb.build() {
            window.paint_path(path, Background::from(alpha(pal().accent, 0.8)));
        }
    }
}

/// A board as a thumbnail: filling it, to be told apart at a glance, though no bigger than the room shows it.
fn thumb_view(e: &BoardBounds) -> View {
    let [left, top, right, bottom] = [7.0, 6.0, 7.0, 8.0];
    let (free_w, free_h) = (THUMB_W - left - right, THUMB_H - top - bottom);
    let bw = (e.x1 - e.x0) as f32 + 40.0;
    let bh = (e.y1 - e.y0) as f32 + 40.0;
    let k = (free_w / bw).min(free_h / bh).min(ROOM_ZOOM * THUMB_W / DESIGN_W);
    View { k, x: (e.x0 + e.x1) as f32 / 2.0 - (left + free_w / 2.0) / k, y: (e.y0 + e.y1) as f32 / 2.0 - (top + free_h / 2.0) / k }
}

/// A still board in `bounds`, fitted as the frame would fit it, or as a thumbnail.
fn paint_still(window: &mut Window, bounds: Bounds<Pixels>, art: &mut PageArt, frame: (f32, f32), insets: [f32; 4], thumb: bool) {
    // A lifted thumbnail is drawn a little bigger.
    let (v, r) = if thumb { (thumb_view(&art.extent), f32::from(bounds.size.width) / THUMB_W) } else { (fit_into(&art.extent, frame.0, frame.1, insets), f32::from(bounds.size.width) / frame.0.max(1.0)) };
    let k = v.k * r;
    let origin = bounds.origin + point(px(-v.x * k), px(-v.y * k));
    if !thumb {
        paint_dots(window, bounds, GRID * r, 2.0 * r.max(1.0));
    }
    if let Some((image, w, h)) = art.page.clone() {
        let area = Bounds::new(origin, size(px(w * k), px(h * k)));
        let _ = window.paint_image(area, area, gpui_kit::Corners::default(), image, 0, false);
    }
    for (image, [x, y, w, h]) in &art.pictures {
        let area = Bounds::new(origin + point(px(x * k), px(y * k)), size(px(w * k), px(h * k)));
        let _ = window.paint_image(area, area, gpui_kit::Corners::default(), image.clone(), 0, false);
    }
    let tier = if thumb { &mut art.thumb } else { &mut art.room };
    tier.under.paint(window, origin, k, Some(bounds));
    tier.layer.paint(window, origin, k, Some(bounds));
}

impl Workbench {
    /// Show the presenter's screen on the iPad, and the room its picture on the external display if one is
    /// connected.
    pub(super) fn enter_presenter(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.presenter.on = true;
        self.presenter.menu = false;
        // Nothing circled could be acted on here: the circle's card is the meeting's own screen's.
        self.ink.borrow_mut().put_tools_down();
        self.presenter.display = room_display(cx);
        if let Some(display) = self.presenter.display.clone() {
            self.present_on_external(display, cx);
        }
        cx.notify();
    }

    /// Back to the meeting's own screen. The room keeps its picture while `keep_room`; otherwise its window
    /// closes.
    fn leave_presenter(&mut self, keep_room: bool, cx: &mut Context<Self>) {
        self.presenter.on = false;
        self.presenter.menu = false;
        if !keep_room {
            if let Some(handle) = self.presenter.room_window.take() {
                let _ = handle.update(cx, |_, window, _| window.remove_window());
            }
        }
        // Let go of the pictures made for the room.
        let mut art = self.presenter.art.borrow_mut();
        let old: Vec<PageArt> = art.pages.drain().map(|(_, a)| a).collect();
        for a in old {
            art.retire(a);
        }
        if let Some((tier, _, _)) = art.live_thumb.take() {
            art.retired.extend(tier.release());
        }
        if let Some(live) = art.live_room.take() {
            art.retired.extend(live.tier.release());
        }
        art.key = None;
        let retired = std::mem::take(&mut art.retired);
        drop(art);
        self.ink.borrow_mut().retired.extend(retired);
        cx.notify();
    }

    /// An external display was connected or went away. Connected, presenter mode starts by itself.
    ///
    /// INTEGRATION POINT (lead): call this from the platform's display-change callback, e.g.
    /// `crate::platform::on_room_display(cx, move |display, window, cx| workbench.update(cx, |w, cx| w.room_display_changed(display, window, cx)))`.
    /// Look for a display coming or going; the pump asks about once a second. The iPad goes into presenter
    /// mode when one is connected, and the room's window closes when it goes.
    pub(super) fn watch_room_display(&mut self, cx: &mut Context<Self>) {
        let display = room_display(cx).filter(|_| cfg!(target_os = "ios"));
        if display.as_ref().map(|d| &d.name) != self.presenter.display.as_ref().map(|d| &d.name) {
            self.room_display_changed(display, cx);
        }
    }

    pub(super) fn room_display_changed(&mut self, display: Option<RoomDisplay>, cx: &mut Context<Self>) {
        self.presenter.display = display.clone();
        match display {
            Some(display) => {
                self.presenter.on = true;
                self.present_on_external(display, cx);
            }
            None => {
                self.presenter.window_tried = false;
                if let Some(handle) = self.presenter.room_window.take() {
                    let _ = handle.update(cx, |_, window, _| window.remove_window());
                }
            }
        }
        cx.notify();
    }

    /// Give the room its own window on the external display, showing [`Workbench::room_picture`] through a
    /// [`RoomView`]; until it is open, the room is simulated inside the iPad's frame only.
    ///
    /// INTEGRATION POINT (lead): open the window on the external display through the platform, e.g.
    /// `crate::platform::open_room_window(&display, cx, move |_, _| room)` returning the window's handle,
    /// fullscreen on that display (a UIWindow on the external UIScreen / UIWindowScene on iOS). On a desktop
    /// build started with `ALCYO_PRESENTER_WINDOW=1` it opens as an ordinary window, to see the room's view
    /// as a window's root.
    fn present_on_external(&mut self, display: RoomDisplay, cx: &mut Context<Self>) {
        if self.presenter.room_window.is_some() {
            return;
        }
        self.presenter.window_tried = true;
        let workbench = cx.entity();
        if cfg!(target_os = "ios") || std::env::var("ALCYO_PRESENTER_WINDOW").is_ok_and(|v| v == "1") {
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(point(px(0.), px(0.)), size(display.size.width / 2., display.size.height / 2.)))),
                titlebar: None,
                focus: false,
                ..Default::default()
            };
            // Opened once the workbench is let go of: the new window draws the room straight away, which reads it.
            cx.defer(move |cx| {
                let room = workbench.clone();
                // On the iPad, a window on the external display's own scene; on a desktop, an ordinary one.
                if let Some(external) = crate::platform::room_display(cx).filter(|_| cfg!(target_os = "ios")) {
                    match crate::platform::open_room_window(cx, &external, move |_, cx| cx.new(|cx| RoomView::new(room, cx))) {
                        Ok((handle, _)) => workbench.update(cx, |this, _| this.presenter.room_window = Some(handle)),
                        Err(error) => eprintln!("the room's window: {error:#}"),
                    }
                    return;
                }
                let _ = &display;
                match cx.open_window(options, move |_, cx| cx.new(|cx| RoomView::new(room, cx))) {
                    Ok(handle) => workbench.update(cx, |this, _| this.presenter.room_window = Some(handle.into())),
                    Err(error) => eprintln!("the room's window: {error:#}"),
                }
            });
        }
    }

    /// The board the room is shown now.
    fn room_shows(&self) -> Shown {
        match self.presenter.room_page {
            Some(page) if page != self.live.page() && page < self.live.pages() => Shown::Page(page),
            _ => Shown::Live,
        }
    }

    /// Send a board to the room. The board being drawn on follows the talk again.
    fn send_to_room(&mut self, page: usize, cx: &mut Context<Self>) {
        self.presenter.room_page = (page != self.live.page()).then_some(page);
        // A circle is drawn on the board it points at: the tool is put down when the room looks elsewhere.
        if self.presenter.room_page.is_some() {
            let mut ink = self.ink.borrow_mut();
            ink.circling = false;
            ink.drawing = None;
        }
        cx.notify();
    }

    /// Make the pictures of the boards not being drawn on again when what they are changed, off the main thread
    /// once laid out, and the live board's thumbnail now and then as its ink arrives.
    fn keep_art(&mut self, cx: &mut Context<Self>) {
        let key = (self.live.id().to_string(), self.live.pages(), self.live.page());
        let (frame, captions) = (self.presenter.frame.get(), self.presenter.captions);
        // The live board's thumbnail, from its finished strokes.
        {
            let ink = self.ink.borrow();
            let made_from = (ink.generation, ink.finished.len());
            let mut art = self.presenter.art.borrow_mut();
            let stale = art.live_thumb.as_ref().is_none_or(|(_, from, at)| *from != made_from && (at.elapsed() >= THUMB_EVERY || ink.queue.is_empty()));
            if stale {
                let level = raster_level(thumb_view(&ink.extent).k * 1.1, 2.0);
                let mut tier = Tier::new(level);
                for stroke in &ink.finished {
                    tier.fill(stroke);
                }
                if let Some((old, _, _)) = art.live_thumb.replace((tier, made_from, Instant::now())) {
                    art.retired.extend(old.release());
                }
            } else if art.live_thumb.as_ref().is_some_and(|(_, from, _)| *from != made_from) {
                // More is coming: look again once it is due.
                cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(THUMB_EVERY).await;
                    let _ = this.update(cx, |_, cx| cx.notify());
                })
                .detach();
            }
        }
        let mut art = self.presenter.art.borrow_mut();
        if art.making || art.key.as_ref() == Some(&key) {
            return;
        }
        art.making = true;
        art.key = Some(key.clone());
        drop(art);
        // Laid out here, where the glyphs are; drawn and rasterized elsewhere.
        let images = self.live.board_images();
        let live = self.live.page();
        let read = |reference: &str| match &self.store {
            Some(store) => store.page_image(self.live.id(), reference).ok(),
            None => alcyo_meetings::decode_data_url(reference).ok(),
        };
        let mut composed = vec![];
        {
            let glyphs = &mut self.ink.borrow_mut().glyphs;
            for (i, (graph, memory)) in self.live.board_pages().into_iter().enumerate() {
                if i == live {
                    continue;
                }
                let image = images.get(i).cloned().flatten().and_then(|r| read(&r));
                composed.push((i, alcyo_export::ink::compose_board(&graph, &memory, image.as_ref(), &Default::default(), glyphs)));
            }
        }
        let insets = frame_insets(frame.0, captions);
        let made = cx.background_executor().spawn(async move {
            composed
                .into_iter()
                .map(|(i, composed)| {
                    let drawn = composed.draw();
                    let extent = drawn.bounds.clone();
                    let v = fit_into(&extent, frame.0, frame.1, insets);
                    // As sharp as the iPad's frame on its dense screen, or a 1080p projector, whichever is more.
                    let mut room = Tier::new(raster_level(v.k * (1920.0 / frame.0).max(1.0), 2.0));
                    let mut thumb = Tier::new(raster_level(thumb_view(&extent).k * 1.1, 2.0));
                    fill_tier(&mut room, &drawn);
                    fill_tier(&mut thumb, &drawn);
                    let page = drawn.page.as_ref().and_then(|p| page_render_image(&p.image.bytes).ok().map(|(image, _)| (image, p.w as f32, p.h as f32)));
                    let pictures = drawn.pictures.iter().filter_map(|p| Some((page_render_image(&p.image.bytes).ok()?.0, [p.x as f32, p.y as f32, p.w as f32, p.h as f32]))).collect();
                    (i, PageArt { room, thumb, extent, page, pictures })
                })
                .collect::<Vec<_>>()
        });
        let art = self.presenter.art.clone();
        cx.spawn(async move |this, cx| {
            let made = made.await;
            let mut art = art.borrow_mut();
            art.making = false;
            // A meeting switched meanwhile has its own pictures made next.
            if art.key.as_ref() == Some(&key) {
                for (i, page) in made {
                    if let Some(old) = art.pages.insert(i, page) {
                        art.retire(old);
                    }
                }
            } else {
                art.key = None;
            }
            drop(art);
            let _ = this.update(cx, |_, cx| cx.notify());
        })
        .detach();
    }

    /// The two lines of captions: the line before, small, and the newest one large, with who is saying it.
    /// The newest is the line being heard, while there is one.
    fn caption_lines(&self) -> (Option<(Option<String>, String)>, Option<(Option<String>, String, bool)>, usize) {
        let spoken: Vec<(Option<String>, String)> = self
            .live
            .lines
            .iter()
            .filter(|l| !l["manual"].as_bool().unwrap_or(false))
            .map(|l| (l["speaker"].as_str().filter(|s| !s.is_empty()).map(str::to_string), crate::i18n::line_text(l)))
            .filter(|(_, text)| !text.trim().is_empty())
            .collect();
        let count = spoken.len() + usize::from(self.draft.is_some());
        let mut lines = spoken.into_iter().rev();
        match self.draft.as_ref().filter(|d| !d.text.trim().is_empty()) {
            Some(draft) => (lines.next(), Some((self.speaker.clone(), draft.text.clone(), true)), count),
            None => {
                let newest = lines.next().map(|(who, text)| (who, text, false));
                (lines.next(), newest, count)
            }
        }
    }

    /// Exactly what the room sees, drawn into whatever holds it: the iPad's frame, or the external display's
    /// window. `width` is how wide it is drawn, which scales the captions; `external` says it is the room's
    /// own window, which then keeps the pen writing even while the iPad's screen is not drawn.
    pub(super) fn room_picture(&self, width: f32, external: bool, radius: f32) -> AnyElement {
        let s = width / DESIGN_W;
        let (ink, art, frame, glide) = (self.ink.clone(), self.presenter.art.clone(), self.presenter.frame.clone(), self.presenter.glide.clone());
        let live_page = self.live.page();
        let captions = self.presenter.captions;
        let insets = frame_insets(frame.get().0, captions);
        // One layer per board shown: the one before fades out under the new one, which glides in from the
        // side the board lies on.
        let layer = |shown: Shown| {
            let (ink, art, frame, glide) = (ink.clone(), art.clone(), frame.clone(), glide.clone());
            canvas(
                |_, _, _| (),
                move |bounds: Bounds<Pixels>, (), window: &mut Window, _: &mut App| {
                    let mut art = art.borrow_mut();
                    for image in std::mem::take(&mut art.retired) {
                        let _ = window.drop_image(image);
                    }
                    window.with_content_mask(Some(gpui_kit::ContentMask { bounds }), |window| match shown {
                        Shown::Live => {
                            let mut b = ink.borrow_mut();
                            if external {
                                b.advance();
                                if !b.queue.is_empty() || !b.sliding.is_empty() || b.gliding() {
                                    window.request_animation_frame();
                                }
                            }
                            paint_live(window, bounds, &b, &mut art, &glide, live_page, frame.get(), insets);
                        }
                        Shown::Page(page) => {
                            if let Some(page_art) = art.pages.get_mut(&page) {
                                paint_still(window, bounds, page_art, frame.get(), insets, false);
                            } else {
                                paint_dots(window, bounds, GRID * f32::from(bounds.size.width) / frame.get().0, 2.0);
                            }
                        }
                    });
                },
            )
            .absolute()
            .size_full()
            .into_any_element()
        };
        let current = self.presenter.shown.unwrap_or((self.room_shows(), self.live.page()));
        let board_of = |shown: Shown, live: usize| match shown {
            Shown::Live => live,
            Shown::Page(p) => p,
        };
        let mut picture = div().absolute().size_full().overflow_hidden();
        let fading = self.presenter.before.filter(|(_, _, at)| at.elapsed() < SWITCH);
        if let Some((old, _, _)) = fading {
            let el = layer(old);
            picture = picture.child(div().absolute().size_full().child(el).with_animation(("room-out", self.presenter.switches), motion::ease(SWITCH), |d, t| d.opacity(1. - t)));
        }
        let el = layer(current.0);
        let from = fading.map_or(0.0, |(old, old_live, _)| (board_of(current.0, current.1) as f32 - board_of(old, old_live) as f32).signum());
        let slide = 36.0 * s * from;
        picture = picture.child(
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .w_full()
                .child(el)
                .with_animation(("room-in", self.presenter.switches), motion::ease(SWITCH), move |d, t| d.left(px(slide * (1. - t))).opacity(if slide == 0.0 && fading.is_none() { 1.0 } else { t })),
        );

        // The captions: the line before, small and grey with who said it, and the newest large, rolling up
        // as each line arrives.
        let ticker = captions.then(|| {
            let (before, newest, count) = self.caption_lines();
            let speakers = self.live.speakers();
            let paper = pal().paper;
            let listening = self.pipeline.listening();
            div()
                .absolute()
                .left_0()
                .right_0()
                .bottom_0()
                .pt(px(16. * s))
                .px(px(28. * s))
                .pb(px(18. * s))
                .flex()
                .flex_col()
                .gap(px(4. * s))
                .bg(linear_gradient(180., linear_color_stop(alpha(paper, 0.), 0.), linear_color_stop(alpha(paper, 0.96), 0.22)))
                .when_some(before, |d, (who, text)| {
                    let line = match who {
                        Some(who) => format!("{who} · {text}"),
                        None => text,
                    };
                    d.child(div().text_size(px(16. * s)).text_color(rgb(pal().faint)).whitespace_nowrap().overflow_hidden().text_ellipsis().child(line))
                })
                .when_some(newest, |d, (who, text, heard)| {
                    let caret = div().flex_none().w(px(2. * s)).h(px(24. * s)).ml(px(-8. * s)).bg(rgb(pal().text));
                    d.child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(12. * s))
                            .text_size(px(24. * s))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(rgb(pal().text))
                            .when_some(who, |d, who| {
                                let (background, ink) = speaker_colors(&speakers, &who);
                                d.child(
                                    div()
                                        .flex_none()
                                        .size(px(30. * s))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .rounded_full()
                                        .bg(rgb(background))
                                        .text_color(rgb(ink))
                                        .text_size(px(14. * s))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(initial(&who)),
                                )
                            })
                            .child(div().min_w_0().whitespace_nowrap().overflow_hidden().text_ellipsis().child(text))
                            // The caret stays while the line is still being heard.
                            .when(heard || listening, |d| d.child(caret.with_animation("room-caret", motion::pulse(), |d, t| d.opacity(t)))),
                    )
                })
                .rounded_b(px(radius))
                .with_animation(("room-ticker", count), motion::ease(Duration::from_millis(320)), move |d, t| d.mt(px(30. * s * (1. - t))))
        });

        div()
            .relative()
            .size_full()
            .overflow_hidden()
            .rounded(px(radius))
            .bg(rgb(pal().paper))
            .font(theme::interface_font())
            .child(picture)
            .when_some(ticker, |d, ticker| d.child(ticker))
            .into_any_element()
    }

    /// The presenter's screen on the iPad: the dark surround, the top bar, the frame with what the room sees,
    /// and the controls along the bottom.
    pub(super) fn presenter_screen(&mut self, parts: super::ipad::Parts, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let size_ = window.viewport_size();
        let (w, h) = (f32::from(size_.width), f32::from(size_.height));
        let safe = crate::platform::safe_area(window);
        let top = f32::from(safe.top);
        let bar_bottom = (f32::from(safe.bottom) + 2.0).max(22.0);
        // The frame: as wide as the screen allows, 16:9, between the bars.
        let frame_top = top + TOP_H + 4.0;
        let room = h - bar_bottom - BAR_H - 11.0 - frame_top;
        let mut fw = w - 2.0 * SIDE;
        let mut fh = (fw * 9.0 / 16.0).round();
        if fh > room {
            fh = room;
            fw = (fh * 16.0 / 9.0).round();
        }
        let frame_left = ((w - fw) / 2.0).round();
        let frame_top = frame_top + ((room - fh) / 2.0).max(0.0).round();
        self.presenter.frame.set((fw, fh));
        if self.presenter.display.is_none() {
            self.presenter.display = room_display(cx);
        }
        // Started in presenter mode with a display there: the room gets its window once this frame is done.
        if let Some(display) = self.presenter.display.clone().filter(|_| self.presenter.room_window.is_none() && !self.presenter.window_tried) {
            self.presenter.window_tried = true;
            cx.defer_in(window, move |this, _, cx| this.present_on_external(display, cx));
        }
        self.keep_art(cx);
        // What the room shows: a change starts a cross-fade.
        let now = (self.room_shows(), self.live.page());
        if self.presenter.shown.is_some_and(|shown| shown != now) {
            let (old, old_live) = self.presenter.shown.expect("checked");
            self.presenter.before = Some((old, old_live, Instant::now()));
            self.presenter.switches += 1;
        }
        self.presenter.shown = Some(now);
        let listening = self.pipeline.listening();
        let dialog = parts.dialog;

        // The pen goes on writing while the iPad shows the frame instead of the board, and the frame is where
        // a circle drawn on it is measured from.
        let (driver_ink, frame_cell, frame_at) = (self.ink.clone(), self.presenter.frame.clone(), self.presenter.frame_at.clone());
        let driver = canvas(
            move |bounds: Bounds<Pixels>, _: &mut Window, _: &mut App| {
                frame_cell.set((f32::from(bounds.size.width), f32::from(bounds.size.height)));
                frame_at.set(bounds.origin);
            },
            move |_, (), window: &mut Window, _: &mut App| {
                let mut b = driver_ink.borrow_mut();
                b.advance();
                for image in std::mem::take(&mut b.retired) {
                    let _ = window.drop_image(image);
                }
            },
        )
        .absolute()
        .size_full();

        let live_shown = self.room_shows() == Shown::Live;
        // A point in the frame, where the board's own canvas has it: the tools measure there.
        let on_canvas = {
            let (glide, frame, frame_at) = (self.presenter.glide.clone(), self.presenter.frame.clone(), self.presenter.frame_at.clone());
            Rc::new(move |b: &Ink, at: Point<Pixels>| {
                let Some(g) = glide.get() else { return at };
                let (v, _) = g.now(frame.get());
                let origin = frame_at.get();
                let board = [v.x + f32::from(at.x - origin.x) / v.k, v.y + f32::from(at.y - origin.y) / v.k];
                let iv = b.shown();
                b.canvas.origin + point(px((board[0] - iv.x) * iv.k), px((board[1] - iv.y) * iv.k))
            })
        };
        let (press, moved) = (self.ink.clone(), self.ink.clone());
        let (on_press, on_move, on_touch) = (on_canvas.clone(), on_canvas.clone(), on_canvas.clone());
        let touch_ink = self.ink.clone();
        let touch_this = cx.entity().downgrade();
        let frame = div()
            .id("presenter-frame")
            .absolute()
            .left(px(frame_left))
            .top(px(frame_top))
            .w(px(fw))
            .h(px(fh))
            .rounded(px(18.))
            .overflow_hidden()
            .bg(rgb(pal().paper))
            .shadow(vec![
                BoxShadow { color: alpha(0xff_ff_ff, 0.08), offset: point(px(0.), px(0.)), blur_radius: px(0.), spread_radius: px(1.), inset: false },
                BoxShadow { color: alpha(0, 0.5), offset: point(px(0.), px(20.)), blur_radius: px(50.), spread_radius: px(0.), inset: false },
            ])
            .child(driver)
            .child(self.room_picture(fw, false, 18.))
            // With the circle in hand, a drag on the frame circles things on the board the room sees.
            .when(live_shown, |d| {
                d.on_mouse_down(MouseButton::Left, move |e: &MouseDownEvent, window, _| {
                    let mut b = press.borrow_mut();
                    let at = on_press(&b, e.position);
                    if b.tool_down(at) {
                        window.refresh();
                    }
                })
                .on_mouse_move(move |e: &MouseMoveEvent, window, _| {
                    let mut b = moved.borrow_mut();
                    let at = on_move(&b, e.position);
                    if e.pressed_button == Some(MouseButton::Left) && b.tool_drag(at) {
                        window.refresh();
                    }
                })
                .on_mouse_up(MouseButton::Left, cx.listener(|this, _: &MouseUpEvent, _, cx| {
                    this.tool_up(cx);
                }))
                .child(
                    canvas(
                        |_, _, _| (),
                        move |bounds: Bounds<Pixels>, (), window: &mut Window, _: &mut App| {
                            let (touch_ink, touch_this, on_touch) = (touch_ink.clone(), touch_this.clone(), on_touch.clone());
                            window.on_mouse_event(move |e: &gpui_kit::TouchDragEvent, phase, window, cx| {
                                if phase != gpui_kit::DispatchPhase::Bubble {
                                    return;
                                }
                                match e.phase {
                                    gpui_kit::TouchPhase::Started => {
                                        let mut b = touch_ink.borrow_mut();
                                        let (start, at) = (on_touch(&b, e.start_position), on_touch(&b, e.position));
                                        if bounds.contains(&e.start_position) && b.tool_down(start) {
                                            b.tool_drag(at);
                                            window.prevent_default();
                                            window.refresh();
                                        }
                                    }
                                    gpui_kit::TouchPhase::Moved => {
                                        let mut b = touch_ink.borrow_mut();
                                        let at = on_touch(&b, e.position);
                                        if b.tool_drag(at) {
                                            window.refresh();
                                        }
                                    }
                                    gpui_kit::TouchPhase::Ended | gpui_kit::TouchPhase::Cancelled => {
                                        let _ = touch_this.update(cx, |this, cx| this.tool_up(cx));
                                    }
                                }
                            });
                        },
                    )
                    .absolute()
                    .size_full(),
                )
            })
            // Only on the iPad: what is in the frame is what the room sees.
            .child(
                div()
                    .absolute()
                    .top(px(12.))
                    .right(px(14.))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .font_family(theme::MONO)
                    .text_size(px(10.5))
                    .text_color(rgb(0x8a_8a_86))
                    .child(div().size(px(6.)).rounded_full().bg(rgb(CONNECTED)))
                    .child(t("presenter-room-sees-this").to_uppercase()),
            );

        div()
            .id("workbench")
            .track_focus(&self.focus)
            .font(theme::interface_font())
            .on_key_down(cx.listener(Self::on_key))
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(rgb(SURROUND))
            .text_color(rgb(ON_DARK))
            .child(self.presenter_top(top, w, cx))
            .child(frame)
            .child(self.presenter_bar(bar_bottom, listening, cx))
            .when(self.presenter.menu, |d| d.child(self.presenter_menu(top, cx)))
            .when_some(dialog, |d, dialog| d.child(dialog))
            .into_any_element()
    }

    /// The top bar: the display the room watches, the meeting, Captions or Board only, and the ⋯ menu.
    fn presenter_top(&self, top: f32, w: f32, cx: &mut Context<Self>) -> gpui_kit::Div {
        let display = self.presenter.display.as_ref().map(|d| d.name.clone());
        let connected = display.is_some();
        let pill = div()
            .flex_none()
            .h(px(34.))
            .flex()
            .items_center()
            .gap(px(8.))
            .pl(px(10.))
            .pr(px(14.))
            .rounded_full()
            .bg(if connected { alpha(CONNECTED, 0.16) } else { alpha(0xff_ff_ff, 0.08) })
            .text_color(rgb(if connected { CONNECTED_TEXT } else { MUTED_ON_DARK }))
            .text_size(px(13.))
            .font_weight(FontWeight::SEMIBOLD)
            .whitespace_nowrap()
            .child(icon("ipad/airplay").size(px(18.)))
            .child(display.unwrap_or_else(|| t("presenter-no-display").to_string()));
        let speakers = self.live.speakers().len();
        let title = div()
            .absolute()
            .left_0()
            .right_0()
            .top_0()
            .bottom_0()
            .flex()
            .items_center()
            .justify_center()
            .gap(px(10.))
            .child(div().max_w(px(w * 0.34)).overflow_hidden().whitespace_nowrap().text_ellipsis().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child(self.live.title().to_string()))
            .when(speakers > 0, |d| d.child(div().font_family(theme::MONO).text_size(px(11.)).text_color(rgb(FAINT_ON_DARK)).whitespace_nowrap().child(ta!("presenter-speakers", n = speakers))));
        let captions = self.presenter.captions;
        let segment = |id: &'static str, on: bool| {
            div()
                .id(id)
                .h(px(32.))
                .flex()
                .items_center()
                .gap(px(6.))
                .px(px(12.))
                .rounded(px(9.))
                .text_size(px(13.))
                .font_weight(FontWeight::MEDIUM)
                .whitespace_nowrap()
                .cursor_pointer()
                .text_color(rgb(if on { 0xff_ff_ff } else { MUTED_ON_DARK }))
                .hover(|d| d.text_color(rgb(0xff_ff_ff)))
        };
        let lit = |id: &'static str, on: bool, el: gpui_kit::Stateful<gpui_kit::Div>| el.with_spring(id, SpringAnimation::new(motion::GLIDE).to(on), |d, v| d.bg(alpha(0xff_ff_ff, 0.16 * v.0.clamp(0., 1.))));
        let toggle = div()
            .flex_none()
            .flex()
            .p(px(3.))
            .gap(px(2.))
            .rounded(px(12.))
            .bg(alpha(0xff_ff_ff, 0.08))
            .child(lit(
                "presenter-captions-lit",
                captions,
                segment("presenter-captions", captions)
                    .child(icon("ipad/cc").size(px(18.)))
                    .child(t("presenter-captions"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.presenter.captions = true;
                        cx.notify();
                    })),
            ))
            .child(lit(
                "presenter-board-only-lit",
                !captions,
                segment("presenter-board-only", !captions).child(t("presenter-board-only")).on_click(cx.listener(|this, _, _, cx| {
                    this.presenter.captions = false;
                    cx.notify();
                })),
            ));
        let more = dark_button("presenter-more", "more", 44.0, self.presenter.menu).on_click(cx.listener(|this, _, _, cx| {
            this.presenter.menu = !this.presenter.menu;
            cx.notify();
        }));
        div()
            .absolute()
            .left_0()
            .right_0()
            .top(px(top))
            .h(px(TOP_H))
            .flex()
            .items_center()
            .gap(px(10.))
            .px(px(SIDE))
            .child(title)
            .child(div().relative().child(pill))
            .child(div().flex_1())
            .child(div().relative().child(toggle))
            .child(div().relative().child(more))
    }

    /// The ⋯ menu: back to the meeting's own screen, or to the meetings and settings.
    fn presenter_menu(&self, top: f32, cx: &mut Context<Self>) -> gpui_kit::Div {
        let row = |id: &'static str, label: String| {
            div()
                .id(id)
                .h(px(44.))
                .flex()
                .items_center()
                .px(px(16.))
                .rounded(px(10.))
                .text_size(px(15.))
                .text_color(rgb(ON_DARK))
                .cursor_pointer()
                .hover(|d| d.bg(alpha(0xff_ff_ff, 0.08)))
                .active(|d| d.bg(alpha(0xff_ff_ff, 0.14)))
                .child(label)
        };
        let menu = div()
            .id("presenter-menu")
            .occlude()
            .absolute()
            .right(px(SIDE))
            .top(px(top + TOP_H))
            .w(px(240.))
            .p(px(6.))
            .flex()
            .flex_col()
            .rounded(px(16.))
            .bg(alpha(0x1c_1c_1e, 0.97))
            .shadow(vec![
                BoxShadow { color: alpha(0xff_ff_ff, 0.1), offset: point(px(0.), px(0.)), blur_radius: px(0.), spread_radius: px(0.5), inset: false },
                BoxShadow { color: alpha(0, 0.5), offset: point(px(0.), px(12.)), blur_radius: px(32.), spread_radius: px(0.), inset: false },
            ])
            .child(row("presenter-leave", t("presenter-leave").to_string()).on_click(cx.listener(|this, _, _, cx| this.leave_presenter(false, cx))))
            .child(div().h(px(1.)).mx(px(10.)).my(px(2.)).bg(alpha(0xff_ff_ff, 0.1)))
            .child(row("presenter-meetings", t("presenter-meetings-and-settings").to_string()).on_click(cx.listener(|this, _, _, cx| {
                // The room keeps its picture while the meetings and settings are looked at.
                this.panel = Panel::Library;
                this.leave_presenter(true, cx);
            })))
            .with_animation("presenter-menu-in", motion::ease(motion::BASE), |d, t| d.opacity(t).mt(px(-6. * (1. - t))));
        div()
            .absolute()
            .size_full()
            .child(div().id("presenter-menu-veil").absolute().size_full().on_click(cx.listener(|this, _, _, cx| {
                this.presenter.menu = false;
                cx.notify();
            })))
            .child(menu)
    }

    /// The controls along the bottom: listening, undo, the boards, and Follow live.
    fn presenter_bar(&self, bottom: f32, listening: bool, cx: &mut Context<Self>) -> gpui_kit::Div {
        let editable = self.editable();
        let level = self.level;
        let recorded = self.recorded();
        // Listening: a big red capsule with its level and time; at rest, Start listening on the dark.
        let capsule = editable.then(|| {
            div()
                .id("presenter-listen")
                .group("presenter-listen")
                .flex_none()
                .h(px(56.))
                .flex()
                .items_center()
                .gap(px(10.))
                .pl(px(10.))
                .pr(px(20.))
                .rounded_full()
                .text_color(rgb(0xff_ff_ff))
                .text_size(px(17.))
                .font_weight(FontWeight::SEMIBOLD)
                .whitespace_nowrap()
                .cursor_pointer()
                .on_click(cx.listener(|this, _, _, cx| {
                    this.toggle_mic();
                    cx.notify();
                }))
                .with_spring("presenter-listen-fill", SpringAnimation::new(motion::SETTLE).to(listening), |d, v| {
                    let v = v.0.clamp(0., 1.);
                    let mut fill = motion::mix(0x2a_2a_2c, pal().recording, v);
                    fill.a = 1.0;
                    d.bg(fill)
                })
                .child(
                    div()
                        .id("presenter-listen-stop")
                        .flex_none()
                        .size(px(38.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_full()
                        .bg(alpha(0xff_ff_ff, 0.22))
                        .group_active("presenter-listen", |d| d.bg(alpha(0xff_ff_ff, 0.32)))
                        // The red dot morphs into the white square that stops.
                        .child(div().with_spring("presenter-listen-glyph", SpringAnimation::new(motion::SETTLE).to(listening), |d, v| {
                            let v = v.0.clamp(0., 1.);
                            let mut c = motion::mix(pal().recording, 0xff_ff_ff, v);
                            c.a = 1.0;
                            d.size(px(12. - 2. * v)).rounded(px(6. - 4. * v)).bg(c)
                        })),
                )
                .child(if listening { t("listening") } else { t("start-listening") })
                .when(listening, |d| {
                    d.child(div().flex().items_center().gap(px(2.5)).h(px(22.)).children((0..6).map(|i| {
                        let h = 5.0 + level * 17.0 * (0.5 + 0.5 * ((i as f32) * 1.7 + 0.6).sin().abs());
                        div().w(px(3.)).h(px(h.min(22.))).rounded(px(2.)).bg(rgb(0xff_ff_ff))
                    })))
                })
                .when(listening || !recorded.is_zero(), |d| d.child(div().font_family(theme::MONO).text_size(px(15.)).font_weight(FontWeight::MEDIUM).child(clock(recorded.as_secs_f64() * 1000.0))))
        });
        let can_undo = self.live.can_undo();
        // Undo takes back the board's last change, and the room's picture of the live board is made again from
        // what is left: added to stroke by stroke, it would otherwise keep what was taken back.
        let undo = editable.then(|| {
            dark_button("presenter-undo", "undo", BIG, false).when(!can_undo, |d| d.opacity(0.35)).when(can_undo, |d| {
                d.on_click(cx.listener(|this, _, _, cx| {
                    this.step(false, cx);
                    let mut art = this.presenter.art.borrow_mut();
                    if let Some(old) = art.live_room.take() {
                        let released = old.tier.release();
                        art.retired.extend(released);
                    }
                    if let Some((tier, _, _)) = art.live_thumb.take() {
                        let released = tier.release();
                        art.retired.extend(released);
                    }
                    cx.notify();
                }))
            })
        });

        // The boards as thumbnails: the one in the room lifted, with the accent around it. A tap sends a board
        // to the room.
        let (pages, live) = (self.live.pages(), self.live.page());
        let on_room = match self.room_shows() {
            Shown::Live => live,
            Shown::Page(p) => p,
        };
        let frame = self.presenter.frame.get();
        let insets = frame_insets(frame.0, self.presenter.captions);
        let thumbs = div()
            .id("presenter-thumbs")
            .flex()
            .items_center()
            .gap(px(10.))
            .px(px(4.))
            .py(px(8.))
            .overflow_x_scroll()
            .children((0..pages).map(|page| {
                let on = page == on_room;
                let (ink, art) = (self.ink.clone(), self.presenter.art.clone());
                let picture = canvas(
                    |_, _, _| (),
                    move |bounds: Bounds<Pixels>, (), window: &mut Window, _: &mut App| {
                        let mut art = art.borrow_mut();
                        window.with_content_mask(Some(gpui_kit::ContentMask { bounds }), |window| {
                            if page == live {
                                let b = ink.borrow();
                                let v = thumb_view(&b.extent);
                                let k = v.k * f32::from(bounds.size.width) / THUMB_W;
                                let origin = bounds.origin + point(px(-v.x * k), px(-v.y * k));
                                if let Some((_, w, h)) = b.page.as_ref().filter(|_| b.graph.as_ref().is_some_and(|g| g.underlay().is_some())) {
                                    if let Some(small) = b.page_small.clone() {
                                        let area = Bounds::new(origin, size(px(w * k), px(h * k)));
                                        let _ = window.paint_image(area, area, gpui_kit::Corners::default(), small, 0, false);
                                    }
                                }
                                if let Some((tier, _, _)) = art.live_thumb.as_mut() {
                                    tier.under.paint(window, origin, k, Some(bounds));
                                    tier.layer.paint(window, origin, k, Some(bounds));
                                }
                            } else if let Some(page_art) = art.pages.get_mut(&page) {
                                paint_still(window, bounds, page_art, frame, insets, true);
                            }
                        });
                    },
                )
                .absolute()
                .size_full();
                div().relative().flex_none().w(px(THUMB_W)).h(px(THUMB_H)).child(
                    div()
                        .id(gpui_kit::ElementId::NamedInteger("presenter-thumb".into(), page as u64))
                        .group("presenter-thumb")
                        .absolute()
                        .cursor_pointer()
                        .rounded(px(10.))
                        .overflow_hidden()
                        .bg(rgb(pal().paper))
                        .child(picture)
                        .child(div().absolute().left(px(6.)).bottom(px(4.)).font_family(theme::MONO).text_size(px(10.)).text_color(rgb(0x6e_6e_6e)).child(format!("{}", page + 1)))
                        .on_click(cx.listener(move |this, _, _, cx| this.send_to_room(page, cx)))
                        // The board in the room lifts out of the row: a little bigger, ringed, on a shadow.
                        .with_spring(gpui_kit::ElementId::NamedInteger("presenter-thumb-lift".into(), page as u64), SpringAnimation::new(motion::GLIDE).to(on), |d, v| {
                            let v = v.0;
                            let grow = 3.0 * v;
                            let ring = alpha(ACCENT_ON_DARK, v.clamp(0., 1.));
                            d.left(px(-grow))
                                .top(px(-grow))
                                .w(px(THUMB_W + 2. * grow))
                                .h(px(THUMB_H + 2. * grow))
                                .opacity(0.55 + 0.45 * v.clamp(0., 1.))
                                .shadow(vec![
                                    BoxShadow { color: alpha(0xff_ff_ff, 0.08 * (1. - v.clamp(0., 1.))), offset: point(px(0.), px(0.)), blur_radius: px(0.), spread_radius: px(1.), inset: false },
                                    BoxShadow { color: ring, offset: point(px(0.), px(0.)), blur_radius: px(0.), spread_radius: px(2.5), inset: false },
                                    BoxShadow { color: alpha(0, 0.45 * v.clamp(0., 1.)), offset: point(px(0.), px(8. * v)), blur_radius: px(18. * v), spread_radius: px(0.), inset: false },
                                ])
                        }),
                )
            }))
            .when(editable, |d| {
                d.child(
                    div()
                        .id("presenter-new-board")
                        .group("presenter-new-board")
                        .flex_none()
                        .w(px(THUMB_W))
                        .h(px(THUMB_H))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(10.))
                        .bg(alpha(0xff_ff_ff, 0.06))
                        .text_color(rgb(MUTED_ON_DARK))
                        .cursor_pointer()
                        .hover(|d| d.bg(alpha(0xff_ff_ff, 0.1)))
                        .active(|d| d.bg(alpha(0xff_ff_ff, 0.14)))
                        .child(div().id("presenter-new-board-plus").group_active("presenter-new-board", |d| d.opacity(0.7)).child(icon("ipad/plus").size(px(22.))))
                        .on_click(cx.listener(|this, _, _, cx| {
                            // A new board goes up in the room straight away.
                            this.presenter.room_page = None;
                            this.new_board(cx);
                        })),
                )
            });
        let following = self.presenter.room_page.is_none();
        let follow = div()
            .id("presenter-follow")
            .group("presenter-follow")
            .flex_none()
            .h(px(BIG))
            .flex()
            .items_center()
            .gap(px(8.))
            .px(px(18.))
            .rounded_full()
            .text_size(px(14.))
            .font_weight(FontWeight::SEMIBOLD)
            .whitespace_nowrap()
            .cursor_pointer()
            .on_click(cx.listener(|this, _, _, cx| {
                this.presenter.room_page = None;
                cx.notify();
            }))
            .with_spring("presenter-follow-tint", SpringAnimation::new(motion::GLIDE).to(following), |d, v| {
                let v = v.0.clamp(0., 1.);
                let mut ink = motion::mix(MUTED_ON_DARK, ACCENT_ON_DARK, v);
                ink.a = 1.0;
                d.bg(motion::mix(0x1d_1d_1f, 0x23_27_33, v)).text_color(ink)
            })
            .child(div().id("presenter-follow-label").group_active("presenter-follow", |d| d.opacity(0.7)).flex().items_center().gap(px(8.)).child(icon("ipad/zoom").size(px(18.))).child(t("presenter-follow-live")));
        div()
            .absolute()
            .left(px(SIDE))
            .right(px(SIDE))
            .bottom(px(bottom))
            .h(px(BAR_H))
            .flex()
            .items_center()
            .gap(px(12.))
            .when_some(capsule, |d, c| d.child(c))
            .when_some(undo, |d, c| d.child(c))
            .child(div().flex_1().min_w_0().flex().justify_center().child(thumbs))
            .child(follow)
    }
}

/// A round button on the dark surround: a line icon on a faint fill, `on` lit in the accent. Pressed, the
/// fill draws in a little, as if pushed.
fn dark_button(id: &'static str, name: &str, size_: f32, on: bool) -> gpui_kit::Stateful<gpui_kit::Div> {
    div()
        .id(id)
        .group(id)
        .relative()
        .flex_none()
        .size(px(size_))
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .text_color(rgb(if on { SURROUND } else { ON_DARK }))
        .child(
            div()
                .id(gpui_kit::ElementId::Name(format!("{id}-fill").into()))
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .rounded_full()
                .bg(if on { rgb(ON_DARK).into() } else { alpha(0xff_ff_ff, 0.08) })
                .group_hover(id, |d| d.bg(if on { rgb(0xff_ff_ff).into() } else { alpha(0xff_ff_ff, 0.12) }))
                .group_active(id, |d| d.top(px(2.)).left(px(2.)).size(px(size_ - 4.)).bg(if on { rgb(0xdd_dd_dd).into() } else { alpha(0xff_ff_ff, 0.16) })),
        )
        .child(div().relative().child(icon(&format!("ipad/{name}")).size(px(if size_ >= BIG { 24. } else { 22. }))))
}

/// The room's own window on the external display: nothing but [`Workbench::room_picture`], filling it.
pub(super) struct RoomView {
    workbench: WeakEntity<Workbench>,
    _changes: gpui_kit::Subscription,
}

impl RoomView {
    pub(super) fn new(workbench: Entity<Workbench>, cx: &mut Context<Self>) -> Self {
        let changes = cx.observe(&workbench, |_, _, cx| cx.notify());
        Self { workbench: workbench.downgrade(), _changes: changes }
    }
}

impl Render for RoomView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let width = f32::from(window.viewport_size().width);
        match self.workbench.upgrade() {
            Some(workbench) => div().size_full().bg(rgb(0)).flex().items_center().justify_center().child(
                // The board's 16:9 in the middle of whatever the display is, black around it.
                div().w_full().h(px(width * 9.0 / 16.0)).child(workbench.read(cx).room_picture(width, true, 0.)),
            ),
            None => div().size_full().bg(rgb(0)),
        }
    }
}
