//! The iPad's meetings page, "Home" (#341): a place of its own rather than a strip beside the board. Boards are
//! what people remember, so each meeting is shown by its board, as a card in a grid grouped by day, with who
//! spoke and for how long. Starting a meeting is the first card; the search field looks inside what was said;
//! filters stand in for the desktop's Pinned group; a long press (or a right click) holds a card's actions.
//!
//! Opening a meeting grows its card into the board, and coming back shrinks the board into its card again.
//!
//! Only the layout and the pictures live here. What each control does is the workbench's own, shared with the
//! desktop's meetings page. A child of `app`, so it reaches the workbench's state directly.

use super::ipad::{glass, who, Parts};
use super::*;
use gpui_kit::{AnyElement, BoxShadow, FontWeight, LongPressEvent, TouchPhase};
use std::sync::Mutex;

/// The page's side margins, the gutter between cards, and the height of a card's board.
const SIDE: f32 = 32.0;
const GUTTER: f32 = 18.0;
const THUMB_H: f32 = 168.0;
/// The room a board's picture is drawn in at most, in pixels: twice the widest landscape card.
const PICTURE: (f32, f32) = (560.0, 336.0);
/// How many meetings get a picture; older ones show their paper until the list is narrowed to them.
const PICTURES: usize = 120;
/// Between one card rising in and the next.
const STAGGER: Duration = Duration::from_millis(25);
/// A softer spring than the panels', for a card growing into the whole screen.
const ZOOM: gpui_kit::SpringConfig = gpui_kit::SpringConfig::new(260., 32.25, 1.);
/// The marker colours a board is drawn in, which the dark palette turns into chalk.
const INKS: [u32; 3] = [0x1e_1e_1e, 0xe0_31_31, 0x19_71_c2];

/// What the grid is narrowed to.
#[derive(Clone, Copy, PartialEq)]
enum Filter {
    All,
    Pinned,
    Summary,
    Imported,
}

/// What a meeting's file says beyond its row in the list, read with its picture.
#[derive(Clone, Default)]
struct Meta {
    summary: bool,
    /// An imported document's pages, when the meeting is one.
    imported: Option<usize>,
    recorded_ms: f64,
}

/// A meeting's picture, for the version of it and the palette it was drawn for.
struct Thumb {
    updated: f64,
    dark: bool,
    image: Option<Arc<RenderImage>>,
    meta: Meta,
}

/// A card growing into the board, or the board shrinking back into its card.
struct Zoom {
    image: Option<Arc<RenderImage>>,
    card: Bounds<Pixels>,
    opening: bool,
    started: Instant,
    /// Counts zooms, so each one starts its animation afresh.
    count: usize,
}

/// What the meetings page keeps for itself, beside the workbench's own state.
pub(super) struct Home {
    thumbs: HashMap<String, Thumb>,
    /// Meetings whose picture is being drawn.
    drawing: HashSet<String>,
    /// Pictures replaced since the last frame, whose textures go back during the next.
    stale: Vec<Arc<RenderImage>>,
    /// One set of handwriting for every picture, loaded with the first.
    glyphs: Arc<Mutex<Option<Glyphs>>>,
    filter: Filter,
    /// Counts the times the page came up, so the cards rise in each time.
    opened: usize,
    /// Whether the page was up at the last frame.
    shown: bool,
    /// Where each card's board was laid out, for a zoom to start from or end at.
    cards: Rc<RefCell<HashMap<String, Bounds<Pixels>>>>,
    zoom: Option<Zoom>,
    zooms: usize,
    /// The card whose actions are out, and where the press was.
    menu: Option<(String, Point<Pixels>)>,
    menus: usize,
    /// The card under a finger, which gives a little.
    pressed: Option<String>,
    /// The meeting being renamed in its card, and the box its name is written in.
    renaming: Option<String>,
    rename_input: Option<Entity<InputState>>,
    placeholder: bool,
    /// The app's name as the splash wrote it, and when the page took over from the splash: on a cold start
    /// the name flows into the page's title and the rest follows.
    intro: Option<(Vec<Inked>, Instant)>,
    /// Who said the first line of each transcript passage found, and when, by meeting and line: read from the
    /// meeting once for each set of results.
    said: (u64, HashMap<(String, usize), (Option<String>, Option<f64>)>),
}

impl Home {
    pub(super) fn new() -> Self {
        Self {
            thumbs: HashMap::new(),
            drawing: HashSet::new(),
            stale: vec![],
            glyphs: Arc::new(Mutex::new(None)),
            filter: Filter::All,
            opened: 0,
            shown: false,
            cards: Rc::new(RefCell::new(HashMap::new())),
            zoom: None,
            zooms: 0,
            menu: None,
            menus: 0,
            pressed: None,
            renaming: None,
            rename_input: None,
            placeholder: false,
            intro: None,
            said: (u64::MAX, HashMap::new()),
        }
    }
}

/// How wide the panel of passages found is, beside the cards.
const RESULTS_W: f32 = 400.0;

/// Where `terms` occur in `text`, as byte ranges, case left aside. Text whose lowercase is not as long as itself (a
/// few letters change length) is left unmarked rather than marked in the wrong place.
fn marked(text: &str, terms: &[String]) -> Vec<std::ops::Range<usize>> {
    let lower = text.to_lowercase();
    if lower.len() != text.len() {
        return vec![];
    }
    let mut out: Vec<std::ops::Range<usize>> = vec![];
    for term in terms.iter().filter(|t| !t.is_empty()) {
        let mut from = 0;
        while let Some(at) = lower[from..].find(term.as_str()) {
            let start = from + at;
            out.push(start..start + term.len());
            from = start + term.len();
        }
    }
    out.sort_by_key(|r| r.start);
    // Overlaps would confuse the text's runs: the earlier wins.
    let mut kept: Vec<std::ops::Range<usize>> = vec![];
    for r in out {
        if kept.last().is_none_or(|k| r.start >= k.end) {
            kept.push(r);
        }
    }
    kept
}

/// A meeting as a card shows it.
struct Entry {
    id: String,
    title: String,
    created: f64,
    updated: f64,
    speakers: Vec<String>,
    /// The meeting on the board.
    open: bool,
}

/// A colour with some see-through.
fn alpha(color: u32, a: f32) -> Hsla {
    let mut c: Hsla = rgb(color).into();
    c.a = a;
    c
}

/// The system's grey fill for a field or a plain button.
fn field() -> Hsla {
    alpha(0x76_76_80, if theme::is_dark() { 0.24 } else { 0.12 })
}

/// A hairline ring around something, drawn outside it.
fn ring(color: Hsla, width: f32) -> BoxShadow {
    BoxShadow { color, offset: point(px(0.), px(0.)), blur_radius: px(0.), spread_radius: px(width), inset: false }
}

/// A spring rising in after `delay`, as a duration-based animation, so a row of cards can follow each other.
fn rise(delay: Duration) -> gpui_kit::Animation {
    let (settle, easing) = gpui_kit::sampled_easing(motion::SETTLE, 0.002);
    let (wait, run) = (delay.as_secs_f32(), settle.as_secs_f32().max(0.001));
    let total = wait + run;
    gpui_kit::Animation::new(delay + settle).with_easing(move |t| {
        let at = t * total;
        if at <= wait { 0. } else { easing(((at - wait) / run).min(1.)) }
    })
}

/// How long a zoom takes, and its curve.
fn zoom_animation() -> (Duration, gpui_kit::Animation) {
    let (settle, easing) = gpui_kit::sampled_easing(ZOOM, 0.002);
    (settle, gpui_kit::Animation::new(settle).with_easing(easing))
}

/// Dots on paper, `step` apart, each `radius` round, with `shade` saying how strong each one is where it is
/// (0 to 1, from its place across and down the area).
fn dots(step: f32, radius: f32, color: Hsla, shade: fn(f32, f32) -> f32) -> gpui_kit::Canvas<()> {
    canvas(
        |_, _, _| (),
        move |bounds, (), window: &mut Window, _| {
            let (w, h) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
            let (x0, y0) = (f32::from(bounds.origin.x), f32::from(bounds.origin.y));
            let mut y = step / 2.;
            while y < h {
                let mut x = step / 2.;
                while x < w {
                    let a = shade(x / w, y / h);
                    if a > 0.01 {
                        let mut c = color;
                        c.a *= a;
                        let dot = Bounds::new(point(px(x0 + x - radius), px(y0 + y - radius)), size(px(radius * 2.), px(radius * 2.)));
                        window.paint_quad(gpui_kit::fill(dot, c).corner_radii(px(radius)));
                    }
                    x += step;
                }
                y += step;
            }
        },
    )
}

/// A board's picture, as large as fits inside the area less `inset` (top, right, bottom, left), centred.
fn picture(image: Arc<RenderImage>, inset: [f32; 4]) -> gpui_kit::Canvas<()> {
    canvas(
        |_, _, _| (),
        move |bounds, (), window: &mut Window, _| {
            let room_w = f32::from(bounds.size.width) - inset[1] - inset[3];
            let room_h = f32::from(bounds.size.height) - inset[0] - inset[2];
            let size = image.size(0);
            let (w, h) = (size.width.0 as f32, size.height.0 as f32);
            if room_w <= 0. || room_h <= 0. || w <= 0. || h <= 0. {
                return;
            }
            let k = (room_w / w).min(room_h / h);
            let (fw, fh) = (w * k, h * k);
            let x = f32::from(bounds.origin.x) + inset[3] + (room_w - fw) / 2.;
            let y = f32::from(bounds.origin.y) + inset[0] + (room_h - fh) / 2.;
            let fitted = Bounds::new(point(px(x), px(y)), size_of(fw, fh));
            let _ = window.paint_image(bounds, fitted, gpui_kit::Corners::default(), image.clone(), 0, false);
        },
    )
}

fn size_of(w: f32, h: f32) -> gpui_kit::Size<Pixels> {
    size(px(w), px(h))
}

/// Draw a meeting's board as a picture, off the main thread: the board it is on, or the first page of an
/// imported document. With what its file says that its row in the list does not.
fn draw_thumb(store: &Store, id: &str, glyphs: &Mutex<Option<Glyphs>>, hanzi: HanziSource, inks: [(u32, u32); 3]) -> Option<(Option<Arc<RenderImage>>, Meta)> {
    use tiny_skia::{BlendMode, FillRule, FilterQuality, IntSize, Paint, PathBuilder as SkPath, Pixmap, PixmapPaint, Transform};
    let live = Live::from_loaded(store.load(id).ok()??);
    let pages = live.pages();
    let documents = (0..pages).filter(|&i| live.document_of(i).is_some()).count();
    let imported = (documents > 0 && documents == pages || live.read_only == Some(ViewOnly::ImportedContent)).then_some(pages);
    let meta = Meta { summary: !live.summary().is_empty(), imported, recorded_ms: live.recorded_ms() };
    let page = if imported.is_some() { 0 } else { live.page().min(pages - 1) };
    let images: Vec<Option<alcyo_meetings::PageImage>> =
        live.board_images().iter().enumerate().map(|(i, r)| if i == page { r.as_ref().and_then(|r| store.page_image(id, r).ok()) } else { None }).collect();
    let asset = |name: &str| store.page_image(id, name).ok();
    let (_, boards) = alcyo_export::words_and_boards(&live, true, &alcyo_export::kept_text, &images, &asset);
    let board = boards.into_iter().nth(page)?;
    if board.graph.nodes.is_empty() && board.image.is_none() {
        return Some((None, meta));
    }
    let drawn = {
        let mut held = glyphs.lock().ok()?;
        if held.is_none() {
            *held = Some(Glyphs::load(hanzi).ok()?);
        }
        alcyo_export::ink::compose_board(&board.graph, &board.memory, board.image.as_ref(), &board.sources, held.as_mut()?).draw()
    };
    let (x, y, w, h) = drawn.frame(24.0);
    let scale = (f64::from(PICTURE.0) / w).min(f64::from(PICTURE.1) / h).min(1.5) as f32;
    let mut canvas = Pixmap::new((w as f32 * scale).round().max(1.) as u32, (h as f32 * scale).round().max(1.) as u32)?;
    let at = Transform::from_translate(-x as f32, -y as f32).post_scale(scale, scale);
    // tiny-skia keeps its colours multiplied by their alpha.
    let pixmap = |image: &alcyo_meetings::PageImage| -> Option<Pixmap> {
        let decoded = image::load_from_memory(&image.bytes).ok()?.to_rgba8();
        let (w, h) = decoded.dimensions();
        let mut data = decoded.into_raw();
        for p in data.chunks_exact_mut(4) {
            let a = u16::from(p[3]);
            for c in &mut p[..3] {
                *c = (u16::from(*c) * a / 255) as u8;
            }
        }
        Pixmap::from_vec(data, IntSize::from_wh(w, h)?)
    };
    let paint = PixmapPaint { opacity: 1.0, blend_mode: BlendMode::SourceOver, quality: FilterQuality::Bilinear };
    if let Some(page) = &drawn.page {
        if let Some(picture) = pixmap(&page.image) {
            let fit = Transform::from_scale((page.w / f64::from(picture.width())) as f32, (page.h / f64::from(picture.height())) as f32).post_concat(at);
            canvas.draw_pixmap(0, 0, picture.as_ref(), &paint, fit, None);
        }
    }
    for p in &drawn.pictures {
        if let Some(picture) = pixmap(&p.image) {
            let fit = Transform::from_scale((p.w / f64::from(picture.width())) as f32, (p.h / f64::from(picture.height())) as f32).post_translate(p.x as f32, p.y as f32).post_concat(at);
            canvas.draw_pixmap(0, 0, picture.as_ref(), &paint, fit, None);
        }
    }
    for stroke in &drawn.strokes {
        let Some((first, rest)) = stroke.outline.split_first() else { continue };
        let mut path = SkPath::new();
        path.move_to(first[0], first[1]);
        for p in rest {
            path.line_to(p[0], p[1]);
        }
        path.close();
        let Some(path) = path.finish() else { continue };
        let color = inks.iter().find(|(from, _)| *from == stroke.style.rgb).map_or(stroke.style.rgb, |(_, to)| *to);
        let [_, r, g, b] = color.to_be_bytes();
        let mut fill = Paint::default();
        fill.set_color_rgba8(r, g, b, (stroke.style.alpha * 255.0) as u8);
        fill.anti_alias = true;
        canvas.fill_path(&path, &fill, FillRule::Winding, at, None);
    }
    // GPUI takes straight-alpha BGRA.
    let mut raw = canvas.data().to_vec();
    for px in raw.chunks_exact_mut(4) {
        let a = u32::from(px[3]);
        if a > 0 && a < 255 {
            for c in &mut px[..3] {
                *c = ((u32::from(*c) * 255 + a / 2) / a).min(255) as u8;
            }
        }
        px.swap(0, 2);
    }
    let buffer = image::RgbaImage::from_raw(canvas.width(), canvas.height(), raw)?;
    Some((Some(Arc::new(RenderImage::new([image::Frame::new(buffer)]))), meta))
}

impl Workbench {
    /// Start drawing the pictures the cards need: missing, out of date, or drawn for the other palette.
    fn want_thumbs(&mut self, entries: &[Entry], cx: &mut Context<Self>) {
        let Some(store) = self.store.clone() else { return };
        let dark = theme::is_dark();
        let inks = INKS.map(|c| (c, pal().ink(c)));
        for e in entries.iter().take(PICTURES) {
            let fresh = self.home.thumbs.get(&e.id).is_some_and(|t| t.updated == e.updated && t.dark == dark);
            if fresh || self.home.drawing.contains(&e.id) {
                continue;
            }
            self.home.drawing.insert(e.id.clone());
            let (store, id, updated, glyphs, hanzi) = (store.clone(), e.id.clone(), e.updated, self.home.glyphs.clone(), self.hanzi.clone());
            cx.spawn(async move |this, cx| {
                let which = id.clone();
                let drawn = cx.background_executor().spawn(async move { draw_thumb(&store, &which, &glyphs, hanzi, inks) }).await;
                let _ = this.update(cx, |this, cx| {
                    this.home.drawing.remove(&id);
                    let (image, meta) = drawn.unwrap_or_default();
                    if let Some(old) = this.home.thumbs.insert(id, Thumb { updated, dark, image, meta }).and_then(|t| t.image) {
                        this.home.stale.push(old);
                    }
                    cx.notify();
                });
            })
            .detach();
        }
    }

    /// The splash is done and the page takes over: on the iPad's meetings page its written name flows into the
    /// title. Elsewhere nothing changes.
    pub(super) fn home_intro(&mut self, word: Vec<Inked>) {
        if crate::platform::ipad_layout() && self.panel == Panel::Library {
            self.home.intro = Some((word, Instant::now()));
        }
    }

    /// Open a meeting from its card, growing the card into the board.
    fn home_open(&mut self, id: &str, cx: &mut Context<Self>) {
        self.home.menu = None;
        if let Some(card) = self.home.cards.borrow().get(id).copied() {
            self.home.zooms += 1;
            let image = self.home.thumbs.get(id).and_then(|t| t.image.clone());
            self.home.zoom = Some(Zoom { image, card, opening: true, started: Instant::now(), count: self.home.zooms });
        }
        self.open_meeting(id, cx);
        if self.panel == Panel::Library {
            // It could not be opened, and the page stays.
            self.home.zoom = None;
        }
    }

    /// Start a meeting and listen at once.
    fn home_listen(&mut self, cx: &mut Context<Self>) {
        self.home.menu = None;
        self.new_meeting(cx);
        if self.panel != Panel::Library && !self.pipeline.listening() {
            self.toggle_mic();
        }
        cx.notify();
    }

    /// Give a meeting a new name: the one on the board directly, any other in its file.
    fn rename_meeting(&mut self, id: &str, title: &str, cx: &mut Context<Self>) {
        let title = title.trim();
        if title.is_empty() {
            return;
        }
        if id == self.live.id() {
            if title != self.live.title() {
                self.live.set_title(title);
            }
        } else if let Some(store) = self.store.clone() {
            let renamed = store.load(id).and_then(|loaded| {
                let loaded = loaded.context("that meeting no longer exists")?;
                let mut live = Live::from_loaded(loaded);
                live.set_title(title);
                let (origin, revision) = (live.origin.clone(), live.revision);
                store.save(id, live.snapshot(), &origin, revision)
            });
            match renamed.and_then(|_| store.list()) {
                Ok(list) => self.library = list,
                Err(e) => self.notice = Some(format!("{e:#}")),
            }
        }
        if let Some(m) = self.library.iter_mut().find(|m| m.id == id) {
            m.title = title.to_string();
        }
        cx.notify();
    }

    fn home_start_rename(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.home.menu = None;
        let title = if id == self.live.id() { self.live.title().to_string() } else { self.library.iter().find(|m| m.id == id).map(|m| m.title.clone()).unwrap_or_default() };
        let input = self.home.rename_input.get_or_insert_with(|| {
            let input = cx.new(|cx| InputState::new(window, cx));
            cx.subscribe_in(&input, window, |this: &mut Workbench, _, event, _, cx| {
                if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                    this.home_finish_rename(cx);
                }
            })
            .detach();
            input
        });
        input.update(cx, |state, cx| {
            state.set_value(title, window, cx);
            state.focus(window, cx);
            state.select_all(window, cx);
        });
        self.home.renaming = Some(id.to_string());
        cx.notify();
    }

    fn home_finish_rename(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.home.renaming.take() else { return };
        let title = self.home.rename_input.as_ref().map(|i| i.read(cx).value().to_string()).unwrap_or_default();
        self.rename_meeting(&id, &title, cx);
    }

    /// Export a meeting from its card: it opens, with Export over it.
    fn home_export(&mut self, id: &str, cx: &mut Context<Self>) {
        self.home.menu = None;
        self.open_meeting(id, cx);
        if self.live.id() == id {
            self.toggle_panel(Panel::Export, cx);
        }
    }

    /// A meeting's diagnostic bundle from its card (#360): it opens, with the bundle's sheet over it.
    fn home_diagnostics(&mut self, id: &str, cx: &mut Context<Self>) {
        self.home.menu = None;
        self.open_meeting(id, cx);
        if self.live.id() == id {
            self.toggle_panel(Panel::Diagnostics, cx);
        }
    }

    /// The meeting screen on the iPad, with a card still growing into it.
    pub(super) fn home_zoom_over(&mut self, screen: AnyElement, window: &mut Window, _cx: &mut Context<Self>) -> AnyElement {
        self.home.shown = false;
        self.home.menu = None;
        self.home.renaming = None;
        let layer = self.zoom_layer(window);
        match layer {
            Some(layer) => div().relative().size_full().child(screen).child(layer).into_any_element(),
            None => screen,
        }
    }

    /// A card growing into the whole screen or shrinking back into its place, as a picture of the board that
    /// moves between the two. Nothing once it has arrived.
    fn zoom_layer(&mut self, window: &Window) -> Option<AnyElement> {
        let (duration, animation) = zoom_animation();
        if self.home.zoom.as_ref().is_some_and(|z| z.started.elapsed() > duration + Duration::from_millis(60)) {
            self.home.zoom = None;
        }
        let zoom = self.home.zoom.as_ref()?;
        let screen = window.viewport_size();
        let (card, opening, image) = (zoom.card, zoom.opening, zoom.image.clone());
        let p = pal();
        let dot = rgb(if theme::is_dark() { 0x1c_1c_1c } else { 0xdc_dc_d6 }).into();
        let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
        Some(
            div()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .child(
                    div()
                        .absolute()
                        .overflow_hidden()
                        .bg(rgb(p.paper))
                        .child(dots(14., 1., dot, |_, _| 1.).absolute().size_full())
                        .when_some(image, |d, image| d.child(picture(image, [10., 8., 8., 8.]).absolute().size_full()))
                        .with_animation(("home-zoom", zoom.count), animation, move |d, t| {
                            // The card's place at 0, the whole screen at 1.
                            let s = if opening { t } else { 1. - t };
                            let x = lerp(f32::from(card.origin.x), 0., s);
                            let y = lerp(f32::from(card.origin.y), 0., s);
                            let w = lerp(f32::from(card.size.width), f32::from(screen.width), s);
                            let h = lerp(f32::from(card.size.height), f32::from(screen.height), s);
                            // Growing, the picture gives way to the board itself over the last stretch.
                            let shown = if opening { ((1. - t) / 0.4).clamp(0., 1.) } else { 1. };
                            d.left(px(x)).top(px(y)).w(px(w)).h(px(h)).rounded(px(16. * (1. - s))).opacity(shown)
                        }),
                )
                .into_any_element(),
        )
    }

    /// The meetings page over the whole screen.
    pub(super) fn home_screen(&mut self, parts: Parts, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Parts { notices, dialog, preparing, shortcuts, .. } = parts;
        for image in self.home.stale.drain(..) {
            let _ = window.drop_image(image);
        }
        // Coming back from a meeting: the cards rise in again, and the board shrinks back into its card.
        if !self.home.shown {
            self.home.shown = true;
            self.home.opened += 1;
            self.home.filter = Filter::All;
            let id = self.live.id().to_string();
            let card = self.home.cards.borrow().get(&id).copied();
            if let Some(card) = card.filter(|_| self.home.zoom.as_ref().is_none_or(|z| !z.opening)) {
                self.home.zooms += 1;
                let image = self.home.thumbs.get(&id).and_then(|t| t.image.clone());
                self.home.zoom = Some(Zoom { image, card, opening: false, started: Instant::now(), count: self.home.zooms });
            } else {
                self.home.zoom = None;
            }
        }
        let (intro_for, _) = intro_animation();
        if self.home.intro.as_ref().is_some_and(|(_, at)| at.elapsed() > intro_for + INTRO_REST + STAGGER * 16 + Duration::from_millis(400)) {
            self.home.intro = None;
        }
        // Cold from the splash, the rest of the page waits for the title to arrive.
        let after = if self.home.intro.is_some() { INTRO_REST } else { Duration::ZERO };
        if !self.home.placeholder {
            self.home.placeholder = true;
            self.library_query.update(cx, |state, cx| state.set_placeholder(t("home-search"), window, cx));
        }
        let size = window.viewport_size();
        let width = f32::from(size.width);
        let safe = crate::platform::safe_area(window);
        let top = f32::from(safe.top);
        // What was found by meaning, shown beside the cards as the desktop shows it: the passages themselves.
        let typed_now = self.library_query.read(cx).value().trim().to_string();
        let results_shown = self.searching_by_meaning() && typed_now.chars().count() >= 2 && self.search.as_ref().is_some_and(|s| s.asking || !s.hits.is_empty());
        let beside = results_shown && width >= 900.;
        let grid_width = if beside { width - RESULTS_W - GUTTER } else { width };
        let columns: usize = if grid_width < 900. { 2 } else if grid_width >= 1300. { 5 } else { 4 };
        let listening = self.pipeline.listening();
        let p = pal();
        let dark = theme::is_dark();

        // Every readable meeting, and the one on the board if it is not saved yet but has something in it.
        let current = self.live.id().to_string();
        let mut entries: Vec<Entry> = self
            .library
            .iter()
            .filter(|m| m.read_error.is_none())
            .map(|m| {
                let open = m.id == current;
                Entry {
                    id: m.id.clone(),
                    title: if open { self.live.title().to_string() } else { m.title.clone() },
                    created: m.created_at,
                    updated: m.updated_at,
                    speakers: if open { self.live.speakers().into_iter().take(4).collect() } else { m.speakers.clone() },
                    open,
                }
            })
            .collect();
        if self.store.is_some() && !entries.iter().any(|e| e.open) && (self.live.has_content() || listening) {
            let now = chrono::Local::now().timestamp_millis() as f64;
            let created = self.live.created_at().unwrap_or(now);
            entries.insert(0, Entry { id: current.clone(), title: self.live.title().to_string(), created, updated: now, speakers: self.live.speakers().into_iter().take(4).collect(), open: true });
        }
        self.want_thumbs(&entries, cx);

        // What the search field narrows to: names that match, and meetings where what was said does.
        let typed = self.library_query.read(cx).value().to_string();
        self.search_typed(&typed);
        let query = typed.trim().to_lowercase();
        let mut hits: HashSet<String> = self.search.as_ref().filter(|s| !query.is_empty() && s.error.is_none()).map(|s| s.hits.iter().map(|h| h.meeting.clone()).collect()).unwrap_or_default();
        // Where search by meaning cannot run (the iPad), the words themselves are looked for.
        if !self.searching_by_meaning() && !query.is_empty() {
            hits.extend(self.word_hits(&query));
        }
        let pinned = |id: &str| self.pinned.iter().any(|p| p == id);
        let meta = |id: &str| self.home.thumbs.get(id).map(|t| t.meta.clone()).unwrap_or_default();
        let counts = [
            entries.len(),
            entries.iter().filter(|e| pinned(&e.id)).count(),
            entries.iter().filter(|e| meta(&e.id).summary).count(),
            entries.iter().filter(|e| meta(&e.id).imported.is_some()).count(),
        ];
        let filter = self.home.filter;
        entries.retain(|e| {
            let kept = match filter {
                Filter::All => true,
                Filter::Pinned => pinned(&e.id),
                Filter::Summary => meta(&e.id).summary,
                Filter::Imported => meta(&e.id).imported.is_some(),
            };
            kept && (query.is_empty() || e.title.to_lowercase().contains(&query) || hits.contains(&e.id))
        });
        let found = entries.len();

        // By day, as the sidebar has them: today, yesterday, and before.
        use chrono::{Datelike, TimeZone};
        let today = chrono::Local::now().date_naive();
        let day_of = |ms: f64| chrono::Local.timestamp_millis_opt(ms as i64).single().map(|t| t.date_naive());
        let age = |e: &Entry| day_of(e.updated).map_or(2, |d| (today - d).num_days().clamp(0, 2)) as usize;
        let mut groups: [Vec<&Entry>; 3] = [vec![], vec![], vec![]];
        for e in &entries {
            groups[age(e)].push(e);
        }
        // The meeting on the board leads its day, next to the card that starts a new one.
        for group in &mut groups {
            group.sort_by_key(|e| !e.open);
        }
        let starting = filter == Filter::All && query.is_empty() && self.editable_store();
        let opened = self.home.opened;
        let mut order = 0usize;
        let mut sections: Vec<AnyElement> = vec![];
        for (age, group) in groups.iter().enumerate() {
            let start_card = age == 0 && starting;
            if group.is_empty() && !start_card {
                continue;
            }
            let name = [t("today-2"), t("yesterday-2"), t("earlier-2")][age];
            let aside: Option<String> = match age {
                1 => Some((today - chrono::Duration::days(1)).format("%a %-d %b").to_string()),
                2 => {
                    let weeks: Vec<u32> = group.iter().filter_map(|e| day_of(e.updated)).map(|d| d.iso_week().week()).collect();
                    match (weeks.iter().max(), weeks.iter().min()) {
                        (Some(newest), Some(oldest)) if newest == oldest => Some(ta!("home-week", n = *newest)),
                        (Some(_), Some(_)) => Some(ta!("home-weeks", from = weeks[weeks.len() - 1], to = weeks[0])),
                        _ => None,
                    }
                }
                _ => None,
            };
            let mut cards: Vec<AnyElement> = vec![];
            if start_card {
                cards.push(self.start_card(opened, order, after, cx));
                order += 1;
            }
            for e in group {
                cards.push(self.meeting_card(e, age, opened, order, after, listening, cx));
                order += 1;
            }
            sections.push(
                div()
                    .flex()
                    .flex_col()
                    .when(!sections.is_empty(), |d| d.pt(px(46.)))
                    .child(
                        div()
                            .flex()
                            .items_baseline()
                            .gap(px(10.))
                            .pb(px(12.))
                            .child(div().text_size(px(20.)).font_weight(FontWeight::BOLD).child(name))
                            .when_some(aside, |d, aside| d.child(div().font_family(theme::MONO).text_size(px(11.)).text_color(rgb(p.faint)).child(aside))),
                    )
                    .child(div().grid().grid_cols(columns as u16).gap(px(GUTTER)).children(cards))
                    .into_any_element(),
            );
        }
        let nothing = sections.is_empty().then(|| {
            div().pt(px(40.)).text_size(px(15.)).text_color(rgb(p.faint)).child(if query.is_empty() { t("home-nothing-here") } else { t("home-nothing-found") })
        });

        // Header: the kingfisher and the page's name, then search, import, settings and a new meeting.
        let narrow = f32::from(window.viewport_size().width) < 760.0;
        let header = div()
            .absolute()
            .top(px(top + 14.))
            .left(px(crate::platform::clear_of_controls(window, SIDE, top, top + 72.)))
            .right(px(SIDE))
            .h(px(44.))
            .flex()
            .items_center()
            .gap(px(10.))
            .child(
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .text_size(px(if narrow { 28. } else { 34. }))
                    .child(theme::app_icon(if narrow { 30. } else { 36. }))
                    .child(div().font_weight(FontWeight::BOLD).whitespace_nowrap().child(t("meetings")))
                    .map(|brand| self.intro_brand(brand, window)),
            )
            .child(div().flex_1())
            .when(self.store.is_some(), |d| {
                d.child(
                    div()
                        .id("home-search")
                        .w(px(340.))
                        .flex_shrink(1.)
                        .min_w(px(if narrow { 110. } else { 150. }))
                        .h(px(44.))
                        .flex()
                        .items_center()
                        .gap(px(9.))
                        .pl(px(12.))
                        .pr(px(10.))
                        .rounded(px(13.))
                        .bg(field())
                        .text_size(px(15.5))
                        .child(div().flex_none().text_color(rgb(p.faint)).child(icon("ipad/search").size(px(18.))))
                        .child(div().flex_1().min_w_0().child(Input::new(&self.library_query).appearance(false).cleanable(true)))
                        .when(typed.is_empty(), |d| {
                            d.child(div().flex_none().px(px(5.)).py(px(1.)).rounded(px(5.)).border_1().border_color(rgb(p.line)).font_family(theme::MONO).text_size(px(10.5)).text_color(rgb(p.faint)).child("⌘K"))
                        }),
                )
                .child(self.plain_button("home-import", "import").on_click(cx.listener(|this, _, _, cx| this.pick_import_file(cx))))
            })
            .child(self.plain_button("home-settings", "sliders").on_click(cx.listener(|this, _, _, cx| this.toggle_panel(Panel::Settings, cx))))
            .when(self.editable_store(), |d| {
                d.child(
                    div()
                        .id("home-new")
                        .flex_none()
                        .h(px(44.))
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .pl(px(if narrow { 11. } else { 14. }))
                        .pr(px(if narrow { 11. } else { 18. }))
                        .rounded(px(13.))
                        .bg(rgb(p.button))
                        .text_color(rgb(p.on_button))
                        .text_size(px(15.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .whitespace_nowrap()
                        .cursor_pointer()
                        .hover(|d| d.bg(rgb(p.button_hover)))
                        .child(icon("ipad/plus").size(px(22.)))
                        // A narrow window keeps the plus alone.
                        .when(!narrow, |d| d.child(t("new-meeting")))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.home.menu = None;
                            this.new_meeting(cx);
                        })),
                )
            });

        // The filters, and how far the search index has come while it is being built.
        let chip = |id: &'static str, filter: Filter, label: &'static str, count: usize, pin: bool| {
            let on = self.home.filter == filter;
            div()
                .id(id)
                .flex_none()
                .h(px(34.))
                .flex()
                .items_center()
                .gap(px(6.))
                .px(px(14.))
                .rounded_full()
                .text_size(px(13.5))
                .font_weight(FontWeight::MEDIUM)
                .whitespace_nowrap()
                .cursor_pointer()
                .map(|d| if on { d.bg(rgb(p.button)).text_color(rgb(p.on_button)) } else { d.bg(rgb(p.surface)).shadow(vec![ring(rgb(p.line).into(), 1.)]).active(|d| d.bg(rgb(p.press))) })
                .when(pin, |d| d.child(icon("ipad/pin").size(px(14.))))
                .child(label)
                .when(count > 0, |d| d.child(div().opacity(0.6).child(count.to_string())))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.home.filter = filter;
                    this.home.opened += 1;
                    cx.notify();
                }))
        };
        let indexing = self.search.as_ref().and_then(|s| s.indexing.map(|done| (done, s.held)));
        // Where search by meaning cannot run, it says it looks for the words themselves, and how many it found.
        let word_search = (!self.searching_by_meaning() && !query.is_empty()).then(|| ta!("home-word-search", n = found));
        let chips = div()
            .absolute()
            .top(px(top + 72.))
            .left(px(SIDE))
            .right(px(SIDE))
            .h(px(34.))
            .flex()
            .items_center()
            .gap(px(8.))
            .child(chip("home-all", Filter::All, t("home-all"), counts[0], false))
            .child(chip("home-pinned", Filter::Pinned, t("pinned-2"), counts[1], true))
            .child(chip("home-summary", Filter::Summary, t("home-with-summary"), counts[2], false))
            .child(chip("home-imported", Filter::Imported, t("home-imported"), counts[3], false))
            .child(div().flex_1())
            .when_some(word_search, |d, said| d.child(div().flex_none().flex().items_center().gap(px(6.)).text_size(px(12.5)).text_color(rgb(p.faint)).whitespace_nowrap().child(icon("ipad/search").size(px(13.))).child(said)))
            .when_some(indexing, |d, ((done, total), held)| {
                let share = if total == 0 { 0. } else { (done as f32 / total as f32).clamp(0., 1.) };
                d.child(
                    div()
                        .flex_none()
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .text_size(px(12.5))
                        .text_color(rgb(p.faint))
                        .whitespace_nowrap()
                        .child(if held { t("search-index-paused") } else { t("indexing-for-search") })
                        .child(div().w(px(90.)).h(px(3.)).rounded(px(2.)).bg(rgb(p.track)).overflow_hidden().child(div().h_full().w(gpui_kit::relative(share)).bg(rgb(if held { p.faint } else { p.text }))))
                        .child(div().font_family(theme::MONO).text_size(px(11.)).child(format!("{done} / {total}"))),
                )
            });

        let list_top = top + 72. + 34.;
        let bottom = f32::from(safe.bottom);
        let results = results_shown.then(|| self.home_results(&typed_now, list_top, bottom, beside, cx));
        let list = div()
            .id("home-grid")
            .absolute()
            .top(px(list_top))
            .left_0()
            .right(px(if beside { RESULTS_W + SIDE } else { 0. }))
            .bottom_0()
            .when(results_shown && !beside, |d| d.invisible())
            .overflow_y_scroll()
            .child(div().pt(px(20.)).px(px(SIDE)).pb(px(bottom + 60.)).flex().flex_col().children(sections).when_some(nothing, |d, n| d.child(n)));
        let fade = div().absolute().left_0().right_0().bottom_0().h(px(70.)).bg(gpui_kit::linear_gradient(
            180.,
            gpui_kit::linear_color_stop(alpha(p.chrome, 0.), 0.),
            gpui_kit::linear_color_stop(rgb(p.chrome), 0.75),
        ));

        // A meeting just deleted can come back for a while.
        let deleted = self.deleted_meeting.clone().map(|(_, title)| {
            div().absolute().left_0().right_0().bottom(px(bottom + 20.)).flex().justify_center().child(
                glass(div(), false)
                    .occlude()
                    .h(px(48.))
                    .flex()
                    .items_center()
                    .gap(px(14.))
                    .pl(px(18.))
                    .pr(px(6.))
                    .rounded_full()
                    .text_size(px(14.))
                    .child(div().max_w(px(360.)).overflow_hidden().whitespace_nowrap().text_ellipsis().child(ta!("deleted-title", title = title.to_string())))
                    .child(
                        div()
                            .id("home-restore")
                            .h(px(36.))
                            .flex()
                            .items_center()
                            .px(px(14.))
                            .rounded_full()
                            .bg(rgb(p.button))
                            .text_color(rgb(p.on_button))
                            .font_weight(FontWeight::SEMIBOLD)
                            .cursor_pointer()
                            .child(t("undo"))
                            .on_click(cx.listener(|this, _, _, cx| this.restore_meeting(cx))),
                    )
                    .with_animation(("home-deleted", self.library.len()), rise(Duration::ZERO), |d, t| d.opacity(t).relative().top(px(16. * (1. - t)))),
            )
        });

        let menu = self.home_menu(window, cx);
        let zoom = self.zoom_layer(window);
        let _ = dark;
        div()
            .id("workbench")
            .track_focus(&self.focus)
            .font(theme::interface_font())
            .on_key_down(cx.listener(Self::on_key))
            .on_drop(cx.listener(|this, paths: &gpui_kit::ExternalPaths, _, cx| this.import_paths(paths.paths(), cx)))
            .on_mouse_up(MouseButton::Left, cx.listener(|this, _, _, cx| {
                if this.home.pressed.take().is_some() {
                    cx.notify();
                }
            }))
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(rgb(p.chrome))
            .text_color(rgb(p.text))
            .child(list)
            .child(fade)
            .when_some(results, |d, results| d.child(results))
            .when_some(self.intro_word(window), |d, word| d.child(word))
            .child(header)
            .child(chips.map(|chips| follow(chips, "home-intro-chips", after)))
            .when_some(deleted, |d, deleted| d.child(deleted))
            .when_some(notices, |d, notices| d.child(div().absolute().top(px(list_top)).left(px(SIDE)).right(px(SIDE)).child(notices)))
            .when_some(menu, |d, menu| d.child(menu))
            .when_some(zoom, |d, zoom| d.child(zoom))
            .when_some(preparing, |d, page| d.child(page))
            .when_some(dialog, |d, dialog| d.child(dialog))
            .when_some(shortcuts, |d, window| d.child(window))
            .into_any_element()
    }

    /// Whether new meetings can be made and kept: not in a sample.
    fn editable_store(&self) -> bool {
        self.store.is_some()
    }

    /// A 44 pt square button with a line icon, on the system's grey.
    fn plain_button(&self, id: &'static str, name: &str) -> gpui_kit::Stateful<gpui_kit::Div> {
        div()
            .id(id)
            .flex_none()
            .size(px(44.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(13.))
            .bg(field())
            .cursor_pointer()
            .active(|d| d.opacity(0.6))
            .child(icon(&format!("ipad/{name}")).size(px(22.)))
    }

    /// A card's board: the dotted paper with the picture of its board, giving a little under a finger.
    fn thumb_frame(&self, key: &str, index: usize) -> gpui_kit::Div {
        let _ = (key, index);
        let p = pal();
        div()
            .relative()
            .h(px(THUMB_H))
            .rounded(px(16.))
            .overflow_hidden()
            .bg(rgb(p.paper))
            .shadow(vec![ring(rgb(p.line).into(), 1.), BoxShadow { color: alpha(0, 0.04), offset: point(px(0.), px(1.)), blur_radius: px(2.), spread_radius: px(0.), inset: false }])
    }

    /// The passages search by meaning found, as a panel beside the cards (or over them in a narrow window): where
    /// things stand first, then what was said, each with its meeting, where in it, who said it and when, its words
    /// with the query's marked, and a tap to open the meeting there.
    fn home_results(&mut self, query: &str, top: f32, bottom: f32, beside: bool, cx: &mut Context<Self>) -> AnyElement {
        let p = pal();
        let (hits, asking, ticket) = self.search.as_ref().map(|s| (s.hits.clone(), s.asking, s.ticket)).unwrap_or_default();
        // Who said a transcript passage's first line, and when: read once for these results.
        let key = ticket.wrapping_mul(1 << 20).wrapping_add(hits.len() as u64);
        if self.home.said.0 != key {
            let mut said = HashMap::new();
            let mut loaded: HashMap<String, Option<serde_json::Map<String, serde_json::Value>>> = HashMap::new();
            for found in &hits {
                if let alcyo_search::Source::Transcript { first, .. } = found.source {
                    let raw = loaded.entry(found.meeting.clone()).or_insert_with(|| self.store.as_ref().and_then(|s| s.load(&found.meeting).ok().flatten()).map(|l| l.raw));
                    let line = raw.as_ref().and_then(|r| r.get("lines")).and_then(|l| l.get(first)).cloned();
                    let speaker = line.as_ref().and_then(|l| l["speaker"].as_str()).filter(|s| !s.is_empty()).map(str::to_string);
                    let ms = line.as_ref().and_then(|l| l["ms"].as_f64());
                    said.insert((found.meeting.clone(), first), (speaker, ms));
                }
            }
            self.home.said = (key, said);
        }
        let terms: Vec<String> = {
            let lower = query.to_lowercase();
            let words: Vec<String> = lower.split_whitespace().filter(|w| w.chars().count() >= 2).map(str::to_string).collect();
            if words.is_empty() { vec![lower] } else { words }
        };
        let mark = gpui_kit::HighlightStyle { background_color: Some(alpha(p.accent, if theme::is_dark() { 0.32 } else { 0.18 })), color: Some(rgb(p.text).into()), ..Default::default() };
        let row = |i: usize, found: alcyo_search::service::Found, said: Option<&(Option<String>, Option<f64>)>, cx: &mut Context<Self>| {
            let place = match &found.source {
                alcyo_search::Source::Transcript { .. } => t("transcript"),
                alcyo_search::Source::Board { .. } => t("board-2"),
                alcyo_search::Source::Summary { .. } => t("summary"),
            };
            // A transcript passage as it was said: its lines without the speaker in front, who is said above.
            let text: String = found.text.lines().map(|l| match found.source {
                alcyo_search::Source::Transcript { .. } => l.split_once(": ").map_or(l, |(_, said)| said).trim(),
                _ => l.trim(),
            }).filter(|l| !l.is_empty()).collect::<Vec<_>>().join(" ");
            let text: String = text.chars().take(240).collect();
            let ranges = marked(&text, &terms);
            let words = gpui_kit::StyledText::new(text).with_highlights(ranges.into_iter().map(|r| (r, mark)));
            let who = said.and_then(|(who, _)| who.clone());
            let when = said.and_then(|(_, ms)| *ms).map(alcyo_board::steps::clock);
            let open = found.clone();
            div()
                .id(("home-found", i))
                .flex()
                .flex_col()
                .gap(px(6.))
                .px(px(14.))
                .py(px(12.))
                .rounded(px(14.))
                .cursor_pointer()
                .hover(|d| d.bg(ipad_dialogs::well()))
                .active(|d| d.bg(ipad_dialogs::well()).opacity(0.8))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .min_w_0()
                        .text_size(px(12.5))
                        .text_color(rgb(p.faint))
                        .child(div().flex_none().max_w(px(180.)).overflow_hidden().whitespace_nowrap().text_ellipsis().font_weight(FontWeight::SEMIBOLD).text_color(rgb(p.text)).text_size(px(14.)).child(found.title.clone()))
                        .child(div().flex_none().child(place))
                        .when_some(who, |d, who| d.child(div().flex_none().child("·")).child(div().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().child(who)))
                        .when_some(when, |d, when| d.child(div().flex_none().font_family(theme::MONO).child(when))),
                )
                .child(div().text_size(px(14.)).line_height(gpui_kit::relative(1.45)).text_color(rgb(p.muted)).child(words))
                .on_click(cx.listener(move |this, _, _, cx| this.open_found(&open, cx)))
        };
        let said = self.home.said.1.clone();
        let heading = |text: &'static str| div().px(px(14.)).pt(px(14.)).pb(px(4.)).text_size(px(12.5)).font_weight(FontWeight::SEMIBOLD).text_color(rgb(p.faint)).child(text.to_uppercase());
        let (stands, spoken): (Vec<_>, Vec<_>) = hits.into_iter().enumerate().partition(|(_, f)| f.stands());
        let count = stands.len() + spoken.len();
        let head = div()
            .flex_none()
            .flex()
            .items_center()
            .gap(px(8.))
            .px(px(18.))
            .h(px(52.))
            .border_b_1()
            .border_color(rgb(p.line))
            .child(icon("ipad/search").size(px(16.)).text_color(rgb(p.muted)))
            .child(div().flex_1().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child(if asking && count == 0 { t("searching-what-was-said").to_string() } else { ta!("home-found-passages", n = count) }));
        let body = div()
            .id("home-found-list")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .p(px(6.))
            .flex()
            .flex_col()
            .when(!stands.is_empty(), |d| d.child(heading(t("where-things-stand"))).children(stands.into_iter().map(|(i, f)| {
                // A board or the summary was not said by one person at one moment.
                row(i, f, None, cx)
            })))
            .when(!spoken.is_empty(), |d| d.child(heading(t("what-was-said"))).children(spoken.into_iter().map(|(i, f)| {
                let key = match f.source {
                    alcyo_search::Source::Transcript { first, .. } => (f.meeting.clone(), first),
                    _ => (String::new(), 0),
                };
                let s = said.get(&key).cloned();
                row(i, f, s.as_ref(), cx)
            })));
        glass(div(), true)
            .id("home-found")
            .occlude()
            .absolute()
            .top(px(top + 12.))
            .bottom(px(bottom + 16.))
            .right(px(SIDE))
            .map(|d| if beside { d.w(px(RESULTS_W)) } else { d.left(px(SIDE)) })
            .flex()
            .flex_col()
            .rounded(px(20.))
            .overflow_hidden()
            .child(head)
            .child(body)
            .into_any_element()
    }

    /// Record where a card's board is laid out, and open its actions on a long press there.
    fn card_watch(&self, id: &str, cx: &mut Context<Self>) -> gpui_kit::Canvas<Bounds<Pixels>> {
        let (cards, key) = (self.home.cards.clone(), id.to_string());
        let this = cx.entity().downgrade();
        let pressed = id.to_string();
        canvas(
            move |bounds, _, _| {
                cards.borrow_mut().insert(key.clone(), bounds);
                bounds
            },
            move |_, bounds, window: &mut Window, _| {
                let (this, id) = (this.clone(), pressed.clone());
                window.on_mouse_event(move |event: &LongPressEvent, phase, _, cx| {
                    if phase.bubble() && event.phase == TouchPhase::Started && bounds.contains(&event.start_position) {
                        let id = id.clone();
                        let _ = this.update(cx, |this, cx| {
                            this.home.menus += 1;
                            this.home.pressed = None;
                            this.home.menu = Some((id, event.position));
                            cx.notify();
                        });
                    }
                });
            },
        )
    }

    /// The first card: start a meeting and listen, on the iPad's microphone.
    fn start_card(&self, opened: usize, order: usize, after: Duration, cx: &mut Context<Self>) -> AnyElement {
        let p = pal();
        let dark = theme::is_dark();
        let pressed = self.home.pressed.as_deref() == Some("");
        let (fill, edge) = if dark { (0x1c_1c_1e, Some(p.line)) } else { (0x11_11_11, None) };
        // Dots that fade out from the top right, where the kingfisher is.
        let vignette = |x: f32, y: f32| {
            let (dx, dy) = ((x - 0.85) / 1.2, (y - 0.1) / 0.9);
            let r = (dx * dx + dy * dy).sqrt();
            ((0.7 - r) / 0.5).clamp(0., 1.)
        };
        let board = div()
            .relative()
            .h(px(THUMB_H))
            .rounded(px(16.))
            .overflow_hidden()
            .bg(rgb(fill))
            .when_some(edge, |d, edge| d.shadow(vec![ring(rgb(edge).into(), 1.)]))
            .child(dots(6., 1.1, alpha(0xff_ff_ff, 0.16), vignette).absolute().size_full())
            .child(div().absolute().right(px(16.)).top(px(14.)).child(theme::app_icon(68.)))
            .child(
                div()
                    .absolute()
                    .left(px(16.))
                    .right(px(16.))
                    .bottom(px(16.))
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .text_size(px(12.))
                            .text_color(rgb(0xc4_c4_c4))
                            .child(div().size(px(7.)).rounded_full().bg(rgb(p.recording)).shadow(vec![ring(alpha(p.recording, 0.22), 3.)]))
                            .child(t("home-mic-ready")),
                    )
                    .child(div().mt(px(6.)).text_size(px(21.)).font_weight(FontWeight::SEMIBOLD).text_color(rgb(0xff_ff_ff)).child(t("start-listening"))),
            );
        let card = div()
            .id("home-start")
            .flex()
            .flex_col()
            .gap(px(9.))
            .cursor_pointer()
            .child(div().h(px(THUMB_H)).child(board.with_spring("home-press-start", SpringAnimation::new(motion::SNAP).to(pressed), |d, v| press(d, v.0))))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(3.))
                    .px(px(4.))
                    .child(div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child(t("new-meeting")))
                    .child(div().h(px(20.)).flex().items_center().text_size(px(12.5)).text_color(rgb(p.faint)).child(t("home-new-hint"))),
            )
            .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| {
                this.home.pressed = Some(String::new());
                cx.notify();
            }))
            .on_click(cx.listener(|this, _, _, cx| this.home_listen(cx)));
        stagger(card, opened, order, after)
    }

    /// A meeting's card: its board, its name, when and how long, and who spoke.
    fn meeting_card(&self, e: &Entry, age: usize, opened: usize, order: usize, after: Duration, listening: bool, cx: &mut Context<Self>) -> AnyElement {
        let p = pal();
        let thumb = self.home.thumbs.get(&e.id);
        let meta = thumb.map(|t| t.meta.clone()).unwrap_or_default();
        let pinned = self.pinned.iter().any(|x| *x == e.id);
        let live = e.open && listening;
        let pressed = self.home.pressed.as_deref() == Some(e.id.as_str());
        let renaming = self.home.renaming.as_deref() == Some(e.id.as_str());
        let index = self.library.iter().position(|m| m.id == e.id).unwrap_or(usize::MAX);
        // When: the time today and yesterday, the day before that; then how long, or where it stands.
        let when = {
            use chrono::TimeZone;
            chrono::Local.timestamp_millis_opt(e.created as i64).single().map(|t| if age < 2 { t.format("%H:%M").to_string() } else { t.format("%a %-d %b").to_string() }).unwrap_or_default()
        };
        let (page, pages) = if e.open { (self.live.page(), self.live.pages()) } else { (0, 1) };
        let recorded = if e.open { self.recorded().as_secs_f64() * 1000. } else { meta.recorded_ms };
        let detail = match () {
            _ if e.open && pages > 1 => Some(ta!("home-board-of", page = page + 1, pages = pages)),
            _ if meta.imported.is_some() => Some(ta!("home-imported-pages", n = meta.imported.unwrap_or(1))),
            _ if recorded >= 1000. => Some(ta!("home-minutes", n = (recorded / 60_000.).ceil().max(1.) as u64)),
            _ => None,
        };
        let line = match detail {
            Some(detail) if !when.is_empty() => format!("{when} · {detail}"),
            Some(detail) => detail,
            None => when,
        };
        let dot = rgb(if theme::is_dark() { 0x1c_1c_1c } else { 0xdc_dc_d6 }).into();
        let picture = thumb.and_then(|t| t.image.clone()).map(|image| {
            let fresh = format!("home-thumb-{}-{}", e.id, thumb.map_or(0., |t| t.updated) as u64);
            div()
                .absolute()
                .size_full()
                .child(picture(image, [10., 8., 8., 8.]).absolute().size_full())
                .with_animation(gpui_kit::ElementId::Name(fresh.into()), motion::ease(motion::BASE), |d, t| d.opacity(t))
        });
        let board = self
            .thumb_frame(&e.id, index)
            .child(self.card_watch(&e.id, cx).absolute().size_full())
            .child(dots(14., 1., dot, |_, _| 1.).absolute().size_full())
            .when_some(picture, |d, picture| d.child(picture))
            // Listening now: a red badge with the time, breathing softly.
            .when(live, |d| {
                d.child(
                    div()
                        .absolute()
                        .left(px(10.))
                        .top(px(10.))
                        .h(px(24.))
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .px(px(9.))
                        .rounded_full()
                        .bg(rgb(p.recording))
                        .text_color(rgb(0xff_ff_ff))
                        .text_size(px(11.5))
                        .font_weight(FontWeight::BOLD)
                        .child(div().size(px(6.)).rounded_full().bg(rgb(0xff_ff_ff)).with_animation("home-live-dot", motion::pulse(), |d, t| d.opacity(t)))
                        .child(format!("{} {}", t("live").to_uppercase(), clock(recorded))),
                )
            })
            .when(pinned, |d| {
                d.child(
                    div()
                        .absolute()
                        .right(px(10.))
                        .top(px(10.))
                        .size(px(28.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_full()
                        .bg(rgb(p.surface))
                        .shadow(vec![ring(rgb(p.line).into(), 1.)])
                        .text_color(rgb(p.muted))
                        .child(icon("ipad/pin").size(px(14.))),
                )
            });
        let speakers = e.speakers.clone();
        let faces = div().flex_none().flex().items_center().children(speakers.iter().take(3).enumerate().map(|(i, name)| {
            who(&speakers, name, 20., true).text_size(px(10.)).when(i > 0, |d| d.ml(px(-5.))).shadow(vec![ring(rgb(p.chrome).into(), 2.)])
        }));
        let title: AnyElement = match (renaming, self.home.rename_input.as_ref()) {
            (true, Some(input)) => div().h(px(20.)).text_size(px(15.)).child(Input::new(input).appearance(false)).into_any_element(),
            _ => div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).overflow_hidden().whitespace_nowrap().text_ellipsis().child(e.title.clone()).into_any_element(),
        };
        let (open_id, press_id, menu_id) = (e.id.clone(), e.id.clone(), e.id.clone());
        let card = div()
            .id(gpui_kit::ElementId::Name(format!("home-card-{}", e.id).into()))
            .flex()
            .flex_col()
            .gap(px(9.))
            .min_w_0()
            .cursor_pointer()
            .child(div().h(px(THUMB_H)).child(board.with_spring(gpui_kit::ElementId::Name(format!("home-press-{}", e.id).into()), SpringAnimation::new(motion::SNAP).to(pressed), |d, v| press(d, v.0))))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(3.))
                    .px(px(4.))
                    .min_w_0()
                    .child(title)
                    .child(div().h(px(20.)).flex().items_center().gap(px(6.)).text_size(px(12.5)).text_color(rgb(p.faint)).child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().child(line)).child(faces)),
            )
            .on_mouse_down(MouseButton::Left, cx.listener(move |this, _, _, cx| {
                this.home.pressed = Some(press_id.clone());
                cx.notify();
            }))
            .on_mouse_down(MouseButton::Right, cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                this.home.menus += 1;
                this.home.menu = Some((menu_id.clone(), event.position));
                cx.notify();
            }))
            .when(!renaming, |d| d.on_click(cx.listener(move |this, _, _, cx| this.home_open(&open_id, cx))));
        stagger(card, opened, order, after)
    }

    /// A card's actions, held out by a long press: pin, rename, export and delete.
    fn home_menu(&self, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (id, at) = self.home.menu.clone()?;
        let p = pal();
        let pinned = self.pinned.iter().any(|x| *x == id);
        let screen = window.viewport_size();
        const W: f32 = 232.;
        const ROW: f32 = 44.;
        let h = ROW * 5. + 9.;
        let x = (f32::from(at.x) - 12.).clamp(12., f32::from(screen.width) - W - 12.);
        let y = (f32::from(at.y) + 8.).min(f32::from(screen.height) - h - 24.).max(12.);
        let row = |key: &'static str, name: &'static str, label: &'static str, danger: bool| {
            div()
                .id(key)
                .h(px(ROW))
                .flex()
                .items_center()
                .justify_between()
                .px(px(16.))
                .text_size(px(15.))
                .cursor_pointer()
                .text_color(rgb(if danger { p.danger } else { p.text }))
                .hover(|d| d.bg(motion::hover_fill(1.)))
                .active(|d| d.bg(rgb(p.press)))
                .child(label)
                .child(icon(&format!("ipad/{name}")).size(px(20.)))
        };
        let (a, b, c, d, e) = (id.clone(), id.clone(), id.clone(), id.clone(), id.clone());
        Some(
            div()
                .absolute()
                .size_full()
                .child(div().id("home-menu-veil").absolute().size_full().on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| {
                    this.home.menu = None;
                    cx.notify();
                })))
                .child(
                    glass(div(), true)
                        .id("home-menu")
                        .occlude()
                        .absolute()
                        .left(px(x))
                        .top(px(y))
                        .w(px(W))
                        .py(px(4.))
                        .rounded(px(14.))
                        .overflow_hidden()
                        .flex()
                        .flex_col()
                        .child(row("home-menu-pin", "pin", if pinned { t("unpin") } else { t("home-pin") }, false).on_click(cx.listener(move |this, _, _, cx| {
                            this.home.menu = None;
                            this.pin_meeting(&a, cx);
                        })))
                        .child(row("home-menu-rename", "pen", t("rename"), false).on_click(cx.listener(move |this, _, window, cx| this.home_start_rename(&b, window, cx))))
                        .child(row("home-menu-export", "export", t("export"), false).on_click(cx.listener(move |this, _, _, cx| this.home_export(&c, cx))))
                        .child(row("home-menu-diag", "info", t("diag-export-menu"), false).on_click(cx.listener(move |this, _, _, cx| this.home_diagnostics(&e, cx))))
                        .child(div().h(px(1.)).mx(px(0.)).my(px(0.5)).bg(rgb(p.line)))
                        .child(row("home-menu-delete", "trash", t("delete"), true).on_click(cx.listener(move |this, _, _, cx| {
                            this.home.menu = None;
                            this.delete_meeting(&d, cx);
                        })))
                        .with_animation(("home-menu-in", self.home.menus), rise(Duration::ZERO), |d, t| d.opacity(t.clamp(0., 1.)).mt(px(-8. * (1. - t)))),
                )
                .into_any_element(),
        )
    }
}

/// A card under a finger gives a little: in by a few points all round, as if pressed into the page.
fn press<E: gpui_kit::Styled>(el: E, v: f32) -> E {
    let v = v.clamp(0., 1.);
    el.mx(px(5. * v)).my(px(3. * v)).h(px(THUMB_H - 6. * v))
}

/// Cards rise into place one after the other each time the page comes up.
fn stagger(card: gpui_kit::Stateful<gpui_kit::Div>, opened: usize, order: usize, after: Duration) -> AnyElement {
    let delay = after + STAGGER * order.min(16) as u32;
    card.with_animation(("home-rise", opened * 10_000 + order), rise(delay), |d, t| d.opacity(t.clamp(0., 1.)).relative().top(px(16. * (1. - t)))).into_any_element()
}

/// How long the splash's name takes to become the title, and its curve.
fn intro_animation() -> (Duration, gpui_kit::Animation) {
    let (settle, easing) = gpui_kit::sampled_easing(ZOOM, 0.002);
    (settle, gpui_kit::Animation::new(settle).with_easing(easing))
}

/// What comes in after the title on a cold start: faded in and risen a little, `after` the page took over.
fn follow(el: gpui_kit::Div, id: &'static str, after: Duration) -> gpui_kit::AnimationElement<gpui_kit::Div> {
    el.with_animation(id, rise(after), |d, t| d.opacity(t.clamp(0., 1.)).mt(px(10. * (1. - t))))
}

/// Once the title has nearly arrived, the rest of the page comes in.
const INTRO_REST: Duration = Duration::from_millis(320);
/// The title's size while it is still the splash's name, and the size the splash writes the name at.
const INTRO_TITLE: f32 = 52.;
const SPLASH_SIZE: f32 = 56.;

impl Workbench {
    /// Where the splash wrote the name: the middle of the window under its bar.
    fn splash_centre(window: &Window) -> (f32, f32) {
        let size = window.viewport_size();
        let inset = window.insets().safe_area;
        let (top, bottom) = (f32::from(inset.top) + BAR_H, f32::from(inset.bottom));
        (f32::from(size.width) / 2., top + (f32::from(size.height) - top - bottom) / 2.)
    }

    /// The page's title on a cold start, travelling from where the splash wrote the name into its place,
    /// shrinking to its size and coming up as the written name fades.
    fn intro_brand(&self, brand: gpui_kit::Div, window: &Window) -> AnyElement {
        if self.home.intro.is_none() {
            return brand.into_any_element();
        }
        let top = f32::from(crate::platform::safe_area(window).top);
        let (cx, cy) = Self::splash_centre(window);
        let (_, animation) = intro_animation();
        // The brand's width at a title size: the mark, the gap and the word, near enough.
        let width = |size: f32| 40. + size * 4.85;
        let home_y = top + 14. + 22.;
        brand
            .relative()
            .with_animation("home-intro-brand", animation, move |d, t| {
                let size = INTRO_TITLE + (34. - INTRO_TITLE) * t;
                let left = (cx - width(size) / 2. - SIDE) * (1. - t);
                let down = (cy - home_y) * (1. - t);
                d.text_size(px(size)).left(px(left)).top(px(down)).opacity(((t - 0.1) / 0.4).clamp(0., 1.))
            })
            .into_any_element()
    }

    /// The splash's written name, fading as the title takes over and drifting the same way, shrinking with it.
    fn intro_word(&self, window: &Window) -> Option<AnyElement> {
        let (word, _) = self.home.intro.as_ref()?;
        let word = word.clone();
        let (cx, cy) = Self::splash_centre(window);
        let top = f32::from(crate::platform::safe_area(window).top);
        let (to_x, to_y) = (SIDE + (40. + 34. * 4.85) / 2., top + 14. + 22.);
        let (_, animation) = intro_animation();
        Some(
            div()
                .absolute()
                .size_full()
                .with_animation("home-intro-word", animation, move |d, t| {
                    let word = word.clone();
                    let k = 1. - (1. - 34. / SPLASH_SIZE) * t;
                    let (x, y) = (cx + (to_x - cx) * t, cy + (to_y - cy) * t);
                    let fade = (1. - t / 0.6).clamp(0., 1.);
                    d.child(
                        canvas(
                            |_, _, _| (),
                            move |_, (), window: &mut Window, _| {
                                if fade <= 0. {
                                    return;
                                }
                                let mut color: Hsla = rgb(pal().ink(0x1e_1e_1e)).into();
                                color.a = fade;
                                let at = |p: &Pt| point(px(x + p[0] * k), px(y + p[1] * k));
                                for stroke in &word {
                                    let shape = outline(stroke, 1.0);
                                    let Some(first) = shape.first() else { continue };
                                    let mut pb = PathBuilder::fill().with_style(PathStyle::Fill(FillOptions::non_zero()));
                                    pb.move_to(at(first));
                                    shape[1..].iter().for_each(|p| pb.line_to(at(p)));
                                    pb.close();
                                    if let Ok(path) = pb.build() {
                                        window.paint_path(path, Background::from(color));
                                    }
                                }
                            },
                        )
                        .absolute()
                        .size_full(),
                    )
                })
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_querys_words_are_marked_where_they_occur() {
        assert_eq!(marked("Ship it on Tuesday, ship!", &["ship".into()]), vec![0..4, 20..24]);
        assert_eq!(marked("物业费下个月起按季度收", &["季度".into()]), vec![24..30]);
        assert!(marked("anything", &[String::new()]).is_empty());
        assert_eq!(marked("abc", &["ab".into(), "b".into()]), vec![0..2], "overlaps keep the first");
    }
}
