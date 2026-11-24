//! The iPad's first run (#341): three screens from install to a live board. Why the microphone is needed,
//! said before the system asks; where the board model runs, the one real decision; and where to put the
//! iPad and what Pencil does, then start. A picture beside each, which slides along more slowly than the
//! words as the steps go by.
//!
//! Shown once on the iPad, until it is finished (`onboarded` in the settings). A desktop build laid out as
//! the iPad shows it when started with `ALCYO_ONBOARDING=1` (or 2 or 3, to start at that step), so it can be
//! worked on and pictured. A child of `app`, laid over the screen by sheet.rs.

use super::*;
use gpui_kit::{AnyElement, ElementId, FontWeight, Image, ImageFormat, ObjectFit};

const STEPS: usize = 3;
/// The pictures beside the steps, cut from the design at twice its size (promo/mockup/ipad/Onboarding.html).
const ART: [&[u8]; 3] = [include_bytes!("../art/onboarding-1.jpg"), include_bytes!("../art/onboarding-2.jpg"), include_bytes!("../art/onboarding-3.jpg")];
const ART_DARK_3: &[u8] = include_bytes!("../art/onboarding-3-dark.jpg");
/// Their size in points, and where in each what it shows sits, as shares of it.
const ART_W: f32 = 520.0;
const ART_H: f32 = 834.0;
const ART_FOCUS: [(f32, f32); 3] = [(0.5, 0.44), (0.5, 0.37), (0.5, 0.45)];
/// The board model the iPad downloads when the board is drawn on it.
const LOCAL_MODEL: &str = "qwen3-4b-2507";

/// Where the first run is.
pub(super) struct Intro {
    /// Whether it is shown, once decided at the first frame.
    shown: Option<bool>,
    step: usize,
    /// Which way the steps went last: 1 forward, -1 back.
    went: f32,
    /// Remote chosen for the board, rather than this iPad.
    remote: bool,
    /// The API key pasted or typed, made at the first frame that shows it.
    key: Option<Entity<InputState>>,
    /// The system's answer about the microphone, once it is asked.
    asking: Option<Receiver<bool>>,
}

impl Intro {
    pub(super) fn new() -> Self {
        Self { shown: None, step: 0, went: 1.0, remote: true, key: None, asking: None }
    }
}

/// A step's picture, kept once decoded.
fn art(step: usize) -> Arc<Image> {
    thread_local! {
        static ART_IMAGES: RefCell<HashMap<(usize, bool), Arc<Image>>> = RefCell::new(HashMap::new());
    }
    // The last picture is the board itself, so it follows light and dark; the first two are dark either way.
    let dark = step == 2 && theme::is_dark();
    ART_IMAGES.with(|images| {
        images
            .borrow_mut()
            .entry((step, dark))
            .or_insert_with(|| Arc::new(Image::from_bytes(ImageFormat::Jpeg, if dark { ART_DARK_3 } else { ART[step] }.to_vec())))
            .clone()
    })
}

/// Ask the system for the microphone, which shows its prompt once; the answer comes back on `answered`.
#[cfg(target_os = "ios")]
fn ask_for_microphone(answered: std::sync::mpsc::Sender<bool>) {
    use objc2::msg_send;
    use objc2::runtime::{AnyClass, AnyObject, Bool};
    let Some(class) = AnyClass::get(c"AVAudioSession") else {
        let _ = answered.send(false);
        return;
    };
    let reply = block2::RcBlock::new(move |granted: Bool| {
        let _ = answered.send(granted.as_bool());
    });
    // SAFETY: AVAudioSession's shared instance, and its requestRecordPermission:, which calls the block once
    // with the answer, on a thread of its own.
    unsafe {
        let session: *mut AnyObject = msg_send![class, sharedInstance];
        if !session.is_null() {
            let _: () = msg_send![session, requestRecordPermission: &*reply];
        }
    }
}

/// Elsewhere the microphone is asked for when listening first starts, if the system asks at all.
#[cfg(not(target_os = "ios"))]
fn ask_for_microphone(answered: std::sync::mpsc::Sender<bool>) {
    let _ = answered.send(true);
}

impl Workbench {
    /// The first run over the whole screen, while it is shown.
    pub(super) fn onboarding(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let shown = *self.intro.shown.get_or_insert_with(|| {
            let asked = std::env::var("ALCYO_ONBOARDING").ok().and_then(|v| v.parse::<usize>().ok()).filter(|n| (1..=STEPS).contains(n));
            if let Some(step) = asked {
                self.intro.step = step - 1;
            }
            crate::platform::ipad_layout() && (asked.is_some() || cfg!(target_os = "ios") && !self.form.saved.onboarded)
        });
        if !shown {
            return None;
        }
        if self.intro.key.is_none() {
            self.intro.key = Some(cx.new(|cx| InputState::new(window, cx).placeholder("sk-…").masked(true)));
        }
        let size = window.viewport_size();
        let (w, h) = (f32::from(size.width), f32::from(size.height));
        let upright = h > w;
        let big = w.max(h) >= 1300.0;
        let step = self.intro.step;
        let went = self.intro.went;
        let dark_art = step < 2;

        // The picture covers its pane, kept on what it shows (the kingfisher, the board) however the pane is cut:
        // narrow beside the words, or a band over them when the iPad is upright.
        let (pane_w, pane_h) = if upright { (w, (h * 0.36).round()) } else { ((w * 0.4355).round(), h) };
        // Upright, the dark pictures are shown at about their size on the dark they fade into; the last, a board
        // on dotted paper, has no such edge and covers the band.
        let scale = if upright && dark_art { pane_h / (ART_H * 0.5) } else { (pane_w / ART_W).max(pane_h / ART_H) };
        let (art_w, art_h) = (ART_W * scale, ART_H * scale);
        let (focus_x, focus_y) = ART_FOCUS[step];
        // Centred on what it shows, without leaving an edge of the pane bare where it is big enough to cover it.
        let place = |pane: f32, art: f32, focus: f32| if art <= pane { (pane - art) / 2.0 } else { (pane / 2.0 - focus * art).clamp(pane - art, 0.0) };
        let (art_left, art_top) = (place(pane_w, art_w, focus_x), place(pane_h, art_h, focus_y));
        // It slides in a little behind the words: the same way, but less far.
        let picture = div()
            .absolute()
            .top(px(art_top))
            .w(px(art_w))
            .h(px(art_h))
            .child(gpui_kit::img(art(step)).size_full().object_fit(ObjectFit::Fill))
            .with_spring(ElementId::Name(format!("ob-art-{step}").into()), SpringAnimation::new(motion::SETTLE).to(true).from(false), move |d, v| {
                let v = v.0.clamp(0., 1.);
                d.opacity(v).left(px(art_left + went * 36. * (1. - v)))
            });
        let (cap_left, cap_right) = match step {
            0 => (t("ob-art-1"), t("ob-art-1-2")),
            1 => (t("ob-art-2"), t("ob-art-2-2")),
            _ => (t("ob-art-3"), t("ob-art-3-2")),
        };
        let art_pane = div()
            .relative()
            .flex_none()
            .overflow_hidden()
            .bg(rgb(if dark_art { 0x0d_0d_0e } else { pal().paper }))
            .w(px(pane_w))
            .h(px(pane_h))
            .child(picture)
            .child(
                div()
                    .absolute()
                    .left(px(40.))
                    .right(px(40.))
                    .bottom(px(if upright { 24. } else { 40. }))
                    .flex()
                    .justify_between()
                    .font_family(theme::MONO)
                    .text_size(px(13.))
                    .text_color(rgb(if dark_art { 0xc4_c4_c4 } else { pal().faint }))
                    .child(cap_left.to_uppercase())
                    .child(cap_right.to_uppercase()),
            );

        // Where the steps are: "1 / 3" and a bar a step, filling in as they go.
        let steps = div()
            .absolute()
            .right(px(40.))
            .top(px(if upright { 28. } else { 44. }))
            .flex()
            .items_center()
            .gap(px(8.))
            .font_family(theme::MONO)
            .text_size(px(11.))
            .text_color(rgb(pal().faint))
            .child(ta!("ob-step", n = step + 1, of = STEPS))
            .children((0..STEPS).map(|i| {
                div().w(px(22.)).h(px(4.)).rounded(px(2.)).bg(rgb(pal().track)).child(div().h_full().rounded(px(2.)).bg(rgb(pal().text)).with_spring(ElementId::Name(format!("ob-bar-{i}").into()), SpringAnimation::new(motion::SETTLE).to(i <= step), |d, v| d.w(px(22. * v.0.clamp(0., 1.)))))
            }));

        let content = match step {
            0 => self.intro_microphone(cx),
            1 => self.intro_model(cx),
            _ => self.intro_start(cx),
        };
        let words = div()
            .relative()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .map(|d| if upright { d.pt(px(56.)).px(px(56.)).pb(px(40.)) } else { d.pt(px(if big { 110. } else { 84. })).pr(px(64.)).pb(px(44.)).pl(px(60.)) })
            .child(content.with_spring(ElementId::Name(format!("ob-words-{step}").into()), SpringAnimation::new(motion::SETTLE).to(true).from(false), move |d, v| {
                let v = v.0.clamp(0., 1.);
                d.opacity(v).ml(px(went * 90. * (1. - v)))
            }))
            .child(steps);

        Some(
            div()
                .id("onboarding")
                .occlude()
                .absolute()
                .top_0()
                .left_0()
                .w(px(w))
                .h(px(h))
                .flex()
                .when(upright, |d| d.flex_col())
                .bg(rgb(pal().paper))
                .text_color(rgb(pal().text))
                .font(theme::interface_font())
                .child(art_pane)
                .child(words)
                .into_any_element(),
        )
    }

    fn intro_go(&mut self, step: usize, cx: &mut Context<Self>) {
        self.intro.went = if step >= self.intro.step { 1.0 } else { -1.0 };
        self.intro.step = step.min(STEPS - 1);
        cx.notify();
    }

    /// The first run is over: it is not shown again.
    fn intro_done(&mut self, cx: &mut Context<Self>) {
        self.intro.shown = Some(false);
        self.form.saved.onboarded = true;
        if let Err(e) = self.form.saved.save() {
            self.form.note = Some(format!("{e:#}"));
        }
        cx.notify();
    }

    /// A step's headline and the line under it.
    fn intro_heading(title: &str, text: Option<&str>) -> gpui_kit::Div {
        div()
            .flex()
            .flex_col()
            .gap(px(20.))
            .child(div().text_size(px(44.)).line_height(gpui_kit::relative(1.05)).font_weight(FontWeight::BOLD).child(title.to_string()))
            .when_some(text, |d, text| d.child(div().max_w(px(520.)).text_size(px(17.)).line_height(gpui_kit::relative(1.5)).text_color(rgb(pal().muted)).child(text.to_string())))
    }

    /// A tip: its picture in a tile, what to do, and why.
    fn intro_tip(glyph: &str, title: &str, text: &str) -> gpui_kit::Div {
        div()
            .flex()
            .items_start()
            .gap(px(16.))
            .child(
                div()
                    .flex_none()
                    .size(px(44.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(13.))
                    .bg(rgb(pal().chrome))
                    .border_1()
                    .border_color(rgb(pal().line))
                    .child(icon(&format!("ipad/{glyph}")).size(px(22.))),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(div().text_size(px(16.)).font_weight(FontWeight::SEMIBOLD).child(title.to_string()))
                    .child(div().text_size(px(14.5)).line_height(gpui_kit::relative(1.45)).text_color(rgb(pal().muted)).child(text.to_string())),
            )
    }

    /// The strong button that moves the first run on, a little smaller while pressed.
    fn intro_button(id: &'static str, text: &str, listening: bool) -> gpui_kit::Stateful<gpui_kit::Div> {
        let (h, radius) = if listening { (60., 18.) } else { (50., 14.) };
        let face = div()
            .id(ElementId::Name(format!("{id}-face").into()))
            .absolute()
            .top_0()
            .bottom_0()
            .left_0()
            .right_0()
            .flex()
            .items_center()
            .justify_center()
            .gap(px(12.))
            .rounded(px(radius))
            .text_size(px(if listening { 18. } else { 16. }))
            .font_weight(FontWeight::SEMIBOLD)
            .whitespace_nowrap()
            .map(|d| if listening { d.bg(rgb(pal().recording)).text_color(rgb(0xff_ff_ff)) } else { d.bg(rgb(pal().text)).text_color(rgb(pal().paper)) })
            .when(listening, |d| d.child(div().size(px(22.)).flex().items_center().justify_center().rounded_full().bg(gpui_kit::hsla(0., 0., 1., 0.25)).child(div().size(px(12.)).rounded_full().bg(rgb(0xff_ff_ff)))))
            .child(text.to_string());
        div()
            .id(id)
            .group(id)
            .relative()
            .flex_none()
            .h(px(h))
            .min_w(px(220.))
            .px(px(28.))
            .cursor_pointer()
            // Its size, held by text that is never seen, so the face can shrink inside it when pressed.
            .child(div().invisible().h_full().flex().items_center().gap(px(12.)).text_size(px(if listening { 18. } else { 16. })).font_weight(FontWeight::SEMIBOLD).whitespace_nowrap().when(listening, |d| d.child(div().size(px(22.)))).child(text.to_string()))
            .child(face.group_active(id, |s| s.top(px(1.5)).bottom(px(1.5)).left(px(4.)).right(px(4.)).opacity(0.92)))
    }

    fn intro_link(id: &'static str, text: &str) -> gpui_kit::Stateful<gpui_kit::Div> {
        div().id(id).text_size(px(15.)).font_weight(FontWeight::MEDIUM).text_color(rgb(pal().accent)).cursor_pointer().active(|d| d.opacity(0.6)).child(text.to_string())
    }

    /// 1: what Alcyo does, and why it needs the microphone, before the system asks for it.
    fn intro_microphone(&mut self, cx: &mut Context<Self>) -> gpui_kit::Div {
        div()
            .flex_1()
            .flex()
            .flex_col()
            .gap(px(20.))
            .child(Self::intro_heading(t("ob-1-title"), Some(t("ob-1-text"))))
            .child(div().mt(px(6.)).child(Self::intro_tip("mic", t("ob-1-tip"), t("ob-1-tip-hint"))))
            .child(div().flex_1())
            .child(div().flex().items_center().gap(px(14.)).child(Self::intro_button("ob-allow-microphone", t("ob-1-allow"), false).on_click(cx.listener(|this, _, _, cx| {
                let (tx, rx) = channel();
                ask_for_microphone(tx);
                this.intro.asking = Some(rx);
                // The answer comes from the system in its own time; the first run moves on once it has.
                cx.spawn(async move |this, cx| loop {
                    cx.background_executor().timer(Duration::from_millis(120)).await;
                    let done = this
                        .update(cx, |this, cx| match this.intro.asking.as_ref().map(|rx| rx.try_recv()) {
                            Some(Err(TryRecvError::Empty)) => false,
                            Some(_) => {
                                this.intro.asking = None;
                                this.intro_go(1, cx);
                                true
                            }
                            None => true,
                        })
                        .unwrap_or(true);
                    if done {
                        break;
                    }
                })
                .detach();
            }))))
    }

    /// 2: who draws the board, remote with a key or this iPad with a download.
    fn intro_model(&mut self, cx: &mut Context<Self>) -> gpui_kit::Div {
        let p = pal();
        // Until llama.cpp runs inside the app the iPad draws with a remote model only (platform.rs): the local
        // choice is shown as coming, and nothing is downloaded for it.
        let local_possible = crate::platform::LOCAL_MODELS;
        let remote = self.intro.remote || !local_possible;
        let local = alcyo_models::catalog().into_iter().find(|m| m.id == LOCAL_MODEL);
        let card = |id: &'static str, glyph: &str, title: String, text: String, on: bool| {
            div()
                .id(id)
                .relative()
                .flex()
                .flex_col()
                .gap(px(10.))
                .p(px(20.))
                .rounded(px(20.))
                .cursor_pointer()
                .map(|d| if on { d.bg(rgb(if theme::is_dark() { 0x16_1c_33 } else { 0xee_f1_fd })).border_2().border_color(rgb(p.accent)).p(px(19.)) } else { d.border_1().border_color(rgb(p.line)) })
                .child(
                    div()
                        .absolute()
                        .top(px(if on { 17. } else { 18. }))
                        .right(px(if on { 17. } else { 18. }))
                        .size(px(24.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_full()
                        .map(|d| if on { d.bg(rgb(p.accent)).text_color(rgb(if theme::is_dark() { 0x0b_10_20 } else { 0xff_ff_ff })).child(icon("ipad/check").size(px(14.))) } else { d.border(px(1.5)).border_color(rgb(if theme::is_dark() { 0x48_48_4a } else { 0xc7_c7_cc })) }),
                )
                .child(div().flex().items_center().gap(px(10.)).pr(px(30.)).text_size(px(18.)).font_weight(FontWeight::SEMIBOLD).child(icon(&format!("ipad/{glyph}")).size(px(22.))).child(title))
                .child(div().text_size(px(14.)).line_height(gpui_kit::relative(1.45)).text_color(rgb(p.muted)).child(text))
        };
        let key = self.intro.key.clone().expect("made before the steps");
        let field = div()
            .h(px(48.))
            .flex()
            .items_center()
            .gap(px(8.))
            .pl(px(14.))
            .pr(px(6.))
            .rounded(px(13.))
            .bg(rgb(p.surface))
            .border_1()
            .border_color(rgb(p.line))
            .font_family(theme::MONO)
            .text_size(px(13.5))
            .child(div().flex_1().min_w_0().child(Input::new(&key).appearance(false)))
            .child(
                div()
                    .id("ob-paste")
                    .flex_none()
                    .h(px(36.))
                    .flex()
                    .items_center()
                    .px(px(14.))
                    .rounded(px(10.))
                    .bg(gpui_kit::hsla(240. / 360., 0.04, 0.48, if theme::is_dark() { 0.24 } else { 0.12 }))
                    .font_family(theme::SANS)
                    .text_size(px(13.5))
                    .font_weight(FontWeight::SEMIBOLD)
                    .cursor_pointer()
                    .active(|d| d.opacity(0.7))
                    .child(t("ob-paste"))
                    .on_click(cx.listener(|this, _, window, cx| {
                        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else { return };
                        if let Some(key) = &this.intro.key {
                            key.update(cx, |state, cx| state.set_value(text.trim().to_string(), window, cx));
                        }
                        this.intro.remote = true;
                        cx.notify();
                    })),
            );
        let remote_card = card("ob-remote", "cloud", t("ob-2-remote").to_string(), t("ob-2-remote-hint").to_string(), remote)
            .when(remote, |d| d.child(field))
            .on_click(cx.listener(|this, _, _, cx| {
                this.intro.remote = true;
                cx.notify();
            }));
        let local_card = local.map(|model| {
            if local_possible {
                card("ob-local", "chip", ta!("ob-2-local", model = model.label), ta!("ob-2-local-hint", size = gigabytes(model.size)), !remote).on_click(cx.listener(|this, _, _, cx| {
                    this.intro.remote = false;
                    cx.notify();
                }))
            } else {
                card("ob-local", "chip", t("ipad-on-this-ipad").to_string(), t("ipad-local-coming").to_string(), false).cursor_default().opacity(0.55)
            }
        });
        div()
            .flex_1()
            .flex()
            .flex_col()
            .gap(px(16.))
            .child(Self::intro_heading(t("ob-2-title"), Some(t("ob-2-text"))))
            .child(remote_card)
            .when_some(local_card, |d, card| d.child(card))
            .child(div().flex_1())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(14.))
                    .child(Self::intro_button("ob-continue", t("ob-continue"), false).on_click(cx.listener(|this, _, window, cx| this.intro_choose(window, cx))))
                    .child(Self::intro_link("ob-later", t("ob-later")).on_click(cx.listener(|this, _, _, cx| this.intro_go(2, cx)))),
            )
    }

    /// Keep what was chosen for the board: the remote model with its key, or this iPad's, downloading.
    fn intro_choose(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.intro.remote || !crate::platform::LOCAL_MODELS {
            let key = self.intro.key.as_ref().map(|k| k.read(cx).value().trim().to_string()).unwrap_or_default();
            if !key.is_empty() {
                // DeepSeek, filled in as Settings' own choice of it is.
                let (_, base, model) = REMOTE_PRESETS[0];
                if self.form.base_url.read(cx).value().trim().is_empty() {
                    self.form.base_url.update(cx, |s, cx| s.set_value(base, window, cx));
                }
                if self.form.model.read(cx).value().trim().is_empty() {
                    self.form.model.update(cx, |s, cx| s.set_value(model, window, cx));
                }
                self.form.key.update(cx, |s, cx| s.set_value(key, window, cx));
                self.form.remote = true;
                self.save_settings(window, cx);
            }
        } else if let Some(model) = alcyo_models::catalog().into_iter().find(|m| m.id == LOCAL_MODEL) {
            self.form.remote = false;
            // It keeps downloading while the meeting starts; Settings shows how far it is.
            if !alcyo_models::installed(&model, &self.models) {
                self.start_download(model, cx);
            }
        }
        self.intro_go(2, cx);
    }

    /// 3: where to put the iPad and what Pencil does, then start.
    fn intro_start(&mut self, cx: &mut Context<Self>) -> gpui_kit::Div {
        div()
            .flex_1()
            .flex()
            .flex_col()
            .gap(px(20.))
            .child(Self::intro_heading(t("ob-3-title"), None))
            .child(
                div()
                    .mt(px(6.))
                    .flex()
                    .flex_col()
                    .gap(px(18.))
                    .child(Self::intro_tip("mic", t("ob-3-tip-1"), t("ob-3-tip-1-hint")))
                    .child(Self::intro_tip("lasso", t("ob-3-tip-2"), t("ob-3-tip-2-hint")))
                    .child(Self::intro_tip("airplay", t("ob-3-tip-3"), t("ob-3-tip-3-hint"))),
            )
            .child(div().flex_1())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(14.))
                    .child(Self::intro_button("ob-start-listening", t("start-listening"), true).on_click(cx.listener(|this, _, _, cx| {
                        this.intro_done(cx);
                        // A new meeting, unless the one open has nothing in it yet.
                        if !this.live.lines.is_empty() || !this.live.graph.nodes.is_empty() {
                            this.new_meeting(cx);
                        }
                        if this.editable() && !this.pipeline.listening() {
                            this.toggle_mic();
                        }
                        cx.notify();
                    })))
                    .child(Self::intro_link("ob-past", t("ob-3-past")).on_click(cx.listener(|this, _, _, cx| {
                        this.intro_done(cx);
                        if this.panel != Panel::Library {
                            this.toggle_panel(Panel::Library, cx);
                        }
                    }))),
            )
    }
}
