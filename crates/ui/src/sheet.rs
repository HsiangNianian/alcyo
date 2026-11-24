//! The iPad's Settings, as a form sheet over the meeting (#341) rather than a page in its place, so the
//! model or the language can change without leaving the board: tabs down the left as on the desktop, but
//! 46 pt rows and grouped inset lists as in iPadOS Settings. It rises from below on a spring while the
//! meeting behind steps back and dims; Done, a tap beside it, a swipe down or Escape puts it away.
//!
//! What each setting does is the workbench's own, shared with the desktop's Settings page; only the layout
//! is here. A child of `app`, so it reaches the workbench's state directly. The first run's introduction
//! (onboarding.rs) is laid over the screen from here too.

use super::*;
use gpui_kit::{AnyElement, BoxShadow, ElementId, FontWeight, Image, ImageFormat, SpringConfig, Stateful};

/// The remote provider an address is for, by its host, however much of the address was typed: `api.deepseek.com`,
/// `https://api.deepseek.com/v1` and the preset's own all name DeepSeek.
fn preset_for(base: &str) -> Option<&'static (&'static str, &'static str, &'static str)> {
    let host = |url: &str| -> String {
        let url = url.trim();
        let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
        rest.split(['/', '?', '#']).next().unwrap_or_default().to_ascii_lowercase()
    };
    let wanted = host(base);
    (!wanted.is_empty()).then(|| REMOTE_PRESETS.iter().find(|(_, b, _)| host(b) == wanted)).flatten()
}

/// The sheet's size, when the window has room for it, and its parts.
const SHEET_W: f32 = 800.0;
const SHEET_H: f32 = 732.0;
const NAV_W: f32 = 232.0;
const HEADER_H: f32 = 64.0;
const TAB_H: f32 = 46.0;
/// The sheet rising and sinking: critically damped, so it settles in about a third of a second without
/// overshooting.
const RISE: SpringConfig = SpringConfig::new(260., 32.25, 1.);
/// How long the sheet is still drawn after it was put away, for it to sink out of sight.
const SINK_FOR: Duration = Duration::from_millis(450);
/// How far the sheet must be pulled down to be put away.
const DISMISS_DRAG: f32 = 110.0;

/// What the sheet keeps for itself, beside the workbench's settings.
pub(super) struct Sheet {
    /// Settings was open at the last frame, and when it was put away, while it sinks.
    open: bool,
    sinking: Option<Instant>,
    /// It was opened over the meetings page, which stays under it.
    under_library: bool,
    /// The sheet is held to be pulled down: where the pointer took hold, and how far down it is now.
    drag: Option<(f32, f32)>,
    /// The glossary's own page is open over the Speech tab.
    glossary: bool,
    /// The last check of the remote model, for which settings, and one under way.
    probe: Option<(String, Result<u128, String>)>,
    probing: Option<String>,
    /// The screen is being kept on, as last asked of the system.
    awake: bool,
    /// A new API key is being typed, in place of the saved one's ends.
    typing_key: bool,
}

impl Sheet {
    pub(super) fn new() -> Self {
        Self { open: false, sinking: None, under_library: false, drag: None, glossary: false, probe: None, probing: None, awake: false, typing_key: false }
    }
}

/// A colour with some see-through.
fn tint(color: u32, a: f32) -> Hsla {
    let mut c: Hsla = rgb(color).into();
    c.a = a;
    c
}

/// The accent's soft fill, behind something chosen.
fn accent_soft() -> Hsla {
    rgb(if theme::is_dark() { 0x16_1c_33 } else { 0xee_f1_fd }).into()
}

/// What is written on the accent.
fn on_accent() -> u32 {
    if theme::is_dark() { 0x0b_10_20 } else { 0xff_ff_ff }
}

/// The system's grey fill for a small button or a segmented control.
fn fill(strength: f32) -> Hsla {
    tint(0x76_76_80, if theme::is_dark() { 0.24 } else { 0.12 } * strength)
}

fn ring(color: Hsla, width: f32) -> BoxShadow {
    BoxShadow { color, offset: point(px(0.), px(0.)), blur_radius: px(0.), spread_radius: px(width), inset: false }
}

fn shade(a: f32, y: f32, blur: f32) -> BoxShadow {
    BoxShadow { color: tint(0, a), offset: point(px(0.), px(y)), blur_radius: px(blur), spread_radius: px(0.), inset: false }
}

fn name(prefix: &str, part: impl std::fmt::Display) -> ElementId {
    ElementId::Name(format!("{prefix}-{part}").into())
}

/// A group's name over its rows, in small capitals, with something said at its right.
fn label(text: &str, aside: Option<AnyElement>) -> gpui_kit::Div {
    div()
        .flex()
        .items_center()
        .justify_between()
        .px(px(4.))
        .font_family(theme::MONO)
        .text_size(px(10.5))
        .text_color(rgb(pal().faint))
        .child(text.to_uppercase())
        .when_some(aside, |d, aside| d.child(aside))
}

/// Rows that go together, inset on the chrome's grey, with a hairline between them that starts at the text.
fn group(rows: Vec<AnyElement>) -> gpui_kit::Div {
    div().flex().flex_col().rounded(px(16.)).bg(rgb(pal().chrome)).overflow_hidden().children(rows.into_iter().enumerate().map(|(i, row)| {
        div().relative().when(i > 0, |d| d.child(div().absolute().top_0().left(px(16.)).right_0().h(px(1.)).bg(rgb(pal().line)))).child(row)
    }))
}

/// A row: at least 50 pt, its parts side by side.
fn row() -> gpui_kit::Div {
    div().min_h(px(50.)).flex().items_center().gap(px(12.)).px(px(16.)).text_size(px(15.))
}

/// A row's name in a column of its own, `w` wide.
fn key(text: &str, w: f32) -> gpui_kit::Div {
    div().w(px(w)).flex_none().text_color(rgb(pal().text)).child(text.to_string())
}

/// A setting's name with a line under it on what it does.
fn titled(title: &str, hint: Option<String>) -> gpui_kit::Div {
    div()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .gap(px(2.))
        .py(px(10.))
        .child(div().font_weight(FontWeight::MEDIUM).child(title.to_string()))
        .when_some(hint, |d, hint| d.child(div().text_size(px(12.5)).text_color(rgb(pal().faint)).line_height(gpui_kit::relative(1.3)).child(hint)))
}

fn chevron(name: &str) -> gpui_kit::Div {
    div().flex_none().text_color(rgb(if theme::is_dark() { 0x5a_5a_5e } else { 0xb4_b4_b8 })).child(icon(&format!("ipad/{name}")).size(px(16.)))
}

/// The system's switch, its knob gliding across and its green filling in.
fn switch(id: &str, on: bool) -> gpui_kit::Div {
    let green = if theme::is_dark() { 0x30_d1_58 } else { 0x34_c7_59 };
    let off = tint(0x78_78_80, if theme::is_dark() { 0.32 } else { 0.16 });
    let key = id;
    div()
        .relative()
        .flex_none()
        .w(px(51.))
        .h(px(31.))
        .rounded_full()
        .bg(off)
        .child(div().absolute().size_full().rounded_full().bg(rgb(green)).with_spring(name(key, "fill"), SpringAnimation::new(motion::GLIDE).to(on), |d, v| d.opacity(v.0.clamp(0., 1.))))
        .child(
            div()
                .absolute()
                .top(px(2.))
                .size(px(27.))
                .rounded_full()
                .bg(rgb(0xff_ff_ff))
                .shadow(vec![shade(0.18, 2., 6.), ring(tint(0, 0.04), 0.5)])
                .with_spring(name(key, "knob"), SpringAnimation::new(motion::GLIDE).to(on), |d, v| d.left(px(2. + 20. * v.0))),
        )
}

/// A radio: a ring, or the accent with a check when chosen.
fn radio(on: bool, size: f32) -> gpui_kit::Div {
    div()
        .flex_none()
        .size(px(size))
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .map(|d| if on { d.bg(rgb(pal().accent)).text_color(rgb(on_accent())).child(icon("ipad/check").size(px(size * 0.6))) } else { d.border(px(1.5)).border_color(rgb(if theme::is_dark() { 0x48_48_4a } else { 0xc7_c7_cc })) })
}

/// A small round button, grey or in the accent.
fn pill_button(id: impl Into<ElementId>, text: &str, accent: bool, glyph: Option<&str>) -> Stateful<gpui_kit::Div> {
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
        .font_weight(FontWeight::SEMIBOLD)
        .whitespace_nowrap()
        .cursor_pointer()
        .map(|d| if accent { d.bg(accent_soft()).text_color(rgb(pal().accent)) } else { d.bg(fill(1.)).text_color(rgb(pal().text)) })
        .active(|d| d.opacity(0.7))
        .when_some(glyph, |d, glyph| d.child(icon(&format!("ipad/{glyph}")).size(px(15.))))
        .child(text.to_string())
}

/// The system's segmented control: the chosen part raised, gliding to another when chosen.
fn segmented(id: &'static str, w: f32, parts: Vec<(String, bool, Box<dyn Fn(&mut Workbench, &mut Window, &mut Context<Workbench>)>)>, cx: &mut Context<Workbench>) -> gpui_kit::Div {
    let n = parts.len().max(1);
    let chosen = parts.iter().position(|(_, on, _)| *on).unwrap_or(0);
    let part_w = (w - 4.0 - 2.0 * (n as f32 - 1.0)) / n as f32;
    let dark = theme::is_dark();
    let raised = div()
        .absolute()
        .top(px(2.))
        .h(px(30.))
        .w(px(part_w))
        .rounded(px(7.))
        .map(|d| if dark { d.bg(rgb(0x5a_5a_5e)) } else { d.bg(rgb(pal().surface)).shadow(vec![shade(0.12, 1., 3.), ring(tint(0, 0.04), 0.5)]) })
        .with_spring(name(id, "raised"), SpringAnimation::new(motion::GLIDE).to(2.0 + chosen as f32 * (part_w + 2.0)), |d, x| d.left(px(x)));
    div()
        .relative()
        .flex_none()
        .w(px(w))
        .h(px(34.))
        .flex()
        .p(px(2.))
        .gap(px(2.))
        .rounded(px(9.))
        .bg(fill(1.))
        .child(raised)
        .children(parts.into_iter().enumerate().map(|(i, (text, _, choose))| {
            div()
                .id(name(id, i))
                .relative()
                .flex_1()
                .h(px(30.))
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(13.))
                .font_weight(FontWeight::MEDIUM)
                .whitespace_nowrap()
                .cursor_pointer()
                .child(text)
                .on_click(cx.listener(move |this, _, window, cx| choose(this, window, cx)))
        }))
}

/// A choice shown as its value and a chevron, which opens the choices as a menu.
fn menu_row<T: Clone + PartialEq + 'static>(id: &'static str, shown: String, choices: Vec<(T, String)>, chosen: T, choose: fn(&mut Workbench, T, &mut Window, &mut Context<Workbench>), cx: &mut Context<Workbench>) -> impl IntoElement {
    let workbench = cx.entity().downgrade();
    Button::new(id)
        .ghost()
        .px(px(6.))
        .mx(px(-6.))
        .child(div().text_size(px(15.)).text_color(rgb(pal().muted)).whitespace_nowrap().child(shown))
        .dropdown_menu(move |menu, _, _| {
            choices.iter().cloned().fold(menu.min_w(px(200.)).max_h(px(420.)).scrollable(true), |menu, (value, text)| {
                let workbench = workbench.clone();
                let on = value == chosen;
                menu.item(PopupMenuItem::new(text).checked(on).on_click(move |_, window, cx| {
                    let value = value.clone();
                    let _ = workbench.update(cx, |this, cx| choose(this, value, window, cx));
                }))
            })
        })
}

/// The kingfisher, in its colours, `w` wide: the app's mark.
pub(super) fn kingfisher(w: f32) -> impl IntoElement {
    thread_local! {
        static MARKS: RefCell<HashMap<(bool, u32), Arc<Image>>> = RefCell::new(HashMap::new());
    }
    let dark = theme::is_dark();
    let h = w * 380.0 / 480.0;
    // Drawn at three times its size, so it stays sharp on a dense screen: GPUI rasterizes an SVG once, at 1x.
    let at = (w * 3.0).round() as u32;
    let mark = MARKS.with(|marks| {
        marks
            .borrow_mut()
            .entry((dark, at))
            .or_insert_with(|| {
                let cut = if dark { "#0e0e0e" } else { "#f3f3f1" };
                let svg = format!(
                    r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="20 100 480 380" width="{at}" height="{}"><path d="M48 430 C120 392 168 300 214 236 C240 182 280 134 334 132 C370 131 394 150 398 172 L480 194 L390 204 C374 226 348 244 316 262 C262 292 214 330 168 372 C134 402 102 426 70 448 L84 429 Z" fill="#0a7ea4"/><path d="M372 236 C350 274 300 318 246 352 C214 372 182 390 148 402 C196 404 252 386 298 352 C336 324 362 284 372 236 Z" fill="#f07a2b"/><path d="M318 176 C330 167 352 163 370 168 C354 175 336 179 318 176 Z" fill="{cut}"/><path d="M150 372 C190 318 232 262 290 222 C246 270 206 320 168 370 Z" fill="{cut}"/></svg>"##,
                    (at as f32 * 380.0 / 480.0).round() as u32
                );
                Arc::new(Image::from_bytes(ImageFormat::Svg, svg.into_bytes()))
            })
            .clone()
    });
    gpui_kit::img(mark).w(px(w)).h(px(h)).flex_none()
}

/// What can be chosen for the Pencil's double tap and squeeze, as saved and as said.
fn pencil_actions() -> Vec<(String, String)> {
    alcyo_agent::settings::PENCIL_ACTIONS
        .iter()
        .map(|a| {
            let said = match *a {
                "undo" => t("ipad-pencil-undo"),
                "eraser" => t("ipad-pencil-eraser"),
                "tools" => t("ipad-pencil-tools"),
                _ => t("ipad-pencil-off"),
            };
            (a.to_string(), said.to_string())
        })
        .collect()
}

/// The languages speech can be heard in, as saved and as said.
fn speech_languages(system: bool) -> Vec<(String, String)> {
    let auto = if system { t("automatic") } else { t("automatic-experimental") };
    [("auto", auto), ("zh-CN", "中文"), ("en-US", "English (US)"), ("ja-JP", "日本語")].into_iter().map(|(code, said)| (code.to_string(), said.to_string())).collect()
}

/// Ask the system to keep the screen on, or to let it lock again by itself.
#[cfg(target_os = "ios")]
fn keep_screen_on(on: bool) {
    use objc2::msg_send;
    use objc2::runtime::{AnyClass, AnyObject, Bool};
    let Some(class) = AnyClass::get(c"UIApplication") else { return };
    // SAFETY: UIApplication's shared instance and its idleTimerDisabled property, on the main thread, where
    // the window draws.
    unsafe {
        let app: *mut AnyObject = msg_send![class, sharedApplication];
        if !app.is_null() {
            let _: () = msg_send![app, setIdleTimerDisabled: Bool::new(on)];
        }
    }
}

/// Elsewhere the screen is the system's to keep on.
#[cfg(not(target_os = "ios"))]
fn keep_screen_on(_on: bool) {}

impl Workbench {
    /// The iPad's whole screen: the meeting, and over it the Settings sheet or the first run's introduction.
    pub(super) fn ipad_layers(&mut self, parts: ipad::Parts, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let open = self.panel == Panel::Settings;
        // An address and model never typed start as the first provider's, written in rather than only hinted, so
        // the provider row names the one the fields are for and a key is all that is left to give.
        if open && !self.sheet.open {
            let empty = |input: &Entity<InputState>, cx: &App| input.read(cx).value().trim().is_empty();
            if empty(&self.form.base_url, cx) && empty(&self.form.model, cx) {
                let (_, base, model) = REMOTE_PRESETS[0];
                self.form.base_url.update(cx, |s, cx| s.set_value(base, window, cx));
                self.form.model.update(cx, |s, cx| s.set_value(model, window, cx));
            }
        }
        if open {
            self.sheet.sinking = None;
        } else if self.sheet.open {
            self.sheet.sinking = Some(Instant::now());
            self.sheet.drag = None;
            self.sheet.glossary = false;
            self.sheet.typing_key = false;
        }
        self.sheet.open = open;
        if self.sheet.sinking.is_some_and(|at| at.elapsed() >= SINK_FOR) {
            self.sheet.sinking = None;
        }
        // While listening, the screen stays on if asked to.
        let awake = self.pipeline.listening() && self.form.saved.keep_screen_on;
        if awake != self.sheet.awake {
            self.sheet.awake = awake;
            keep_screen_on(awake);
        }
        // Under the sheet the meeting stays as it was, its chrome and all.
        if open {
            self.panel = Panel::Transcript;
        }
        // Under the sheet: the meetings page it was opened from, or the meeting.
        if !open && self.sheet.sinking.is_none() {
            self.sheet.under_library = self.panel == Panel::Library;
        }
        let screen = if self.sheet.under_library && (open || self.sheet.sinking.is_some()) || self.panel == Panel::Library {
            if open {
                self.panel = Panel::Library;
            }
            let home = self.home_screen(parts, window, cx);
            if open {
                self.panel = Panel::Transcript;
            }
            home
        } else {
            let screen = self.ipad_screen(parts, window, cx);
            self.home_zoom_over(screen, window, cx)
        };
        if open {
            self.panel = Panel::Settings;
        }
        let shown = open || self.sheet.sinking.is_some();
        let intro = self.onboarding(window, cx);
        if !shown && intro.is_none() {
            return screen;
        }
        if self.sheet.sinking.is_some() {
            window.request_animation_frame();
        }
        div()
            .relative()
            .size_full()
            // Escape in one of the sheet's fields, which is outside the meeting's own keys, puts it away too.
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" && this.panel == Panel::Settings && !this.focus.is_focused(window) {
                    this.close_sheet(window, cx);
                }
            }))
            .child(screen)
            .when(shown, |d| d.child(self.sheet_layer(open, window, cx)))
            .when_some(intro, |d, intro| d.child(intro))
            .into_any_element()
    }

    /// Put the sheet away, keeping what was typed for the remote model as iPadOS settings keep what is typed.
    fn close_sheet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let typed = self.form_settings(cx);
        let saved = &self.form.saved;
        let changed = typed.base_url != saved.base_url || typed.model != saved.model || typed.api_key != saved.api_key;
        if self.form.remote && changed && typed.is_complete() {
            self.save_settings(window, cx);
        }
        self.panel = Panel::Transcript;
        window.focus(&self.focus, cx);
        cx.notify();
    }

    /// Check the saved remote model answers, once for each change to it, to say so over its fields.
    fn probe_remote(&mut self, cx: &mut Context<Self>) {
        let saved = &self.form.saved;
        if !(saved.enabled && saved.is_complete()) {
            return;
        }
        let key = format!("{}|{}|{}", saved.base_url, saved.model, saved.key_hint().unwrap_or_default());
        if self.sheet.probing.as_ref() == Some(&key) || self.sheet.probe.as_ref().is_some_and(|(k, _)| *k == key) {
            return;
        }
        let provider = Provider::Remote { base_url: saved.base_url.clone(), key: saved.api_key.clone(), model: saved.model.clone() };
        self.sheet.probing = Some(key.clone());
        let answer = cx.background_executor().spawn(async move {
            let hello = Utterance { speaker: None, text: "Hello".into(), pointing: vec![] };
            propose(&provider, &Graph::default(), &[], &hello, Guidance::default(), None).map(|r| r.ms).map_err(|e| format!("{e:#}"))
        });
        cx.spawn(async move |this, cx| {
            let answer = answer.await;
            let _ = this.update(cx, |this, cx| {
                if this.sheet.probing.as_ref() == Some(&key) {
                    this.sheet.probing = None;
                    this.sheet.probe = Some((key, answer));
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// The sheet, the scrim under it, and the meeting stepping back behind both.
    fn sheet_layer(&mut self, open: bool, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let size = window.viewport_size();
        let (w, h) = (f32::from(size.width), f32::from(size.height));
        let safe = crate::platform::safe_area(window);
        let (safe_top, safe_bottom) = (f32::from(safe.top), f32::from(safe.bottom));
        // As big as the design has it, or as the window allows: an 11" iPad upright, or a smaller window.
        let sheet_w = SHEET_W.min(w - 32.0);
        let sheet_h = SHEET_H.min(h - safe_top - safe_bottom - 24.0).max(320.0);
        let top = safe_top + ((h - safe_top - safe_bottom - sheet_h) / 2.0).max(12.0);
        let left = ((w - sheet_w) / 2.0).round();
        let drag = self.sheet.drag.map_or(0.0, |(_, dy)| dy);
        let travel = h - top + 40.0;
        let dark = theme::is_dark();

        // The meeting behind dims. It does not step back as UIKit's would: GPUI cannot scale what it draws, and a
        // frame closing in over it cut off what was at its edges.
        let behind = div()
            .id("ipad-sheet-scrim")
            .absolute()
            .inset_0()
            .bg(tint(0x0a_0a_0a, if dark { 0.5 } else { 0.3 }))
            .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| this.close_sheet(window, cx)))
            .with_spring("ipad-sheet-scrim-fade", SpringAnimation::new(RISE).to(open).from(false), |d, v| d.opacity(v.0.clamp(0., 1.)));

        let sheet = div()
            .id("ipad-sheet")
            .occlude()
            .absolute()
            .left(px(left))
            .w(px(sheet_w))
            .h(px(sheet_h))
            .flex()
            .rounded(px(30.))
            .overflow_hidden()
            .bg(rgb(pal().surface))
            .shadow(vec![shade(if dark { 0.6 } else { 0.28 }, 30., 80.), ring(if dark { tint(0xff_ff_ff, 0.1) } else { tint(0, 0.08) }, 0.5)])
            .child(self.sheet_nav(sheet_h, cx))
            .child(self.sheet_body(cx))
            .with_spring("ipad-sheet-rise", SpringAnimation::new(RISE).to(open).from(false), move |d, v| d.top(px(top + drag + (1.0 - v.0) * travel)));

        div()
            .absolute()
            .top_0()
            .left_0()
            .w(px(w))
            .h(px(h))
            .child(behind)
            .child(sheet)
            .child(self.sheet_pull(cx))
            .into_any_element()
    }

    /// Pulled down by its top or its tabs, the sheet follows the finger, and goes once it is far enough. The
    /// window hears the pull wherever the finger goes, over the sheet or off it, and however quickly it ends.
    fn sheet_pull(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.entity().downgrade();
        canvas(
            |_, _, _| (),
            move |_, (), window: &mut Window, _| {
                let moved = this.clone();
                window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                    if phase != gpui_kit::DispatchPhase::Capture {
                        return;
                    }
                    let _ = moved.update(cx, |this, cx| {
                        if let Some((from, dy)) = this.sheet.drag.as_mut() {
                            // Upward it gives only a little, as a sheet does.
                            let pulled = f32::from(event.position.y) - *from;
                            *dy = if pulled >= 0.0 { pulled } else { -(-pulled).sqrt() * 2.0 };
                            cx.notify();
                        }
                    });
                });
                let released = this.clone();
                window.on_mouse_event(move |_: &MouseUpEvent, phase, window, cx| {
                    if phase != gpui_kit::DispatchPhase::Capture {
                        return;
                    }
                    let _ = released.update(cx, |this, cx| {
                        if let Some((_, dy)) = this.sheet.drag.take() {
                            if dy > DISMISS_DRAG {
                                this.close_sheet(window, cx);
                            }
                            cx.notify();
                        }
                    });
                });
            },
        )
        .absolute()
        .size(px(0.))
    }

    /// Where the sheet can be taken hold of to pull it down.
    fn grip<E: InteractiveElement>(&self, el: E, cx: &mut Context<Self>) -> E {
        el.on_mouse_down(MouseButton::Left, cx.listener(|this, event: &MouseDownEvent, _, cx| {
            this.sheet.drag = Some((f32::from(event.position.y), 0.0));
            cx.notify();
        }))
    }

    /// The tabs down the left, under the sheet's name, with the version at the foot.
    fn sheet_nav(&self, sheet_h: f32, cx: &mut Context<Self>) -> AnyElement {
        let tabs: [(SettingsTab, &str, u32, &str); 5] = [
            (SettingsTab::Model, "sparkle", 0x3b_5b_db, t("board-model")),
            (SettingsTab::Speech, "waveform", 0xe8_43_2a, t("speech")),
            (SettingsTab::General, "sliders", 0x63_63_66, t("general")),
            (SettingsTab::Shortcuts, "keyboard", 0x8e_8e_93, t("shortcuts")),
            (SettingsTab::About, "info", 0x0a_7e_a4, t("about")),
        ];
        let chosen = tabs.iter().position(|(tab, ..)| *tab == self.settings_tab).unwrap_or(0);
        let dark = theme::is_dark();
        // The chosen tab's raised fill glides to another when it is chosen.
        let raised = div()
            .absolute()
            .left_0()
            .right_0()
            .h(px(TAB_H))
            .rounded(px(12.))
            .bg(rgb(if dark { 0x1c_1c_1e } else { pal().surface }))
            .when(!dark, |d| d.shadow(vec![shade(0.07, 1., 3.), ring(tint(0, 0.05), 0.5)]))
            .with_spring("ipad-sheet-tab", SpringAnimation::new(motion::GLIDE).to(chosen as f32 * (TAB_H + 2.0)), |d, y| d.top(px(y)));
        let list = div().relative().flex().flex_col().gap(px(2.)).child(raised).children(tabs.into_iter().map(|(tab, glyph, color, text)| {
            div()
                .id(name("ipad-sheet-tab", text))
                .relative()
                .h(px(TAB_H))
                .flex()
                .items_center()
                .gap(px(12.))
                .px(px(12.))
                .rounded(px(12.))
                .text_size(px(15.5))
                .font_weight(FontWeight::MEDIUM)
                .cursor_pointer()
                .child(div().flex_none().size(px(30.)).flex().items_center().justify_center().rounded(px(8.)).bg(rgb(color)).text_color(rgb(0xff_ff_ff)).child(icon(&format!("ipad/{glyph}")).size(px(18.))))
                .child(text)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.settings_tab = tab;
                    this.sheet.glossary = false;
                    cx.notify();
                }))
        }));
        let nav = div()
            .id("ipad-sheet-nav")
            .flex_none()
            .w(px(NAV_W))
            .h(px(sheet_h))
            .flex()
            .flex_col()
            .pt(px(20.))
            .px(px(12.))
            .pb(px(20.))
            .bg(rgb(pal().chrome))
            // The sheet's corners do not clip what is in it, so the tabs' fill is rounded with them.
            .rounded_l(px(30.))
            .child(div().mt(px(4.)).mx(px(10.)).mb(px(14.)).text_size(px(24.)).font_weight(FontWeight::BOLD).child(t("settings")))
            .child(list)
            .child(div().flex_1())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .px(px(12.))
                    .text_size(px(12.))
                    .text_color(rgb(pal().faint))
                    .child(kingfisher(22.))
                    .child(ta!("ipad-version", version = alcyo_update::VERSION)),
            );
        self.grip(nav, cx).into_any_element()
    }

    /// The chosen tab: its name and Done over its settings.
    fn sheet_body(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let tab = self.settings_tab;
        let glossary = self.sheet.glossary && tab == SettingsTab::Speech;
        let title = match tab {
            _ if glossary => t("ipad-glossary"),
            SettingsTab::Model => t("board-model"),
            SettingsTab::Speech => t("speech"),
            SettingsTab::General => t("general"),
            SettingsTab::Shortcuts => t("shortcuts"),
            SettingsTab::About => t("about"),
        };
        let header = div()
            .id("ipad-sheet-header")
            .flex_none()
            .h(px(HEADER_H))
            .flex()
            .items_center()
            .gap(px(4.))
            .pl(px(if glossary { 14. } else { 28. }))
            .pr(px(16.))
            .border_b_1()
            .border_color(rgb(pal().line))
            .when(glossary, |d| {
                d.child(
                    div()
                        .id("ipad-sheet-back")
                        .flex()
                        .items_center()
                        .h(px(44.))
                        .pr(px(6.))
                        .text_color(rgb(pal().accent))
                        .text_size(px(15.))
                        .cursor_pointer()
                        .child(icon("ipad/chevron-left").size(px(22.)))
                        .child(t("speech"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.sheet.glossary = false;
                            cx.notify();
                        })),
                )
                .child(div().w(px(10.)))
            })
            .child(div().text_size(px(19.)).font_weight(FontWeight::BOLD).child(title))
            .child(div().flex_1())
            .child(
                div()
                    .id("ipad-sheet-done")
                    .h(px(40.))
                    .flex()
                    .items_center()
                    .px(px(18.))
                    .rounded_full()
                    .bg(rgb(pal().accent))
                    .text_color(rgb(on_accent()))
                    .text_size(px(15.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .cursor_pointer()
                    .active(|d| d.opacity(0.8))
                    .child(t("done"))
                    .on_click(cx.listener(|this, _, window, cx| this.close_sheet(window, cx))),
            );
        let header = self.grip(header, cx);
        let content: Vec<AnyElement> = match tab {
            _ if glossary => self.glossary_card(cx),
            SettingsTab::Model => self.sheet_model(cx),
            SettingsTab::Speech => self.sheet_speech(cx),
            SettingsTab::General => self.sheet_general(cx),
            SettingsTab::Shortcuts => self.sheet_shortcuts(cx),
            SettingsTab::About => self.sheet_about(cx),
        };
        let page = if glossary { "glossary".to_string() } else { format!("{}", tab as u8) };
        // Another tab's settings slide in a little from below and fade in, a tab at a time.
        let body = div()
            .id(name("ipad-sheet-page", &page))
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(18.))
                    .pt(px(20.))
                    .px(px(28.))
                    .pb(px(24.))
                    .when(glossary, |d| d.gap(px(8.)).text_size(px(13.)))
                    .children(content)
                    .with_spring(name("ipad-sheet-in", &page), SpringAnimation::new(motion::SETTLE).to(true).from(false), |d, v| {
                        let v = v.0.clamp(0., 1.);
                        d.opacity(v).mt(px(10. * (1. - v)))
                    }),
            );
        div().flex_1().min_w_0().flex().flex_col().child(header).child(body).into_any_element()
    }

    /// Board model: where the board is drawn, remote or on this iPad, with what each needs under it.
    fn sheet_model(&mut self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        self.probe_remote(cx);
        let p = pal();
        // Only where the model can run: on the iPad, one with memory enough for it (platform::local_models).
        let local_possible = crate::platform::local_models();
        let remote = self.form.remote || !local_possible;
        let option = |id: &'static str, glyph: &str, title: &str, text: &str, on: bool| {
            div()
                .id(id)
                .relative()
                .flex_1().min_w_0()
                .flex()
                .flex_col()
                .gap(px(6.))
                .p(px(16.))
                .rounded(px(18.))
                .cursor_pointer()
                // A ring as a shadow, so choosing does not move what is inside; GPUI fills a shadow, so the card is
                // filled over it.
                .map(|d| if on { d.bg(accent_soft()).shadow(vec![ring(rgb(p.accent).into(), 2.)]) } else { d.bg(rgb(p.surface)).shadow(vec![ring(rgb(p.line).into(), 1.)]) })
                .child(div().absolute().top(px(14.)).right(px(14.)).child(radio(on, 22.)))
                .child(div().flex().items_center().gap(px(8.)).pr(px(28.)).text_size(px(16.)).font_weight(FontWeight::SEMIBOLD).child(icon(&format!("ipad/{glyph}")).size(px(20.))).child(title.to_string()))
                .child(div().text_size(px(13.)).line_height(gpui_kit::relative(1.45)).text_color(rgb(p.muted)).child(text.to_string()))
        };
        let options = div()
            .flex()
            .gap(px(12.))
            .child(option("ipad-use-remote", "cloud", t("remote"), t("ipad-remote-card"), remote).on_click(cx.listener(|this, _, window, cx| {
                this.form.remote = true;
                if !this.form.saved.enabled && this.form_settings(cx).is_complete() {
                    this.save_settings(window, cx);
                }
                cx.notify();
            })))
            .child(if local_possible {
                option("ipad-use-local", "chip", t("ipad-on-this-ipad"), t("ipad-local-card"), !remote).on_click(cx.listener(|this, _, window, cx| {
                    this.form.remote = false;
                    // A model already here is used at once; otherwise one is downloaded below.
                    let here = alcyo_models::current(&this.models).and_then(|path| alcyo_models::catalog().into_iter().find(|m| path.ends_with(&m.file)));
                    match here {
                        Some(model) => this.use_model(&model, cx),
                        None if this.form.saved.enabled => this.save_settings(window, cx),
                        None => {}
                    }
                    cx.notify();
                }))
            } else {
                // Shown, but this iPad has too little memory for the model.
                option("ipad-use-local", "chip", t("ipad-on-this-ipad"), t("ipad-local-card"), false).cursor_default().opacity(0.55)
            });
        let mut out = vec![
            div().text_size(px(14.)).line_height(gpui_kit::relative(1.5)).text_color(rgb(p.muted)).child(t("ipad-model-intro")).into_any_element(),
            options.into_any_element(),
        ];

        // Remote: the provider and its address, model and key, and whether it answers.
        let saved = &self.form.saved;
        let probe_key = format!("{}|{}|{}", saved.base_url, saved.model, saved.key_hint().unwrap_or_default());
        let checked: Option<AnyElement> = if !(saved.enabled && saved.is_complete()) {
            None
        } else if self.sheet.probing.as_ref() == Some(&probe_key) {
            Some(div().text_color(rgb(p.faint)).child(t("ipad-checking")).with_animation("ipad-probing", motion::pulse(), |d, v| d.opacity(v)).into_any_element())
        } else {
            match self.sheet.probe.as_ref().filter(|(k, _)| *k == probe_key) {
                Some((_, Ok(ms))) => Some(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(if theme::is_dark() { 0x30_d1_58 } else { 0x24_8a_3d }))
                        .child(icon("ipad/check").size(px(14.)))
                        .child(ta!("ipad-connected", ms = *ms as u64).to_uppercase())
                        .into_any_element(),
                ),
                Some((_, Err(error))) => Some(
                    div()
                        .id("ipad-probe-failed")
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(p.danger))
                        .child(t("ipad-not-connected").to_uppercase())
                        .tooltip({
                            let error = error.clone();
                            move |window, cx| Tooltip::new(error.clone()).build(window, cx)
                        })
                        .into_any_element(),
                ),
                None => None,
            }
        };
        let base = self.form.base_url.read(cx).value().to_string();
        let provider = preset_for(&base).map(|(name, ..)| name.to_string());
        let mut providers: Vec<(String, String)> = REMOTE_PRESETS.iter().map(|(name, ..)| (name.to_string(), name.to_string())).collect();
        providers.push((String::new(), t("ipad-other-provider").to_string()));
        let provider_row = row().child(key(t("ipad-provider"), 96.)).child(menu_row(
            "ipad-provider-menu",
            provider.clone().unwrap_or_else(|| t("ipad-other-provider").to_string()),
            providers,
            provider.unwrap_or_default(),
            |this, choice, window, cx| {
                if let Some((_, base, model)) = REMOTE_PRESETS.iter().find(|(name, ..)| *name == choice) {
                    this.form.base_url.update(cx, |s, cx| s.set_value(*base, window, cx));
                    this.form.model.update(cx, |s, cx| s.set_value(*model, window, cx));
                    this.form.key.update(cx, |s, cx| s.focus(window, cx));
                } else {
                    this.form.base_url.update(cx, |s, cx| {
                        s.set_value("", window, cx);
                        s.focus(window, cx);
                    });
                }
                cx.notify();
            },
            cx,
        ))
        .child(div().flex_1())
        .child(chevron("chevron-right"));
        // The field's own inset taken back, so its text lines up with the values above and below it.
        let field = |input: &Entity<InputState>| div().flex_1().min_w_0().ml(px(-10.)).font_family(theme::MONO).text_size(px(13.5)).text_color(rgb(p.muted)).child(Input::new(input).appearance(false));
        let key_empty = self.form.key.read(cx).value().is_empty();
        let hint = self.form.saved.key_hint().map(|hint| hint.replace('…', "••••••••••••••••"));
        let kept = if alcyo_agent::settings::Settings::secrets_in_file() { t("ipad-key-file") } else { t("ipad-keychain") };
        let key_row = row()
            .child(key(t("api-key"), 96.))
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_w_0()
                    .font_family(theme::MONO)
                    .text_size(px(13.5))
                    .text_color(rgb(p.muted))
                    // The saved key's ends in place of the empty field, until it is tapped to type a new one.
                    .map(|d| match hint.filter(|_| key_empty && !self.sheet.typing_key) {
                        Some(hint) => d.child(div().id("ipad-key-hint").cursor_text().child(hint).on_click(cx.listener(|this, _, window, cx| {
                            this.sheet.typing_key = true;
                            this.form.key.update(cx, |s, cx| s.focus(window, cx));
                            cx.notify();
                        }))),
                        None => d.child(Input::new(&self.form.key).appearance(false)),
                    }),
            )
            .when(self.form.saved.key_hint().is_some(), |d| d.child(div().flex_none().flex().items_center().gap(px(4.)).font_family(theme::MONO).text_size(px(11.)).text_color(rgb(p.faint)).child(icon("ipad/lock").size(px(12.))).child(kept)));
        out.push(
            div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(label(t("remote"), checked.map(|c| div().text_size(px(11.)).child(c).into_any_element())))
                .child(group(vec![
                    provider_row.into_any_element(),
                    row().child(key(t("ipad-endpoint"), 96.)).child(field(&self.form.base_url)).into_any_element(),
                    row().child(key(t("model"), 96.)).child(field(&self.form.model)).into_any_element(),
                    key_row.into_any_element(),
                ]))
                .into_any_element(),
        );

        // On this iPad: the models that fit it, each to download or to use. Not where they cannot run.
        if !local_possible {
            if let Some(note) = self.form.note.clone().filter(|_| self.sheet.probing.is_none()) {
                out.push(div().px(px(4.)).mt(px(-8.)).text_size(px(12.5)).text_color(rgb(p.faint)).child(note).into_any_element());
            }
            return out;
        }
        let free = {
            let mut at = self.models.clone();
            while !at.exists() && at.pop() {}
            alcyo_models::free_space(&at).ok()
        };
        let current = alcyo_models::current(&self.models);
        let using_remote = self.form.saved.enabled && self.form.saved.is_complete();
        let recommended = alcyo_models::recommended(0);
        let busy = self.download.as_ref().map(|d| (d.id, d.progress.as_ref().map_or(0.0, |p| p.done as f32 / p.total.max(1) as f32)));
        let models = alcyo_models::catalog().into_iter().filter(|m| m.vram_mib == 0).enumerate().map(|(i, model)| {
            let installed = alcyo_models::installed(&model, &self.models);
            let active = !using_remote && current.as_ref().is_some_and(|path| path.ends_with(&model.file));
            let what = match model.id {
                "qwen3-4b-2507" => t("ipad-model-draws"),
                "qwen3.5-4b" => t("ipad-model-reads"),
                _ => t("ipad-model-other"),
            };
            let downloading = busy.filter(|(id, _)| *id == model.id).map(|(_, f)| f);
            let (fetch, choose) = (model.clone(), model.clone());
            let right: AnyElement = match () {
                _ if downloading.is_some() => {
                    let fraction = downloading.unwrap_or(0.0);
                    div()
                        .flex_none()
                        .flex()
                        .items_center()
                        .gap(px(12.))
                        .child(div().w(px(110.)).h(px(4.)).rounded(px(2.)).bg(rgb(p.track)).overflow_hidden().child(div().h_full().rounded(px(2.)).bg(rgb(p.accent)).with_spring(name("ipad-download", model.id), SpringAnimation::new(motion::GLIDE).to(fraction), |d, f| d.w(px(110. * f.clamp(0., 1.))))))
                        .child(div().w(px(36.)).font_family(theme::MONO).text_size(px(11.)).text_color(rgb(p.faint)).child(format!("{} %", (fraction * 100.0).round() as u32)))
                        .into_any_element()
                }
                _ if active => div().flex_none().flex().items_center().gap(px(6.)).text_size(px(13.5)).text_color(rgb(p.accent)).child(icon("ipad/check").size(px(16.))).child(t("in-use")).into_any_element(),
                _ if installed => pill_button(name("ipad-use-model", i), t("use"), false, None).on_click(cx.listener(move |this, _, _, cx| this.use_model(&choose, cx))).into_any_element(),
                _ if busy.is_some() => div().into_any_element(),
                _ => pill_button(name("ipad-get-model", i), t("download"), true, Some("download")).on_click(cx.listener(move |this, _, _, cx| this.start_download(fetch.clone(), cx))).into_any_element(),
            };
            row()
                .min_h(px(62.))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(8.))
                                .font_weight(FontWeight::MEDIUM)
                                .child(model.label)
                                .when(model.id == recommended, |d| d.child(div().px(px(7.)).py(px(2.)).rounded(px(6.)).bg(accent_soft()).text_color(rgb(p.accent)).text_size(px(11.)).font_weight(FontWeight::SEMIBOLD).child(t("recommended-2")))),
                        )
                        .child(div().text_size(px(12.5)).text_color(rgb(p.faint)).child(format!("{} · {what}", gigabytes(model.size)))),
                )
                .child(right)
                .into_any_element()
        });
        out.push(
            div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(label(t("ipad-on-this-ipad"), free.map(|free| div().child(ta!("ipad-free", size = gigabytes(free)).to_uppercase()).into_any_element())))
                .child(group(models.collect()))
                .into_any_element(),
        );
        if let Some(note) = self.form.note.clone().filter(|_| self.sheet.probing.is_none()) {
            out.push(div().px(px(4.)).mt(px(-8.)).text_size(px(12.5)).text_color(rgb(p.faint)).child(note).into_any_element());
        }
        out
    }

    /// Speech: what hears it, in which language, through which microphone, and what is kept.
    fn sheet_speech(&mut self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let p = pal();
        let apple = self.speech_helper.is_some() || cfg!(target_os = "ios");
        let system = apple && self.form.saved.system_speech;
        let choice = |id: &'static str, title: &str, hint: &str, on: bool, enabled: bool| {
            row()
                .id(id)
                .min_h(px(64.))
                .when(enabled, |d| d.cursor_pointer())
                .when(!enabled, |d| d.opacity(0.4))
                .child(radio(on, 24.))
                .child(titled(title, Some(hint.to_string())))
        };
        let recognizers = group(vec![
            choice("ipad-speech-apple", t("ipad-apple-speech"), t("ipad-apple-speech-hint"), system, apple).when(apple, |d| d.on_click(cx.listener(|this, _, _, cx| this.choose_speech(true, None, cx)))).into_any_element(),
            choice("ipad-speech-models", t("ipad-alcyo-speech"), t("ipad-alcyo-speech-hint"), !system, true).on_click(cx.listener(|this, _, _, cx| this.choose_speech(false, None, cx))).into_any_element(),
        ]);
        let language = self.form.saved.speech_language.clone();
        let languages = speech_languages(system);
        let shown = languages.iter().find(|(code, _)| *code == language).map_or_else(|| language.clone(), |(_, said)| said.clone());
        let listening = self.pipeline.listening();
        let level = if listening { self.level } else { 0.0 };
        // The microphone's level, live while listening.
        let meter = div().flex_none().flex().items_center().gap(px(2.5)).h(px(16.)).children([5.0_f32, 10., 15., 8., 11.].into_iter().enumerate().map(|(i, tall)| {
            // At rest the bars keep their shape, faint, as the microphone is not being heard.
            let h = if listening { (3.0 + tall * (0.25 + 0.75 * level) * (0.7 + 0.3 * ((i as f32) * 1.9 + level * 6.0).sin().abs())).min(16.) } else { tall };
            div().w(px(3.)).h(px(h)).rounded(px(2.)).bg(rgb(p.recording)).when(!listening, |d| d.opacity(0.45))
        }));
        let devices = group(vec![
            row()
                .child(key(t("language"), 120.))
                .child(menu_row("ipad-speech-language", shown, languages, language, |this, code: String, _, cx| this.choose_speech(this.form.saved.system_speech, Some(&code), cx), cx))
                .child(div().flex_1())
                .child(chevron("chevron-down"))
                .into_any_element(),
            row().child(key(t("microphone-2"), 120.)).child(div().flex_1().text_color(rgb(p.muted)).child(t("ipad-microphone-name"))).child(meter).into_any_element(),
        ]);
        let glossary = self.glossary();
        let names: Vec<String> = glossary.terms.iter().take(3).map(|term| term.text.clone()).collect();
        let glossary_hint = if names.is_empty() { t("ipad-glossary-hint").to_string() } else { ta!("ipad-glossary-hint-names", names = format!("{}{}", names.join(", "), if glossary.terms.len() > 3 { "…" } else { "" })) };
        let count = glossary.terms.len();
        let keep = group(vec![
            row()
                .child(titled(t("tell-speakers-apart-by-voice"), None))
                .child(switch("ipad-diarize", self.form.saved.diarize).id("ipad-diarize-switch").cursor_pointer().on_click(cx.listener(|this, _, _, cx| {
                    this.form.saved.diarize = !this.form.saved.diarize;
                    if let Err(e) = this.form.saved.save() {
                        this.form.note = Some(format!("{e:#}"));
                    }
                    cx.notify();
                })))
                .into_any_element(),
            row()
                .min_h(px(64.))
                .child(titled(t("ipad-keep-sound"), Some(t("ipad-keep-sound-hint").into())))
                .child(switch("ipad-keep-audio", self.form.saved.keep_audio).id("ipad-keep-audio-switch").cursor_pointer().on_click(cx.listener(|this, _, _, cx| {
                    let on = !this.form.saved.keep_audio;
                    this.set_keep_audio(on, cx);
                })))
                .into_any_element(),
            row()
                .id("ipad-glossary-row")
                .cursor_pointer()
                .child(titled(t("ipad-glossary"), Some(glossary_hint)))
                .when(count > 0, |d| d.child(div().font_family(theme::MONO).text_size(px(11.)).text_color(rgb(p.faint)).child(count.to_string())))
                .child(chevron("chevron-right"))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.sheet.glossary = true;
                    cx.notify();
                }))
                .into_any_element(),
        ]);
        vec![
            div().flex().flex_col().gap(px(8.)).child(label(t("ipad-recognize-with"), None)).child(recognizers).into_any_element(),
            devices.into_any_element(),
            div().mt(px(-8.)).px(px(4.)).text_size(px(12.5)).line_height(gpui_kit::relative(1.45)).text_color(rgb(p.muted)).child(t("ipad-speech-note")).into_any_element(),
            keep.into_any_element(),
        ]
    }

    /// General: the look and the language, what happens while listening, and the Pencil.
    fn sheet_general(&mut self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let look = self.form.saved.appearance.clone();
        let appearance = segmented(
            "ipad-appearance",
            270.,
            [("auto", t("system")), ("light", t("light")), ("dark", t("dark"))]
                .into_iter()
                .map(|(id, text)| (text.to_string(), look == id, Box::new(move |this: &mut Workbench, _: &mut Window, cx: &mut Context<Workbench>| this.choose_appearance(id, cx)) as Box<dyn Fn(&mut Workbench, &mut Window, &mut Context<Workbench>)>))
                .collect(),
            cx,
        );
        let language = self.form.saved.interface_language.clone();
        let chosen = if language == "zh" { "zh-Hans".to_string() } else { language };
        let languages: Vec<(String, String)> = std::iter::once(("auto", t("system"))).chain(crate::i18n::LANGUAGES.iter().copied()).map(|(code, said)| (code.to_string(), said.to_string())).collect();
        let shown = languages.iter().find(|(code, _)| *code == chosen).map_or_else(|| t("system").to_string(), |(_, said)| said.clone());
        let saved = &self.form.saved;
        let setting = |id: &'static str, title: &str, hint: &str, on: bool, flip: fn(&mut Workbench, &mut Context<Workbench>), cx: &mut Context<Workbench>| {
            row().min_h(px(64.)).child(titled(title, Some(hint.to_string()))).child(switch(id, on).id(name(id, "switch")).cursor_pointer().on_click(cx.listener(move |this, _, _, cx| flip(this, cx))))
        };
        fn saving(this: &mut Workbench, cx: &mut Context<Workbench>) {
            if let Err(e) = this.form.saved.save() {
                this.form.note = Some(format!("{e:#}"));
            }
            cx.notify();
        }
        let look_group = group(vec![
            row().min_h(px(58.)).child(key(t("appearance"), 150.)).child(div().flex_1()).child(appearance).into_any_element(),
            row()
                .child(key(t("language"), 150.))
                .child(menu_row("ipad-interface-language", shown, languages, chosen, |this, code: String, window, cx| this.choose_language(&code, window, cx), cx))
                .child(div().flex_1())
                .child(chevron("chevron-right"))
                .into_any_element(),
        ]);
        let listening = group(vec![
            setting("ipad-fade-tools", t("ipad-fade-tools"), t("ipad-fade-tools-hint"), saved.fade_tools, |this, cx| {
                this.form.saved.fade_tools = !this.form.saved.fade_tools;
                saving(this, cx);
            }, cx)
            .into_any_element(),
            setting("ipad-reduce-motion", t("reduce-motion"), t("ipad-reduce-motion-hint"), saved.reduce_motion, |this, cx| {
                let on = !this.form.saved.reduce_motion;
                this.set_reduce_motion(on, cx);
            }, cx)
            .into_any_element(),
            setting("ipad-keep-screen-on", t("ipad-keep-screen-on"), t("ipad-keep-screen-on-hint"), saved.keep_screen_on, |this, cx| {
                this.form.saved.keep_screen_on = !this.form.saved.keep_screen_on;
                saving(this, cx);
            }, cx)
            .into_any_element(),
        ]);
        let pencil_menu = |id: &'static str, chosen: String, set: fn(&mut Workbench, String, &mut Window, &mut Context<Workbench>), cx: &mut Context<Workbench>| {
            let actions = pencil_actions();
            let shown = actions.iter().find(|(a, _)| *a == chosen).map(|(_, said)| said.clone()).unwrap_or_default();
            menu_row(id, shown, actions, chosen, set, cx)
        };
        let pencil = group(vec![
            setting("ipad-pencil-only", t("ipad-pencil-only"), t("ipad-pencil-only-hint"), saved.pencil_only, |this, cx| {
                this.form.saved.pencil_only = !this.form.saved.pencil_only;
                saving(this, cx);
            }, cx)
            .into_any_element(),
            row()
                .child(key(t("ipad-double-tap"), 190.))
                .child(div().flex_1())
                .child(pencil_menu("ipad-double-tap-menu", saved.pencil_double_tap.clone(), |this, action, _, cx| {
                    this.form.saved.pencil_double_tap = action;
                    saving(this, cx);
                }, cx))
                .child(chevron("chevron-down"))
                .into_any_element(),
            row()
                .child(key(t("ipad-squeeze"), 190.))
                .child(div().flex_1())
                .child(pencil_menu("ipad-squeeze-menu", saved.pencil_squeeze.clone(), |this, action, _, cx| {
                    this.form.saved.pencil_squeeze = action;
                    saving(this, cx);
                }, cx))
                .child(chevron("chevron-down"))
                .into_any_element(),
        ]);
        vec![
            look_group.into_any_element(),
            listening.into_any_element(),
            div().flex().flex_col().gap(px(8.)).child(label(t("ipad-pencil-touch"), None)).child(pencil).into_any_element(),
        ]
    }

    /// Shortcuts, for a keyboard attached to the iPad: each group's in a list, a tap to set one's keys.
    fn sheet_shortcuts(&mut self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let p = pal();
        let mut out = vec![div().text_size(px(14.)).line_height(gpui_kit::relative(1.5)).text_color(rgb(p.muted)).child(t("click-a-shortcut-then-press")).into_any_element()];
        for group_of in [Group::Meeting, Group::Board, Group::App] {
            let rows = ACTIONS
                .iter()
                .enumerate()
                .filter(|(_, a)| a.2 == group_of)
                .map(|(i, (action, ..))| {
                    let action = *action;
                    let waiting = self.recording_keys == Some(action);
                    let shown = if waiting { t("press-keys").to_string() } else { self.keys.label(action).unwrap_or_else(|| t("none-2").to_string()) };
                    let note = self.keys_note.clone().filter(|_| waiting);
                    row()
                        .child(titled(action.name(), note))
                        .when(self.keys.changed(action) && !waiting, |d| {
                            d.child(pill_button(name("ipad-shortcut-reset", i), t("reset"), false, None).on_click(cx.listener(move |this, _, _, cx| {
                                this.keys.reset(action);
                                this.save_shortcuts();
                                cx.notify();
                            })))
                        })
                        .child(
                            div()
                                .id(name("ipad-shortcut", i))
                                .flex_none()
                                .min_w(px(96.))
                                .h(px(30.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .px(px(10.))
                                .rounded(px(8.))
                                .font_family(theme::MONO)
                                .text_size(px(12.))
                                .cursor_pointer()
                                .map(|d| if waiting { d.bg(rgb(p.accent)).text_color(rgb(on_accent())) } else { d.bg(fill(1.)).text_color(rgb(p.muted)) })
                                .child(shown)
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.recording_keys = Some(action);
                                    this.keys_note = None;
                                    // The keys go to the board's handler, which takes them while a shortcut waits.
                                    window.focus(&this.focus, cx);
                                    cx.notify();
                                })),
                        )
                        .into_any_element()
                })
                .collect();
            out.push(div().flex().flex_col().gap(px(8.)).child(label(group_of.name(), None)).child(group(rows)).into_any_element());
        }
        if !self.keys.custom().is_empty() {
            out.push(
                div()
                    .flex()
                    .child(pill_button("ipad-shortcuts-reset-all", t("reset-all-shortcuts"), false, None).on_click(cx.listener(|this, _, _, cx| {
                        this.keys.reset_all();
                        this.recording_keys = None;
                        this.keys_note = None;
                        this.save_shortcuts();
                        cx.notify();
                    })))
                    .into_any_element(),
            );
        }
        out
    }

    /// About: the app, its version, and where its meetings are kept.
    fn sheet_about(&mut self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let p = pal();
        let app = row()
            .min_h(px(84.))
            .gap(px(16.))
            .child(div().flex_none().size(px(56.)).flex().items_center().justify_center().rounded(px(14.)).bg(rgb(p.surface)).shadow(vec![ring(rgb(p.line).into(), 1.)]).child(kingfisher(40.)))
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(div().text_size(px(19.)).font_weight(FontWeight::BOLD).child("Alcyo"))
                    .child(div().font_family(theme::MONO).text_size(px(12.)).text_color(rgb(p.faint)).child(ta!("version-number", version = alcyo_update::VERSION)))
                    .child(div().pt(px(2.)).text_size(px(13.)).text_color(rgb(p.muted)).child(t("ipad-about-line"))),
            );
        let mut out = vec![group(vec![app.into_any_element()]).into_any_element()];
        let mut rows: Vec<AnyElement> = vec![];
        rows.push(row().child(titled(t("meetings"), None)).child(div().font_family(theme::MONO).text_size(px(12.)).text_color(rgb(p.faint)).child(self.library.len().to_string())).into_any_element());
        // A desktop build laid out as the iPad can still show its folder; the iPad's Files app shows its own.
        if let (false, Some(store)) = (cfg!(target_os = "ios"), self.store.as_ref()) {
            let folder = store.folder();
            rows.push(
                row()
                    .child(titled(t("ipad-meetings-folder"), Some(folder.display().to_string())))
                    .child(pill_button("ipad-open-data", t("open-folder"), false, None).on_click(cx.listener(move |_, _, _, cx| {
                        let _ = std::fs::create_dir_all(&folder);
                        cx.open_with_system(&folder);
                    })))
                    .into_any_element(),
            );
        }
        out.push(group(rows).into_any_element());
        out
    }
}
