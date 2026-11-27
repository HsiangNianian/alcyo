//! Voice packs: the speech models no longer come with the app (only the voice detector and the meeting search model
//! do); the person picks the languages their meetings use and those packs download together in the background
//! (alcyo_models::voice). Here is what the window keeps of them and shows:
//!
//! - the first run's question, a page of its own on the desktop and a step of the iPad's first run (onboarding.rs),
//!   pre-ticked by the system's language, with the total, the free space, and the way out (Later, or Apple's
//!   recognizer on a Mac and the iPad);
//! - the download's progress on that page or step, then in a sidebar row with a popover (desktop) or a sheet (iPad);
//! - what happens when listening starts before a pack that hears is here: Apple's recognizer listens meanwhile on a
//!   Mac and the iPad, and elsewhere a note says so and can start listening by itself once it is;
//! - the packs that came with the person's old package downloading again after an update, said once;
//! - Settings › Speech's rows (download, delete, progress), Storage's, and the switches that need a pack.
//!
//! A pack that arrives while listening is not switched to; it hears from the next time listening starts. Updates to
//! a pack download by themselves. A child of `app`.

use super::*;
use alcyo_models::voice::{self, Choice, Got, Pack, Places, Status};
use gpui_kit::{AnyElement, ElementId, FontWeight};

/// How long a short note over the dock stays.
const NOTE_FOR: Duration = Duration::from_secs(6);
/// How long "ready" stays in the sidebar row after the last pack arrived.
const READY_FOR: Duration = Duration::from_secs(8);
/// How long the word about packs coming back after an update stays.
const TOLD_FOR: Duration = Duration::from_secs(14);

/// A note over the dock about listening and the packs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Note {
    /// Nothing can hear yet; the packs are downloading (Linux, Windows).
    Waiting,
    /// Apple's recognizer listens until the packs are here (a Mac, the iPad).
    Apple,
    /// A pack that hears arrived and listening started by itself.
    ReadyListening,
    /// The packs arrived while Apple's recognizer listened: they hear from the next start.
    ReadyNext,
    /// The download stopped, so nothing can hear.
    Stopped,
    /// No pack was chosen, so nothing can hear.
    NoneYet,
}

struct Job {
    events: Receiver<voice::Event>,
    cancel: Arc<AtomicBool>,
    got: Got,
}

pub(super) struct Packs {
    pub(super) places: Places,
    pub(super) choice: Choice,
    status: Vec<(Pack, Status)>,
    job: Option<Job>,
    /// How far the last download got, kept while it is paused or stopped.
    got: Option<Got>,
    /// Why the last download stopped: it is not tried again by itself until Retry.
    pub(super) failed: Option<String>,
    pub(super) paused: bool,
    /// The source to use first on the next attempt, when the person asked for one.
    prefer: Option<&'static str>,
    /// The first run's question is being asked, and what is ticked in it.
    pub(super) asking: bool,
    pub(super) ticks: Vec<Pack>,
    /// The page or step shows the download, after Download was pressed.
    pub(super) showing_progress: bool,
    /// Free bytes where the packs go, and when that was looked at.
    free: Option<(u64, Instant)>,
    /// Listening starts by itself once a pack that hears is here.
    pub(super) armed: bool,
    pub(super) note: Option<(Note, Instant)>,
    /// A pack arrived while listening: the recognizers are loaded again at the next start.
    reload: bool,
    /// The details are open: the popover by the sidebar row, or the iPad's sheet.
    pub(super) details: bool,
    /// When the last download finished, for the row to say so a while.
    ready_at: Option<Instant>,
    /// Whether the last download finished bringing back the packs of the old package.
    restored: bool,
    /// When the one-time word about packs coming back after an update was first shown.
    told_at: Option<Instant>,
}

impl Packs {
    /// What was chosen before, or, the first time, whether to ask: someone updating from a version that came with
    /// its speech models gets those back without being asked (`used_before`, `variant` of the old package when it is
    /// still known), nothing if they listened with Apple's recognizer.
    pub(super) fn new(places: Places, used_before: bool, apple: bool, variant: Option<String>, language: &str, locale: &str) -> Self {
        let choice = match Choice::load(&places.packs) {
            Some(choice) => choice,
            None => match voice::after_update(used_before, apple, variant.as_deref(), language) {
                Some(packs) => {
                    let wanted = voice::closed(&packs);
                    let restoring: Vec<Pack> = wanted.iter().copied().filter(|p| !places.status(*p).usable()).collect();
                    let choice = Choice { wanted, decided: true, told: restoring.is_empty(), restoring };
                    if let Err(e) = choice.save(&places.packs) {
                        eprintln!("{e:#}");
                    }
                    choice
                }
                None => Choice::default(),
            },
        };
        let status: Vec<(Pack, Status)> = Pack::ALL.iter().map(|p| (*p, places.status(*p))).collect();
        // A checkout carries every model: nothing to ask, unless asked to show it (ALCYO_VOICE_PACKS=ask).
        let forced = std::env::var("ALCYO_VOICE_PACKS").is_ok_and(|v| v == "ask");
        let asking = forced || !choice.decided && !status.iter().any(|(_, s)| *s == Status::BuiltIn);
        Self {
            places,
            choice,
            status,
            job: None,
            got: None,
            failed: None,
            paused: false,
            prefer: None,
            asking,
            ticks: voice::preset(locale),
            showing_progress: false,
            free: None,
            armed: false,
            note: None,
            reload: false,
            details: false,
            ready_at: None,
            restored: false,
            told_at: None,
        }
    }

    pub(super) fn status(&self, pack: Pack) -> Status {
        self.status.iter().find(|(p, _)| *p == pack).map_or(Status::Missing, |(_, s)| *s)
    }

    pub(super) fn usable(&self, pack: Pack) -> bool {
        self.status(pack).usable()
    }

    /// Whether a pack that recognizes speech is here.
    pub(super) fn hears(&self) -> bool {
        [Pack::Zh, Pack::En, Pack::Ja].into_iter().any(|p| self.usable(p))
    }

    fn refresh(&mut self) {
        self.status = Pack::ALL.iter().map(|p| (*p, self.places.status(*p))).collect();
    }

    /// The download now, or how far the last one got while it is paused or stopped.
    pub(super) fn got(&self) -> Option<&Got> {
        self.job.as_ref().map(|j| &j.got).or(self.got.as_ref())
    }

    /// Whether the packs are under way in any sense: downloading, paused or stopped part way.
    pub(super) fn under_way(&self) -> bool {
        self.job.is_some() || (self.paused || self.failed.is_some()) && self.got.is_some()
    }

    /// The wanted packs not here as the catalogue has them.
    fn to_fetch(&self) -> Vec<Pack> {
        self.choice.wanted.iter().copied().filter(|p| !matches!(self.status(*p), Status::Downloaded | Status::BuiltIn)).collect()
    }

    fn save(&self) {
        if let Err(e) = self.choice.save(&self.places.packs) {
            eprintln!("{e:#}");
        }
    }

    /// Free bytes where the packs go, looked at again every few seconds.
    pub(super) fn free(&mut self) -> Option<u64> {
        // To try the low-space state without filling a disk: the free bytes to pretend.
        if let Some(free) = std::env::var("ALCYO_PACKS_FREE").ok().and_then(|v| v.parse().ok()) {
            return Some(free);
        }
        if self.free.is_none_or(|(_, at)| at.elapsed() > Duration::from_secs(5)) {
            let dir = self.places.packs.ancestors().find(|d| d.exists()).map(std::path::Path::to_path_buf).unwrap_or_default();
            self.free = alcyo_models::free_space(&dir).ok().map(|f| (f, Instant::now()));
        }
        self.free.map(|(f, _)| f)
    }

    /// Whether `bytes` more fit on the disk, leaving the downloader's margin.
    pub(super) fn fits(&mut self, bytes: u64) -> bool {
        self.free().is_none_or(|free| bytes + voice::SPACE_MARGIN <= free)
    }

    /// The share of the download done, as a whole percent.
    pub(super) fn percent(&self) -> u32 {
        self.got().map_or(0, |g| (g.share() * 100.0).floor() as u32)
    }
}

/// A size as the packs show it: whole megabytes, or gigabytes to two places.
pub(super) fn mb(bytes: u64) -> String {
    let mb = bytes as f64 / 1e6;
    if mb >= 1000.0 { format!("{:.2} GB", mb / 1000.0) } else { format!("{} MB", mb.round() as u64) }
}

pub(super) fn pack_name(pack: Pack) -> &'static str {
    match pack {
        Pack::Zh => t("vp-zh-name"),
        Pack::Live => t("vp-live-name"),
        Pack::En => t("vp-en-name"),
        Pack::Ja => t("vp-ja-name"),
        Pack::Spk => t("vp-spk-name"),
    }
}

pub(super) fn pack_tag(pack: Pack) -> &'static str {
    match pack {
        Pack::Zh => t("vp-zh-tag"),
        Pack::Live => t("vp-live-tag"),
        Pack::En => t("vp-en-tag"),
        Pack::Ja => t("vp-ja-tag"),
        Pack::Spk => t("vp-spk-tag"),
    }
}

pub(super) fn pack_hint(pack: Pack) -> &'static str {
    match pack {
        Pack::Zh => t("vp-zh-hint"),
        Pack::Live => t("vp-live-hint"),
        Pack::En => t("vp-en-hint"),
        Pack::Ja => t("vp-ja-hint"),
        Pack::Spk => t("vp-spk-hint"),
    }
}

/// The pack a recognition language needs: "zh-CN", "en-US", "ja-JP"; none for automatic.
pub(super) fn pack_for_language(code: &str) -> Option<Pack> {
    match code {
        "zh-CN" => Some(Pack::Zh),
        "en-US" => Some(Pack::En),
        "ja-JP" => Some(Pack::Ja),
        _ => None,
    }
}

/// Where a download comes from, as said: the mirror is Cloudflare's.
fn source_name(source: &str) -> String {
    if source == "Alcyo" { "Cloudflare · models.alcyo.app".to_string() } else { source.to_string() }
}

/// How long the rest will take, roughly.
fn time_left(got: &Got) -> Option<String> {
    if got.speed == 0 {
        return None;
    }
    let seconds = got.total().saturating_sub(got.done()) / got.speed;
    Some(if seconds < 60 { ta!("vp-seconds", n = (seconds.max(1) as f64 / 5.0).ceil() as u64 * 5) } else { ta!("vp-minutes", n = seconds.div_ceil(60)) })
}

fn speed(got: &Got) -> Option<String> {
    (got.speed > 0).then(|| if got.speed < 1_000_000 { format!("{} KB/s", got.speed / 1000) } else { format!("{:.1} MB/s", got.speed as f64 / 1e6) })
}

/// A ring that fills as `share` goes from 0 to 1, `size` across.
pub(super) fn ring(share: f32, size: f32, color: u32) -> impl IntoElement {
    let track = pal().track;
    let width = if size >= 40.0 { 5.0 } else if size >= 22.0 { 2.5 } else { 2.0 };
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let center = bounds.center();
            let r = f32::from(bounds.size.width.min(bounds.size.height)) / 2.0 - width / 2.0 - 0.5;
            let arc = |from: f32, to: f32| {
                let mut pb = PathBuilder::stroke(px(width));
                let steps = ((to - from).abs() * 40.0).ceil().max(2.0) as usize;
                for i in 0..=steps {
                    let a = from + (to - from) * i as f32 / steps as f32;
                    let p = center + point(px(r * a.cos()), px(r * a.sin()));
                    if i == 0 { pb.move_to(p) } else { pb.line_to(p) }
                }
                pb.build().ok()
            };
            let top = -std::f32::consts::FRAC_PI_2;
            if let Some(path) = arc(top, top + std::f32::consts::TAU) {
                window.paint_path(path, Background::from(rgb(track)));
            }
            let share = share.clamp(0.0, 1.0);
            if share > 0.0 {
                if let Some(path) = arc(top, top + std::f32::consts::TAU * share) {
                    window.paint_path(path, Background::from(rgb(color)));
                }
            }
        },
    )
    .flex_none()
    .size(px(size))
}

/// A bar that fills as `share` goes from 0 to 1.
pub(super) fn bar(share: f32, h: f32, color: u32) -> gpui_kit::Div {
    div().h(px(h)).w_full().rounded(px(h / 2.0)).bg(rgb(pal().track)).child(div().h_full().rounded(px(h / 2.0)).w(gpui_kit::relative(share.clamp(0.0, 1.0))).bg(rgb(color)))
}

/// A tick box, filled when ticked.
fn tick_box(on: bool, dim: bool) -> gpui_kit::Div {
    div()
        .flex_none()
        .size(px(15.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(3.))
        .map(|d| if on { d.bg(rgb(pal().text)).text_color(rgb(pal().paper)).child(icon("check").size(px(12.))) } else { d.border_1().border_color(rgb(pal().faint)) })
        .when(dim, |d| d.opacity(0.4))
}

impl Workbench {
    /// The recognizers loaded again, to hear with what is here now.
    fn packs_reload_ear(&mut self) {
        let saved = &self.form.saved;
        self.pipeline.set_speech(Speech { helper: self.speech_helper.clone(), system: saved.system_speech, language: saved.speech_language.clone() });
        self.hearing = None;
    }

    /// Start downloading what is wanted and not here.
    fn packs_start_job(&mut self) {
        let todo = self.packs.to_fetch();
        if todo.is_empty() || self.packs.job.is_some() {
            return;
        }
        let (tx, events) = channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let (places, prefer, stop) = (self.packs.places.clone(), self.packs.prefer.take(), cancel.clone());
        let total: Vec<(Pack, u64, u64)> = todo.iter().map(|p| (*p, 0, p.size())).collect();
        std::thread::spawn(move || {
            voice::fetch(&todo, &places, prefer, &stop, &mut |event| {
                let _ = tx.send(event);
            });
        });
        alcyo_diag::event("voice_packs.download", serde_json::json!({ "packs": self.packs.to_fetch().iter().map(|p| p.id()).collect::<Vec<_>>() }));
        self.packs.got = None;
        self.packs.failed = None;
        self.packs.paused = false;
        self.packs.job = Some(Job { events, cancel, got: Got { packs: total, checking: true, ..Default::default() } });
    }

    /// Download `packs`, with what they come with, and keep them from now on.
    pub(super) fn packs_download(&mut self, packs: &[Pack], cx: &mut Context<Self>) {
        self.packs.choice.want(packs);
        self.packs.choice.decided = true;
        self.packs.save();
        self.packs.failed = None;
        self.packs.paused = false;
        // A download under way goes on; what was added is fetched after it, by the next look.
        self.packs_start_job();
        cx.notify();
    }

    pub(super) fn packs_pause(&mut self, cx: &mut Context<Self>) {
        if let Some(job) = &self.packs.job {
            job.cancel.store(true, Ordering::Relaxed);
        }
        self.packs.paused = true;
        cx.notify();
    }

    /// Carry on after a pause or a failure, from where it stopped; `prefer` names the source to try first.
    pub(super) fn packs_retry(&mut self, prefer: Option<&'static str>, cx: &mut Context<Self>) {
        self.packs.paused = false;
        self.packs.failed = None;
        self.packs.prefer = prefer;
        self.packs_start_job();
        cx.notify();
    }

    /// Stop the download and no longer want what had not arrived; what did arrive is kept.
    pub(super) fn packs_cancel(&mut self, cx: &mut Context<Self>) {
        if let Some(job) = self.packs.job.take() {
            job.cancel.store(true, Ordering::Relaxed);
        }
        for pack in self.packs.to_fetch() {
            self.packs.choice.unwant(pack);
        }
        self.packs.save();
        self.packs.got = None;
        self.packs.failed = None;
        self.packs.paused = false;
        self.packs.details = false;
        self.storage_changed(cx);
    }

    /// Delete a pack (Chinese takes its live captions with it) and stop keeping it.
    pub(super) fn packs_delete(&mut self, pack: Pack, cx: &mut Context<Self>) {
        // Its download stops first, so no file is written while it goes.
        if let Some(job) = self.packs.job.take() {
            job.cancel.store(true, Ordering::Relaxed);
        }
        self.packs.choice.unwant(pack);
        self.packs.save();
        if let Err(e) = self.packs.places.delete(pack) {
            self.notice = Some(format!("{e:#}"));
        }
        self.packs.refresh();
        self.packs.got = None;
        if self.pipeline.listening() {
            self.packs.reload = true;
        } else {
            self.packs_reload_ear();
        }
        self.storage_changed(cx);
    }

    /// The first run's question answered without downloading: Later, or Apple's recognizer where there is one.
    pub(super) fn packs_skip(&mut self, cx: &mut Context<Self>) {
        self.packs.asking = false;
        self.packs.choice.decided = true;
        self.packs.save();
        if self.speech_helper.is_some() && !self.form.saved.system_speech {
            self.choose_speech(true, None, cx);
        }
        cx.notify();
    }

    /// The first run's question answered: download what is ticked, and show how it goes.
    pub(super) fn packs_first_download(&mut self, cx: &mut Context<Self>) {
        let ticks = voice::closed(&self.packs.ticks);
        if ticks.is_empty() {
            return;
        }
        self.packs.showing_progress = true;
        // The packs are what was chosen to hear with; Apple's recognizer listens only until they are here.
        if self.form.saved.system_speech {
            self.choose_speech(false, None, cx);
        }
        self.packs_download(&ticks, cx);
    }

    /// Tick or untick a pack on the first run: Chinese brings its live captions.
    pub(super) fn packs_tick(&mut self, pack: Pack, cx: &mut Context<Self>) {
        let ticks = &mut self.packs.ticks;
        if let Some(i) = ticks.iter().position(|p| *p == pack) {
            ticks.remove(i);
        } else {
            ticks.push(pack);
            if pack == Pack::Zh && !ticks.contains(&Pack::Live) {
                ticks.push(Pack::Live);
            }
            if pack == Pack::Live && !ticks.contains(&Pack::Zh) {
                ticks.push(Pack::Zh);
            }
        }
        cx.notify();
    }

    /// The first run is over on the desktop page: on into the app, the download going on.
    pub(super) fn packs_first_done(&mut self, cx: &mut Context<Self>) {
        self.packs.asking = false;
        self.packs.showing_progress = false;
        cx.notify();
    }

    /// Before listening starts: the recognizers loaded again if a pack came meanwhile, and whether anything can hear.
    /// Where nothing can (no pack, and no recognizer of the system's), listening does not start and a note says why;
    /// pressed again while it waits to start by itself, it no longer waits.
    pub(super) fn packs_before_listening(&mut self) -> bool {
        if self.packs.armed {
            self.packs.armed = false;
            self.packs.note = None;
            return false;
        }
        if std::mem::take(&mut self.packs.reload) {
            self.packs_reload_ear();
        }
        let apple = self.speech_helper.is_some();
        if self.packs.hears() || apple && self.form.saved.system_speech {
            return true;
        }
        if apple {
            // Apple's recognizer hears meanwhile; said while the packs come.
            if self.packs.under_way() {
                self.packs.note = Some((Note::Apple, Instant::now()));
            }
            return true;
        }
        let note = if self.packs.failed.is_some() {
            Note::Stopped
        } else if self.packs.under_way() || !self.packs.to_fetch().is_empty() {
            Note::Waiting
        } else {
            Note::NoneYet
        };
        self.packs.note = Some((note, Instant::now()));
        false
    }

    /// Follow the download, and start what is wanted and not here (an update, a download an earlier run left).
    pub(super) fn pump_packs(&mut self, changed: &mut bool) {
        if self.packs.job.is_none() && !self.packs.paused && self.packs.failed.is_none() && self.packs.choice.decided && !self.packs.to_fetch().is_empty() {
            self.packs_start_job();
            *changed = true;
        }
        let mut events = vec![];
        if let Some(job) = &self.packs.job {
            while let Ok(event) = job.events.try_recv() {
                events.push(event);
            }
        }
        for event in events {
            *changed = true;
            match event {
                voice::Event::Progress(got) => {
                    if let Some(job) = &mut self.packs.job {
                        job.got = got;
                    }
                }
                voice::Event::Ready(pack) => {
                    self.packs.refresh();
                    alcyo_diag::event("voice_packs.ready", serde_json::json!({ "pack": pack.id() }));
                    let listening = self.pipeline.listening();
                    // Never switched mid-meeting: it hears from the next time listening starts.
                    if listening {
                        self.packs.reload = true;
                        if self.packs.note.is_some_and(|(n, _)| n == Note::Apple) || self.speech_helper.is_some() && self.packs.hears() {
                            self.packs.note = Some((Note::ReadyNext, Instant::now()));
                        }
                    } else {
                        self.packs_reload_ear();
                    }
                    if self.packs.armed && self.packs.hears() && !listening {
                        self.packs.armed = false;
                        self.packs.note = Some((Note::ReadyListening, Instant::now()));
                        if self.editable() {
                            self.toggle_mic();
                        }
                    }
                }
                voice::Event::Failed(error) => {
                    eprintln!("voice packs: {error}");
                    alcyo_diag::event("voice_packs.failed", serde_json::json!({ "error": error }));
                    self.packs.got = self.packs.job.take().map(|j| j.got);
                    // Paused by the person: not a failure.
                    if !self.packs.paused {
                        self.packs.failed = Some(error);
                    }
                }
                voice::Event::Cancelled => {
                    self.packs.got = self.packs.job.take().map(|j| j.got);
                }
                voice::Event::Done => {
                    self.packs.got = self.packs.job.take().map(|j| j.got);
                    self.packs.refresh();
                    self.packs.ready_at = Some(Instant::now());
                    self.packs.restored = !self.packs.choice.restoring.is_empty();
                    if self.packs.restored {
                        self.packs.choice.restoring.clear();
                        self.packs.save();
                    }
                    self.storage.changed();
                }
            }
        }
        // Notes go by themselves, all but those waiting on something.
        if self.packs.note.is_some_and(|(n, at)| !matches!(n, Note::Waiting | Note::Stopped | Note::NoneYet) && at.elapsed() > NOTE_FOR) {
            self.packs.note = None;
            *changed = true;
        }
        // While armed, the note goes once the button says it.
        if self.packs.armed && self.packs.note.is_some_and(|(n, _)| n == Note::Waiting) {
            self.packs.note = None;
            *changed = true;
        }
        if self.packs.ready_at.is_some_and(|at| at.elapsed() > READY_FOR) {
            self.packs.ready_at = None;
            self.packs.restored = false;
            *changed = true;
        }
        if self.packs.told_at.is_some_and(|at| at.elapsed() > TOLD_FOR) {
            self.packs.told_at = None;
            *changed = true;
        }
    }

    /// Note that the word about packs coming back after an update is shown, once.
    fn packs_tell(&mut self) -> bool {
        if self.packs.choice.restoring.is_empty() {
            return false;
        }
        if !self.packs.choice.told {
            self.packs.choice.told = true;
            self.packs.save();
            self.packs.told_at = Some(Instant::now());
        }
        self.packs.told_at.is_some()
    }

    /// The packs being brought back, by name, and their size.
    fn packs_restoring_said(&self) -> (String, String) {
        let list = &self.packs.choice.restoring;
        let names = list.iter().map(|p| pack_name(*p)).collect::<Vec<_>>().join(if crate::i18n::chinese() { "、" } else { ", " });
        (names, mb(list.iter().map(|p| p.size()).sum()))
    }

    // ---------- the desktop's first run ----------

    /// The first run's page on the desktop, while the question is asked: in place of everything else.
    pub(super) fn packs_first_page(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.packs.asking {
            return None;
        }
        let body = if self.packs.showing_progress { self.packs_first_progress(cx) } else { self.packs_first_question(cx) };
        let bar = title_bar().child(div().pl_3().text_ui().font_weight(FontWeight::MEDIUM).child("Alcyo"));
        Some(
            div()
                .id("voice-packs-first")
                .size_full()
                .flex()
                .flex_col()
                .font(theme::interface_font())
                .bg(rgb(pal().paper))
                .text_color(rgb(pal().text))
                .child(bar)
                .child(
                    div()
                        .id("voice-packs-first-body")
                        .relative()
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scroll()
                        .child(div().absolute().size_full().child(ground_layer(&self.ground)))
                        .child(div().relative().flex().justify_center().py(px(56.)).px_6().child(div().w(px(560.)).max_w_full().flex().flex_col().child(body))),
                )
                .into_any_element(),
        )
    }

    fn packs_first_question(&mut self, cx: &mut Context<Self>) -> gpui_kit::Div {
        let p = pal();
        let ticks = voice::closed(&self.packs.ticks);
        let on = |pack: Pack| ticks.contains(&pack);
        let need = self.packs.places.missing_bytes(&ticks);
        let free = self.packs.free();
        let low = !self.packs.fits(need);
        let apple = self.speech_helper.is_some();
        let card = |pack: Pack, cx: &mut Context<Self>| {
            let ticked = on(pack);
            div()
                .id(ElementId::Name(format!("vp-card-{}", pack.id()).into()))
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(4.))
                .p(px(12.))
                .rounded(px(3.))
                .bg(rgb(p.surface))
                .cursor_pointer()
                .map(|d| if ticked { d.border_2().border_color(rgb(p.text)).p(px(11.)) } else { d.border_1().border_color(rgb(p.line)) })
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(tick_box(ticked, false))
                        .child(div().flex_1().min_w_0().text_size(px(16.)).font_weight(FontWeight::MEDIUM).child(pack_name(pack)))
                        .child(div().font_family(theme::MONO).text_meta().text_color(rgb(p.faint)).child(mb(pack.size()))),
                )
                .child(div().text_small().font_weight(FontWeight::MEDIUM).child(pack_tag(pack)))
                .child(div().text_small().text_color(rgb(p.muted)).line_height(gpui_kit::relative(1.4)).child(pack_hint(pack)))
                .on_click(cx.listener(move |this, _, _, cx| this.packs_tick(pack, cx)))
                .when(pack == Pack::Zh, |d| {
                    let live = on(Pack::Live);
                    d.child(
                        div()
                            .id("vp-card-live")
                            .mt_2()
                            .pt_2()
                            .border_t_1()
                            .border_color(rgb(p.line))
                            .flex()
                            .items_start()
                            .gap_2()
                            .when(!on(Pack::Zh), |d| d.opacity(0.45))
                            .child(div().pt(px(1.)).child(tick_box(live, false)))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .flex()
                                    .flex_col()
                                    .gap(px(2.))
                                    .child(div().flex().gap_2().text_small().child(pack_name(Pack::Live)).child(div().font_family(theme::MONO).text_meta().text_color(rgb(p.faint)).child(format!("+{}", mb(Pack::Live.size())))))
                                    .child(div().text_meta().text_color(rgb(p.muted)).child(pack_hint(Pack::Live))),
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                cx.stop_propagation();
                                this.packs_tick(Pack::Live, cx);
                            })),
                    )
                })
        };
        let cards = div().flex().gap_2().child(card(Pack::Zh, cx)).child(card(Pack::En, cx)).child(card(Pack::Ja, cx));
        let speakers = div()
            .flex()
            .items_center()
            .gap_3()
            .px_4()
            .py_3()
            .rounded(px(3.))
            .border_1()
            .border_color(rgb(p.line))
            .bg(rgb(p.surface))
            .child(icon("people").size(px(15.)).text_color(rgb(p.muted)))
            .child(div().flex_1().min_w_0().flex().flex_col().gap(px(2.)).child(div().text_ui().child(pack_name(Pack::Spk))).child(div().text_small().text_color(rgb(p.muted)).child(pack_hint(Pack::Spk))))
            .child(div().font_family(theme::MONO).text_meta().text_color(rgb(p.faint)).child(mb(Pack::Spk.size())))
            .child(Switch::new("vp-first-spk").checked(on(Pack::Spk)).accessibility_label(pack_name(Pack::Spk)).on_click(cx.listener(|this, _, _, cx| this.packs_tick(Pack::Spk, cx))));
        let search = mb(alcyo_models::embedding().size);
        let total = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(2.))
            .child(div().text_small().text_color(rgb(if low { p.danger } else { p.text })).child(match free {
                Some(free) => format!("{} · {}", ta!("vp-need", size = mb(need)), ta!("vp-free", size = mb(free))),
                None => ta!("vp-need", size = mb(need)),
            }))
            .child(div().text_meta().text_color(rgb(p.faint)).child(ta!("vp-bundled", size = search)));
        let nothing = ticks.is_empty();
        let later = if apple {
            Quiet::new("vp-first-apple").outline().label(t("vp-apple-use")).on_click(cx.listener(|this, _, _, cx| this.packs_skip(cx)))
        } else {
            Quiet::new("vp-first-later").outline().label(t("vp-later")).on_click(cx.listener(|this, _, _, cx| this.packs_skip(cx)))
        };
        let download = Quiet::new("vp-first-download")
            .solid()
            .label(if nothing { t("vp-download").to_string() } else { ta!("vp-download-size", size = mb(need)) })
            .disabled(low || nothing)
            .on_click(cx.listener(|this, _, _, cx| this.packs_first_download(cx)));
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(div().font_family(theme::MONO).text_label().text_color(rgb(p.faint)).child(t("vp-kicker").to_uppercase()))
            .child(div().mt(px(-8.)).text_size(px(22.)).font_weight(FontWeight::MEDIUM).child(t("vp-title")))
            .child(div().text_ui().text_color(rgb(p.muted)).line_height(gpui_kit::relative(1.5)).child(t("vp-lead")))
            .child(cards)
            .child(speakers)
            .when(apple, |d| {
                d.child(div().flex().items_start().gap_2().px_3().py(px(10.)).rounded(px(3.)).bg(rgb(p.chrome)).text_small().text_color(rgb(p.muted)).child(icon("apple").size(px(14.)).text_color(rgb(p.text))).child(div().flex_1().min_w_0().child(t("vp-apple-now"))))
            })
            .when(low, |d| d.child(div().text_small().text_color(rgb(p.danger)).child(ta!("vp-low-space", need = mb(need), free = mb(free.unwrap_or(0))))))
            .child(div().flex().items_center().gap_2().child(total).child(later).child(download))
    }

    /// The first run's page once Download was pressed: how far it is, and on into the app.
    fn packs_first_progress(&mut self, cx: &mut Context<Self>) -> gpui_kit::Div {
        let p = pal();
        let done = !self.packs.under_way() && self.packs.failed.is_none();
        let apple = self.speech_helper.is_some();
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(div().font_family(theme::MONO).text_label().text_color(rgb(p.faint)).child(t("vp-kicker").to_uppercase()))
            .child(div().mt(px(-8.)).text_size(px(22.)).font_weight(FontWeight::MEDIUM).child(if done {
                t("vp-all-ready")
            } else if self.packs.failed.is_some() {
                t("vp-stopped")
            } else {
                t("vp-downloading")
            }))
            .child(div().text_ui().text_color(rgb(p.muted)).line_height(gpui_kit::relative(1.5)).child(if done {
                t("vp-all-ready-hint")
            } else if apple {
                t("vp-carry-on")
            } else {
                t("vp-carry-on-plain")
            }))
            .child(div().rounded(px(3.)).border_1().border_color(rgb(p.line)).bg(rgb(p.surface)).child(self.packs_progress_body(true, cx)))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().flex_1().min_w_0().text_meta().text_color(rgb(p.faint)).when(!done && self.packs.failed.is_none(), |d| d.child(t("vp-resume-note"))))
                    .when(!done, |d| d.children(self.packs_controls("vp-first", cx)))
                    .child(Quiet::new("vp-first-continue").solid().label(if done { t("vp-done") } else { t("vp-continue") }).on_click(cx.listener(|this, _, _, cx| this.packs_first_done(cx)))),
            )
    }

    /// Pause or Resume and Cancel, or Retry and ModelScope after a failure.
    fn packs_controls(&self, id: &'static str, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let name = |what: &str| ElementId::Name(format!("{id}-{what}").into());
        if self.packs.failed.is_some() {
            return vec![
                Quiet::new(name("retry")).solid().icon("refresh", 13.).label(t("vp-retry")).on_click(cx.listener(|this, _, _, cx| this.packs_retry(None, cx))).into_any_element(),
                Quiet::new(name("modelscope")).label(t("vp-try-modelscope")).on_click(cx.listener(|this, _, _, cx| this.packs_retry(Some("ModelScope"), cx))).into_any_element(),
            ];
        }
        let paused = self.packs.paused;
        vec![
            Quiet::new(name("cancel")).label(t("vp-cancel")).on_click(cx.listener(|this, _, _, cx| this.packs_cancel(cx))).into_any_element(),
            Quiet::new(name("pause"))
                .outline()
                .label(if paused { t("vp-resume") } else { t("vp-pause") })
                .on_click(cx.listener(move |this, _, _, cx| if paused { this.packs_retry(None, cx) } else { this.packs_pause(cx) }))
                .into_any_element(),
        ]
    }

    /// The whole download: how much of how much, a bar, where from, then each pack.
    fn packs_progress_body(&self, big: bool, _cx: &mut Context<Self>) -> gpui_kit::Div {
        let p = pal();
        let got = self.packs.got().cloned().unwrap_or_default();
        let (failed, paused) = (self.packs.failed.is_some(), self.packs.paused);
        let done = !self.packs.under_way() && !failed;
        let tone = if failed { p.warning } else if paused { p.faint } else { p.text };
        let head = div()
            .flex()
            .items_baseline()
            .gap_2()
            .child(div().text_size(px(if big { 17. } else { 14. })).font_weight(FontWeight::MEDIUM).child(if done { mb(got.total()) } else { ta!("vp-of", done = mb(got.done()), total = mb(got.total())) }))
            .child(div().flex_1().text_small().text_color(rgb(p.muted)).child(if done {
                String::new()
            } else if paused {
                t("vp-paused").to_string()
            } else if got.checking && got.speed == 0 {
                t("vp-checking").to_string()
            } else {
                time_left(&got).map(|time| ta!("vp-left", time = time)).unwrap_or_default()
            }))
            .when_some(speed(&got).filter(|_| !done && !paused && !failed), |d, s| d.child(div().font_family(theme::MONO).text_meta().text_color(rgb(p.faint)).child(s)))
            .when(done, |d| d.child(icon("check").size(px(15.)).text_color(rgb(0x1f_8a_4c))));
        let source = div().flex().items_start().gap_2().text_small().text_color(rgb(if failed { p.warning } else { p.muted })).map(|d| {
            if failed {
                d.child(icon("cloud-off").size(px(14.))).child(div().flex_1().child(t("vp-unreachable")))
            } else if done {
                d.child(icon("cloud-off").size(px(0.))).child(t("vp-checked"))
            } else {
                let from = got.source.as_deref().map(source_name).unwrap_or_else(|| "Cloudflare · models.alcyo.app".to_string());
                d.child(ta!("vp-from", source = from)).child(div().text_color(rgb(p.faint)).child(format!("· {}", t("vp-fallback"))))
            }
        });
        let rows = got.packs.iter().map(|(pack, here, total)| {
            let share = if *total == 0 { 1.0 } else { *here as f32 / *total as f32 };
            let state = if here >= total {
                div().text_color(rgb(0x1f_8a_4c)).child(t("vp-ready"))
            } else if *here > 0 {
                div().text_color(rgb(if failed { p.warning } else { p.muted })).child(if failed {
                    t("vp-stopped-short").to_string()
                } else if paused {
                    t("vp-paused").to_string()
                } else {
                    format!("{} / {}", mb(*here), mb(*total))
                })
            } else {
                div().text_color(rgb(p.faint)).child(t("vp-waiting"))
            };
            div()
                .flex()
                .items_center()
                .gap_3()
                .px_4()
                .py(px(9.))
                .border_t_1()
                .border_color(rgb(p.line))
                .child(div().w(px(if big { 150. } else { 116. })).flex_none().flex().items_baseline().gap_2().text_small().child(pack_name(*pack)).child(div().font_family(theme::MONO).text_meta().text_color(rgb(p.faint)).child(mb(pack.size()))))
                .child(div().flex_1().child(bar(share, 3., if here >= total { 0x1f_8a_4c } else { tone })))
                .child(div().w(px(if big { 120. } else { 100. })).flex_none().flex().justify_end().text_small().child(state))
        });
        div()
            .flex()
            .flex_col()
            .child(div().flex().flex_col().gap_2().px_4().py_3().child(head).child(bar(if done { 1.0 } else { got.share() }, 4., tone)).child(source))
            .children(rows)
    }

    // ---------- the desktop's sidebar row and popover ----------

    /// The sidebar's row while packs download, are paused or stopped, or have just arrived; with its popover, and
    /// the one-time bubble after an update.
    pub(super) fn packs_side_row(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let p = pal();
        let restoring = !self.packs.choice.restoring.is_empty();
        let ready = self.packs.ready_at.is_some() && !self.packs.under_way();
        if !self.packs.under_way() && !ready && self.packs.failed.is_none() {
            return None;
        }
        let failed = self.packs.failed.is_some();
        let share = self.packs.got().map_or(0.0, Got::share);
        let row = div()
            .id("vp-side-row")
            .flex()
            .flex_none()
            .items_center()
            .gap_2()
            .h(px(30.))
            .px_2()
            .rounded(px(2.))
            .cursor_pointer()
            .text_ui()
            .hover(|d| d.bg(rgb(p.hover)))
            .when(self.packs.details, |d| d.bg(rgb(p.selected)))
            .map(|d| {
                if failed {
                    d.text_color(rgb(p.warning))
                        .child(icon("cloud-off").size(px(14.)))
                        .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().child(t("vp-row-stopped")))
                        .child(div().id("vp-side-retry").text_small().text_color(rgb(p.accent)).child(t("vp-retry")).on_click(cx.listener(|this, _, _, cx| {
                            cx.stop_propagation();
                            this.packs_retry(None, cx);
                        })))
                } else if ready {
                    d.child(icon("check").size(px(14.)).text_color(rgb(0x1f_8a_4c))).child(div().flex_1().min_w_0().child(if self.packs.restored { t("vp-back-done-row") } else { t("vp-all-ready") }))
                } else {
                    d.child(ring(share, 15., if self.packs.paused { p.faint } else { p.text }))
                        .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().child(if restoring { t("vp-back-row") } else { t("vp-packs") }))
                        .child(div().font_family(theme::MONO).text_meta().text_color(rgb(p.faint)).child(if self.packs.paused { t("vp-paused").to_lowercase() } else { format!("{}%", self.packs.percent()) }))
                }
            })
            .on_click(cx.listener(|this, _, _, cx| {
                this.packs.details = !this.packs.details;
                cx.notify();
            }));
        let telling = !ready && self.packs_tell();
        let pop = (self.packs.details && !ready).then(|| self.packs_popover(cx));
        let bubble = (telling && !self.packs.details).then(|| {
            let (names, size) = self.packs_restoring_said();
            div()
                .occlude()
                .w(px(300.))
                .flex()
                .flex_col()
                .gap_1()
                .p_3()
                .bg(rgb(p.raised))
                .border_1()
                .border_color(rgb(p.raised_line))
                .rounded(px(3.))
                .shadow(theme::shadow(theme::Lift::Float))
                .text_small()
                .child(div().font_weight(FontWeight::MEDIUM).child(t("vp-back-title")))
                .child(div().text_color(rgb(p.muted)).line_height(gpui_kit::relative(1.45)).child(ta!("vp-back-why", packs = names, size = size)))
        });
        let floating = pop.or(bubble.map(IntoElement::into_any_element));
        Some(
            div()
                .relative()
                .child(row)
                .when_some(floating, |d, card| {
                    d.child(gpui_kit::deferred(gpui_kit::anchored().anchor(gpui_kit::Anchor::BottomLeft).offset(point(px(SIDEBAR_W - 8.), px(30.))).snap_to_window_with_margin(px(8.)).child(card)).with_priority(2))
                })
                .into_any_element(),
        )
    }

    fn packs_popover(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let p = pal();
        let failed = self.packs.failed.is_some();
        div()
            .id("vp-popover")
            .occlude()
            .w(px(440.))
            .flex()
            .flex_col()
            .bg(rgb(p.raised))
            .border_1()
            .border_color(rgb(p.raised_line))
            .rounded(px(3.))
            .shadow(theme::shadow(theme::Lift::Float))
            .child(div().px_4().pt_3().text_ui().font_weight(FontWeight::MEDIUM).child(if failed { t("vp-stopped") } else { t("vp-downloading") }))
            .child(self.packs_progress_body(false, cx))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .border_t_1()
                    .border_color(rgb(p.line))
                    .children(self.packs_controls("vp-pop", cx))
                    .child(div().flex_1())
                    .child(div().id("vp-pop-settings").text_small().text_color(rgb(p.accent)).cursor_pointer().child(t("vp-speech-settings")).on_click(cx.listener(|this, _, _, cx| {
                        this.packs.details = false;
                        this.settings_tab = SettingsTab::Speech;
                        if this.panel != Panel::Settings {
                            this.toggle_panel(Panel::Settings, cx);
                        }
                        cx.notify();
                    }))),
            )
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                this.packs.details = false;
                cx.notify();
            }))
            .into_any_element()
    }

    // ---------- the note over the dock ----------

    /// The note over the dock about listening while the packs are not here yet, on the desktop and the iPad.
    pub(super) fn packs_dock_note(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (note, _) = self.packs.note?;
        let p = pal();
        let ipad = crate::platform::ipad_layout();
        let percent = self.packs.percent();
        let share = self.packs.got().map_or(0.0, Got::share);
        let ok = || icon("check").size(px(16.)).text_color(rgb(0x1f_8a_4c));
        let (lead, title, text): (AnyElement, String, Option<String>) = match note {
            Note::Waiting => (ring(share, 28., p.text).into_any_element(), ta!("vp-wait-note", percent = percent), Some(t("vp-wait-hint").to_string())),
            Note::Apple => (icon("apple").size(px(16.)).into_any_element(), t("vp-apple-listening").to_string(), Some(ta!("vp-apple-until", percent = percent))),
            Note::ReadyListening => (ok().into_any_element(), t("vp-ready-listening").to_string(), None),
            Note::ReadyNext => (ok().into_any_element(), t("vp-all-ready").to_string(), Some(t("vp-ready-next").to_string())),
            Note::Stopped => (icon("cloud-off").size(px(16.)).text_color(rgb(p.warning)).into_any_element(), ta!("vp-stopped-at", percent = percent), Some(ta!("vp-stopped-hint", percent = percent))),
            Note::NoneYet => (icon("download").size(px(16.)).into_any_element(), t("vp-none-yet").to_string(), Some(t("vp-none-yet-hint").to_string())),
        };
        let actions: Vec<AnyElement> = match note {
            Note::Waiting => vec![
                Quiet::new("vp-arm").solid().label(t("vp-start-when-ready")).on_click(cx.listener(|this, _, _, cx| {
                    this.packs.armed = true;
                    this.packs.note = None;
                    cx.notify();
                })).into_any_element(),
                Quiet::new("vp-note-dismiss").label(t("vp-not-now")).on_click(cx.listener(|this, _, _, cx| {
                    this.packs.note = None;
                    cx.notify();
                })).into_any_element(),
            ],
            Note::Stopped => vec![
                Quiet::new("vp-note-retry").solid().icon("refresh", 13.).label(t("vp-retry")).on_click(cx.listener(|this, _, _, cx| {
                    this.packs.note = None;
                    this.packs_retry(None, cx);
                })).into_any_element(),
                Quiet::new("vp-note-dismiss").label(t("vp-not-now")).on_click(cx.listener(|this, _, _, cx| {
                    this.packs.note = None;
                    cx.notify();
                })).into_any_element(),
            ],
            Note::NoneYet => vec![
                Quiet::new("vp-note-choose").solid().label(t("vp-choose")).on_click(cx.listener(|this, _, _, cx| {
                    this.packs.note = None;
                    this.packs_open_settings(cx);
                })).into_any_element(),
                Quiet::new("vp-note-dismiss").label(t("vp-not-now")).on_click(cx.listener(|this, _, _, cx| {
                    this.packs.note = None;
                    cx.notify();
                })).into_any_element(),
            ],
            _ => vec![],
        };
        let words = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(2.))
            .child(div().text_ui().font_weight(FontWeight::MEDIUM).child(title))
            .when_some(text, |d, text| d.child(div().text_small().text_color(rgb(p.muted)).line_height(gpui_kit::relative(1.4)).child(text)))
            .when(!actions.is_empty(), |d| d.child(div().flex().gap_2().pt_2().children(actions)));
        let card = div().id("vp-note").occlude().flex().items_start().gap_3().max_w(px(460.)).px(px(14.)).py(px(11.)).child(div().pt(px(1.)).child(lead)).child(words);
        let card = if ipad {
            ipad::glass(card.rounded(px(18.)), false).cursor_pointer().on_click(cx.listener(|this, _, _, cx| {
                this.packs.details = true;
                cx.notify();
            }))
        } else {
            card.bg(rgb(p.raised)).border_1().border_color(rgb(p.raised_line)).rounded(px(3.)).shadow(theme::shadow(theme::Lift::Float))
        };
        Some(card.into_any_element())
    }

    pub(super) fn packs_open_settings(&mut self, cx: &mut Context<Self>) {
        self.settings_tab = SettingsTab::Speech;
        if self.panel != Panel::Settings {
            self.toggle_panel(Panel::Settings, cx);
        }
        cx.notify();
    }

    // ---------- Settings › Speech on the desktop ----------

    /// A pack's row in Settings › Speech: what it is, and download, delete or how far it is.
    fn packs_settings_row(&self, pack: Pack, cx: &mut Context<Self>) -> AnyElement {
        let p = pal();
        let status = self.packs.status(pack);
        let got = self.packs.job.as_ref().map(|j| &j.got).and_then(|g| g.packs.iter().find(|(q, ..)| *q == pack).copied());
        let sub = pack == Pack::Live;
        let dim = sub && !self.packs.usable(Pack::Zh) && got.is_none();
        let id = |what: &str| ElementId::Name(format!("vp-set-{what}-{}", pack.id()).into());
        let right: AnyElement = match (status, got) {
            (_, Some((_, here, total))) if here < total => div()
                .flex()
                .items_center()
                .gap_2()
                .child(div().w(px(170.)).flex().flex_col().gap_1().child(bar(here as f32 / total.max(1) as f32, 3., p.text)).child(div().font_family(theme::MONO).text_meta().text_color(rgb(p.faint)).child(format!("{} / {}{}", mb(here), mb(total), speed(self.packs.got().unwrap()).map(|s| format!(" · {s}")).unwrap_or_default()))))
                .child(Quiet::new(id("pause")).label(t("vp-pause")).on_click(cx.listener(|this, _, _, cx| this.packs_pause(cx))))
                .into_any_element(),
            (Status::BuiltIn, _) => div().flex().items_center().gap_3().child(div().font_family(theme::MONO).text_meta().text_color(rgb(p.faint)).child(mb(pack.size()))).child(div().text_small().text_color(rgb(p.faint)).child(t("vp-built-in"))).into_any_element(),
            (Status::Downloaded | Status::Outdated, _) => div()
                .flex()
                .items_center()
                .gap_3()
                .child(div().font_family(theme::MONO).text_meta().text_color(rgb(p.faint)).child(mb(pack.size())))
                .when(status == Status::Outdated, |d| d.child(div().text_small().text_color(rgb(p.muted)).child(t("vp-updating"))))
                .child(Quiet::new(id("delete")).icon("trash", 13.).label(t("vp-delete")).on_click(cx.listener(move |this, _, _, cx| this.packs_delete(pack, cx))))
                .into_any_element(),
            (Status::Missing, _) => div()
                .flex()
                .items_center()
                .gap_3()
                .child(div().font_family(theme::MONO).text_meta().text_color(rgb(p.faint)).child(mb(pack.size())))
                .child(Quiet::new(id("get")).outline().icon("download", 13.).label(t("vp-download")).on_click(cx.listener(move |this, _, _, cx| {
                    let with = if pack == Pack::Live { vec![Pack::Zh, Pack::Live] } else { vec![pack] };
                    this.packs_download(&with, cx);
                })))
                .into_any_element(),
        };
        div()
            .flex()
            .items_center()
            .gap_4()
            .px_4()
            .py_3()
            .when(sub, |d| d.pl(px(36.)))
            .when(dim, |d| d.opacity(0.5))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(div().flex().items_baseline().gap_2().child(div().text_ui().child(pack_name(pack))).when(!sub, |d| d.child(div().text_small().text_color(rgb(p.muted)).child(pack_tag(pack)))))
                    .child(div().text_small().text_color(rgb(p.muted)).child(if sub { format!("{} {}", pack_hint(pack), t("vp-with-zh")) } else { pack_hint(pack).to_string() })),
            )
            .child(right)
            .into_any_element()
    }

    /// Settings › Speech's voice packs: the caption's aside, the rows, and the line under them.
    pub(super) fn packs_settings_section(&mut self, cx: &mut Context<Self>) -> (String, Vec<AnyElement>, AnyElement) {
        let p = pal();
        let here: u64 = Pack::ALL.iter().map(|pack| self.packs.places.on_disk(*pack)).sum();
        let aside = ta!("vp-on-computer", size = mb(here));
        let rows = [Pack::Zh, Pack::Live, Pack::Spk, Pack::En, Pack::Ja].into_iter().map(|pack| self.packs_settings_row(pack, cx)).collect();
        let failed = self.packs.failed.clone();
        let foot = div()
            .flex()
            .flex_col()
            .gap_2()
            .px_1()
            .pt_2()
            .when_some(failed, |d, _| {
                d.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_small()
                        .text_color(rgb(p.warning))
                        .child(icon("cloud-off").size(px(14.)))
                        .child(div().flex_1().child(t("vp-unreachable")))
                        .children(self.packs_controls("vp-set", cx)),
                )
            })
            .child(div().text_small().text_color(rgb(p.muted)).child(format!("{} {}", t("vp-src-note"), ta!("vp-bundled", size = mb(alcyo_models::embedding().size)))))
            .into_any_element();
        (aside, rows, foot)
    }

    // ---------- the iPad ----------

    /// The iPad's first-run step: the language cards, then how the download goes.
    pub(super) fn ipad_packs_step(&mut self, cx: &mut Context<Self>) -> gpui_kit::Div {
        if self.packs.showing_progress {
            return self.ipad_packs_step_progress(cx);
        }
        let p = pal();
        let ticks = voice::closed(&self.packs.ticks);
        let on = |pack: Pack| ticks.contains(&pack);
        let need = self.packs.places.missing_bytes(&ticks);
        let free = self.packs.free();
        let low = !self.packs.fits(need);
        let soft = if theme::is_dark() { 0x16_1c_33 } else { 0xee_f1_fd };
        let card = |pack: Pack, tall: bool, cx: &mut Context<Self>| {
            let ticked = on(pack);
            div()
                .id(ElementId::Name(format!("vp-ipad-card-{}", pack.id()).into()))
                .relative()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(3.))
                .p(px(16.))
                .rounded(px(18.))
                .cursor_pointer()
                .map(|d| if ticked { d.bg(rgb(soft)).border_2().border_color(rgb(p.accent)).p(px(15.)) } else { d.bg(rgb(p.surface)).border_1().border_color(rgb(p.line)) })
                .when(tall, |d| d.h_full())
                .child(
                    div()
                        .absolute()
                        .top(px(14.))
                        .right(px(14.))
                        .size(px(22.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_full()
                        .map(|d| if ticked { d.bg(rgb(p.accent)).text_color(rgb(0xff_ff_ff)).child(icon("ipad/check").size(px(13.))) } else { d.border(px(1.5)).border_color(rgb(if theme::is_dark() { 0x48_48_4a } else { 0xc7_c7_cc })) }),
                )
                .child(div().text_size(px(20.)).font_weight(FontWeight::BOLD).child(pack_name(pack)))
                .child(div().text_size(px(12.5)).font_weight(FontWeight::MEDIUM).child(pack_tag(pack)))
                .child(div().pr(px(24.)).text_size(px(13.)).line_height(gpui_kit::relative(1.4)).text_color(rgb(p.muted)).child(pack_hint(pack)))
                .when(tall, |d| d.child(div().flex_1()))
                .child(div().pt_1().font_family(theme::MONO).text_size(px(11.)).text_color(rgb(p.faint)).child(mb(pack.size())))
                .on_click(cx.listener(move |this, _, _, cx| this.packs_tick(pack, cx)))
                .when(pack == Pack::Zh, |d| {
                    d.child(
                        div()
                            .id("vp-ipad-live")
                            .mt_2()
                            .pt_2()
                            .border_t_1()
                            .border_color(rgb(p.line))
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .when(!on(Pack::Zh), |d| d.opacity(0.45))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(icon("ipad/cc").size(px(16.)).text_color(rgb(p.accent)))
                                    .child(div().flex_1().text_size(px(13.5)).font_weight(FontWeight::MEDIUM).child(pack_name(Pack::Live)))
                                    .child(div().font_family(theme::MONO).text_size(px(10.5)).text_color(rgb(p.faint)).child(format!("+{}", mb(Pack::Live.size()))))
                                    .child(sheet::switch("vp-ipad-live-switch", on(Pack::Live))),
                            )
                            .child(div().text_size(px(12.)).text_color(rgb(p.faint)).child(pack_hint(Pack::Live)))
                            .on_click(cx.listener(|this, _, _, cx| {
                                cx.stop_propagation();
                                this.packs_tick(Pack::Live, cx);
                            })),
                    )
                })
        };
        let grid = div().flex().gap(px(12.)).child(div().flex_1().flex().child(card(Pack::Zh, true, cx))).child(div().flex_1().flex().flex_col().gap(px(12.)).child(card(Pack::En, false, cx)).child(card(Pack::Ja, false, cx)));
        let speakers = div()
            .id("vp-ipad-spk")
            .flex()
            .items_center()
            .gap(px(12.))
            .px(px(16.))
            .py(px(12.))
            .rounded(px(18.))
            .border_1()
            .border_color(rgb(p.line))
            .bg(rgb(p.surface))
            .cursor_pointer()
            .child(div().size(px(36.)).flex().items_center().justify_center().rounded(px(10.)).bg(rgb(p.chrome)).child(icon("ipad/people").size(px(18.))))
            .child(div().flex_1().min_w_0().flex().flex_col().child(div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child(pack_name(Pack::Spk))).child(div().text_size(px(13.)).text_color(rgb(p.muted)).child(pack_hint(Pack::Spk))))
            .child(div().font_family(theme::MONO).text_size(px(11.)).text_color(rgb(p.faint)).child(mb(Pack::Spk.size())))
            .child(sheet::switch("vp-ipad-spk-switch", on(Pack::Spk)))
            .on_click(cx.listener(|this, _, _, cx| this.packs_tick(Pack::Spk, cx)));
        let nothing = ticks.is_empty();
        let sum = div()
            .flex()
            .gap_2()
            .text_size(px(13.5))
            .text_color(rgb(if low { p.danger } else { p.muted }))
            .child(ta!("vp-need", size = mb(need)))
            .when_some(free, |d, free| d.child("·").child(ta!("vp-free-ipad", size = mb(free))));
        div()
            .flex_1()
            .flex()
            .flex_col()
            .gap(px(16.))
            .child(Self::intro_heading(t("vp-title"), Some(t("vp-lead"))))
            .child(grid)
            .child(speakers)
            .child(div().flex_1())
            .child(sum)
            .when(low, |d| d.child(div().flex().gap_2().text_size(px(13.)).text_color(rgb(p.danger)).child(t("vp-low-space-ipad"))))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(18.))
                    .child(Self::intro_button("vp-ipad-download", &if nothing { t("vp-download").to_string() } else { ta!("vp-download-size", size = mb(need)) }, false).when(low || nothing, |d| d.opacity(0.35)).on_click(cx.listener(move |this, _, _, cx| {
                        if !low && !nothing {
                            this.packs_first_download(cx);
                        }
                    })))
                    .child(Self::intro_link("vp-ipad-skip", t("vp-apple-skip")).on_click(cx.listener(|this, _, _, cx| {
                        this.packs_skip(cx);
                        this.intro_go(2, cx);
                    }))),
            )
            .child(div().flex().items_start().gap_2().text_size(px(12.5)).line_height(gpui_kit::relative(1.45)).text_color(rgb(p.faint)).child(icon("ipad/apple").size(px(13.))).child(div().flex_1().min_w_0().child(t("vp-apple-now"))))
    }

    fn ipad_packs_step_progress(&mut self, cx: &mut Context<Self>) -> gpui_kit::Div {
        let p = pal();
        let done = !self.packs.under_way() && self.packs.failed.is_none();
        let paused = self.packs.paused;
        div()
            .flex_1()
            .flex()
            .flex_col()
            .gap(px(16.))
            .child(Self::intro_heading(if done { t("vp-all-ready") } else { t("vp-getting") }, Some(if done { t("vp-all-ready-hint") } else { t("vp-carry-on") })))
            .child(div().rounded(px(18.)).border_1().border_color(rgb(p.line)).bg(rgb(p.surface)).overflow_hidden().child(self.packs_progress_body(true, cx)))
            .child(div().flex_1())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(18.))
                    .child(Self::intro_button("vp-ipad-continue", if done { t("vp-next") } else { t("vp-continue") }, false).on_click(cx.listener(|this, _, _, cx| {
                        this.packs.showing_progress = false;
                        this.packs.asking = false;
                        this.intro_go(2, cx);
                    })))
                    .when(!done && self.packs.failed.is_none(), |d| {
                        d.child(Self::intro_link("vp-ipad-pause", if paused { t("vp-resume") } else { t("vp-pause") }).on_click(cx.listener(move |this, _, _, cx| if paused { this.packs_retry(None, cx) } else { this.packs_pause(cx) })))
                            .child(Self::intro_link("vp-ipad-cancel", t("vp-cancel")).text_color(rgb(p.faint)).on_click(cx.listener(|this, _, _, cx| {
                                this.packs_cancel(cx);
                                this.packs.showing_progress = false;
                            })))
                    })
                    .when(self.packs.failed.is_some(), |d| d.child(Self::intro_link("vp-ipad-retry", t("vp-retry")).on_click(cx.listener(|this, _, _, cx| this.packs_retry(None, cx))))),
            )
            .when(!done, |d| d.child(div().flex().items_start().gap_2().text_size(px(12.5)).line_height(gpui_kit::relative(1.45)).text_color(rgb(p.faint)).child(icon("info").size(px(13.))).child(div().flex_1().min_w_0().child(t("vp-src-note")))))
    }

    /// The iPad's toast under the status bar after an update: getting the packs back, back, or failed with Retry.
    pub(super) fn ipad_packs_toast(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let p = pal();
        let restoring = !self.packs.choice.restoring.is_empty();
        let back = self.packs.restored && self.packs.ready_at.is_some();
        let failed = restoring && self.packs.failed.is_some();
        let telling = restoring && self.packs_tell();
        if !(telling || back || failed) {
            return None;
        }
        let (names, size) = self.packs_restoring_said();
        let (lead, title, text): (AnyElement, String, String) = if back {
            (icon("ipad/check").size(px(18.)).text_color(rgb(0x1f_8a_4c)).into_any_element(), t("vp-back-done").into(), t("vp-all-ready-hint").into())
        } else if failed {
            (icon("ipad/cloud-off").size(px(18.)).text_color(rgb(p.warning)).into_any_element(), t("vp-back-failed").into(), t("vp-back-apple").into())
        } else {
            (icon("ipad/download").size(px(18.)).into_any_element(), t("vp-back-title").into(), ta!("vp-back-percent", percent = self.packs.percent()))
        };
        let _ = (names, size);
        let card = div()
            .id("vp-ipad-toast")
            .occlude()
            .flex()
            .items_center()
            .gap(px(12.))
            .px(px(16.))
            .py(px(10.))
            .min_w(px(360.))
            .rounded_full()
            .cursor_pointer()
            .child(lead)
            .child(div().flex_1().min_w_0().flex().flex_col().child(div().text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).child(title)).child(div().text_size(px(12.5)).text_color(rgb(p.muted)).child(text)))
            .when(!back && !failed, |d| d.child(ring(self.packs.got().map_or(0.0, Got::share), 24., p.accent)))
            .when(failed, |d| d.child(div().id("vp-ipad-toast-retry").text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).text_color(rgb(p.accent)).child(t("vp-retry")).on_click(cx.listener(|this, _, _, cx| {
                cx.stop_propagation();
                this.packs_retry(None, cx);
            }))))
            .on_click(cx.listener(|this, _, _, cx| {
                this.packs.details = true;
                cx.notify();
            }));
        Some(ipad::glass(card, false).into_any_element())
    }

    /// The iPad's Voice packs sheet, from the toast, the note or Settings: a ring, each pack, the source, and Pause,
    /// Retry or Cancel.
    pub(super) fn ipad_packs_sheet(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.packs.details {
            return None;
        }
        let p = pal();
        let got = self.packs.got().cloned().unwrap_or_default();
        let failed = self.packs.failed.is_some();
        let paused = self.packs.paused;
        let done = !self.packs.under_way() && !failed;
        let size = window.viewport_size();
        let (w, h) = (f32::from(size.width), f32::from(size.height));
        let link = |id: &'static str, text: &str, bold: bool| div().id(id).min_w(px(80.)).text_size(px(16.)).text_color(rgb(p.accent)).cursor_pointer().when(bold, |d| d.font_weight(FontWeight::SEMIBOLD)).child(text.to_string());
        let navbar = div()
            .flex()
            .items_center()
            .h(px(56.))
            .px(px(18.))
            .border_b_1()
            .border_color(rgb(p.line))
            .child(div().min_w(px(80.)).when(!done && !failed, |d| d.child(link("vp-sheet-pause", if paused { t("vp-resume") } else { t("vp-pause") }, false).on_click(cx.listener(move |this, _, _, cx| if paused { this.packs_retry(None, cx) } else { this.packs_pause(cx) })))))
            .child(div().flex_1().flex().justify_center().text_size(px(17.)).font_weight(FontWeight::SEMIBOLD).child(t("vp-packs")))
            .child(div().min_w(px(80.)).flex().justify_end().child(link("vp-sheet-done", t("vp-done"), true).on_click(cx.listener(|this, _, _, cx| {
                this.packs.details = false;
                cx.notify();
            }))));
        let hero_mark: AnyElement = if done {
            div().size(px(64.)).flex().items_center().justify_center().rounded_full().bg(rgb(0x1f_8a_4c)).text_color(rgb(0xff_ff_ff)).child(icon("ipad/check").size(px(30.))).into_any_element()
        } else if failed {
            div().size(px(64.)).flex().items_center().justify_center().rounded_full().bg(rgb(0xfd_f0_e1)).text_color(rgb(0xc4_6a_00)).child(icon("ipad/cloud-off").size(px(30.))).into_any_element()
        } else {
            ring(got.share(), 64., p.accent).into_any_element()
        };
        let hero = div()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(6.))
            .py(px(18.))
            .child(hero_mark)
            .child(div().pt_2().text_size(px(17.)).font_weight(FontWeight::SEMIBOLD).child(if done {
                t("vp-all-ready").to_string()
            } else if failed {
                t("vp-stopped").to_string()
            } else {
                ta!("vp-of", done = mb(got.done()), total = mb(got.total()))
            }))
            .child(div().text_size(px(13.5)).text_color(rgb(p.muted)).child(if done {
                t("vp-all-ready-hint").to_string()
            } else if failed {
                t("vp-unreachable").to_string()
            } else if paused {
                t("vp-paused").to_string()
            } else {
                [time_left(&got).map(|time| ta!("vp-left", time = time)), speed(&got)].into_iter().flatten().collect::<Vec<_>>().join(" · ")
            }));
        let group = |rows: Vec<AnyElement>| div().flex().flex_col().rounded(px(14.)).bg(rgb(p.chrome)).overflow_hidden().children(rows.into_iter().enumerate().map(|(i, r)| div().when(i > 0, |d| d.border_t_1().border_color(rgb(p.line))).child(r)));
        let head = |text: String| div().px(px(4.)).pt(px(14.)).pb(px(6.)).font_family(theme::MONO).text_size(px(10.5)).text_color(rgb(p.faint)).child(text.to_uppercase());
        let rows: Vec<AnyElement> = got
            .packs
            .iter()
            .map(|(pack, here, total)| {
                let share = if *total == 0 { 1.0 } else { *here as f32 / *total as f32 };
                let state = if here >= total {
                    t("vp-ready").to_string()
                } else if *here > 0 {
                    if failed { format!("{} {}%", t("vp-stopped-short"), (share * 100.0) as u32) } else if paused { t("vp-paused").to_string() } else { format!("{} / {}", mb(*here), mb(*total)) }
                } else {
                    t("vp-waiting").to_string()
                };
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .px(px(16.))
                    .py(px(12.))
                    .child(div().flex().items_baseline().gap_2().text_size(px(15.)).child(pack_name(*pack)).when(*pack == Pack::Live, |d| d.child(div().text_size(px(12.)).text_color(rgb(p.faint)).child(pack_tag(*pack)))).child(div().flex_1()).child(div().text_size(px(13.)).text_color(rgb(if here >= total { 0x1f_8a_4c } else if failed && *here > 0 { p.warning } else { p.muted })).child(state)))
                    .when(here < total, |d| d.child(bar(share, 3., if failed { p.warning } else if paused { p.faint } else { p.accent })))
                    .into_any_element()
            })
            .collect();
        let n = rows.len();
        let source = got.source.clone().unwrap_or_else(|| "Alcyo".into());
        let source_row = |name: &str, hint: &str, on: bool| div().flex().items_center().gap_3().px(px(16.)).py(px(10.)).child(div().flex_1().flex().flex_col().child(div().text_size(px(15.)).child(name.to_string())).child(div().text_size(px(12.5)).text_color(rgb(p.faint)).child(hint.to_string()))).child(div().w(px(18.)).text_color(rgb(p.accent)).when(on, |d| d.child(icon("ipad/check").size(px(16.))))).into_any_element();
        let body = div()
            .id("vp-sheet-body")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px(px(24.))
            .pb(px(24.))
            .child(hero)
            .when(failed, |d| {
                d.child(group(vec![div().id("vp-sheet-retry").flex().items_center().justify_center().gap_2().py(px(13.)).text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).text_color(rgb(p.accent)).cursor_pointer().child(icon("ipad/refresh").size(px(16.))).child(t("vp-retry")).on_click(cx.listener(|this, _, _, cx| this.packs_retry(None, cx))).into_any_element(), div().id("vp-sheet-modelscope").flex().justify_center().py(px(13.)).text_size(px(15.)).text_color(rgb(p.accent)).cursor_pointer().child(t("vp-try-modelscope")).on_click(cx.listener(|this, _, _, cx| this.packs_retry(Some("ModelScope"), cx))).into_any_element()]))
                    .child(div().px(px(4.)).pt_2().text_size(px(12.5)).text_color(rgb(p.faint)).child(format!("{} {}", t("vp-kept-note"), t("vp-back-apple"))))
            })
            .when(n > 0, |d| d.child(head(ta!("vp-n-packs", n = n))).child(group(rows)))
            .when(!done && !failed, |d| {
                d.child(head(t("vp-source").to_string()))
                    .child(group(vec![source_row("Cloudflare", "models.alcyo.app", source == "Alcyo"), source_row("ModelScope", t("vp-faster-auto"), source == "ModelScope")]))
                    .child(div().px(px(4.)).pt_2().text_size(px(12.5)).text_color(rgb(p.faint)).child(t("vp-src-note")))
                    .child(div().pt(px(18.)).child(group(vec![div().id("vp-sheet-cancel").flex().justify_center().py(px(13.)).text_size(px(15.)).text_color(rgb(p.danger)).cursor_pointer().child(t("vp-cancel-download")).on_click(cx.listener(|this, _, _, cx| this.packs_cancel(cx))).into_any_element()])))
            });
        let sheet_w = 540f32.min(w - 40.);
        let sheet_h = 640f32.min(h - 80.);
        Some(
            div()
                .id("vp-sheet-layer")
                .occlude()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(gpui_kit::hsla(0., 0., 0., 0.28))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.packs.details = false;
                    cx.notify();
                }))
                .child(
                    div()
                        .id("vp-sheet")
                        .occlude()
                        .w(px(sheet_w))
                        .h(px(sheet_h))
                        .flex()
                        .flex_col()
                        .rounded(px(16.))
                        .bg(rgb(p.surface))
                        .overflow_hidden()
                        .on_click(|_, _, cx| cx.stop_propagation())
                        .child(navbar)
                        .child(body),
                )
                .into_any_element(),
        )
    }

    /// The iPad's Settings › Speech rows for the packs: what is on this iPad, then what can be added.
    pub(super) fn sheet_packs(&mut self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let p = pal();
        let downloading = self.packs.job.as_ref().map(|j| j.got.clone());
        let row = |pack: Pack, cx: &mut Context<Self>| {
            let status = self.packs.status(pack);
            let got = downloading.as_ref().and_then(|g| g.packs.iter().find(|(q, ..)| *q == pack).copied()).filter(|(_, here, total)| here < total);
            let id = |what: &str| ElementId::Name(format!("vp-ipad-set-{what}-{}", pack.id()).into());
            let right: AnyElement = if let Some((_, here, total)) = got {
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(div().w(px(130.)).flex().flex_col().gap_1().child(div().font_family(theme::MONO).text_size(px(11.)).text_color(rgb(p.faint)).child(format!("{} / {}", mb(here), mb(total)))).child(bar(here as f32 / total.max(1) as f32, 3., p.accent)))
                    .child(div().id(id("stop")).text_color(rgb(p.accent)).cursor_pointer().child(icon("x").size(px(13.))).on_click(cx.listener(|this, _, _, cx| this.packs_pause(cx))))
                    .into_any_element()
            } else if status.usable() {
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(div().font_family(theme::MONO).text_size(px(11.)).text_color(rgb(p.faint)).child(mb(pack.size())))
                    .when(status == Status::BuiltIn, |d| d.child(div().text_size(px(12.5)).text_color(rgb(p.faint)).child(t("vp-built-in"))))
                    .when(status != Status::BuiltIn, |d| d.child(div().id(id("delete")).text_color(rgb(p.danger)).cursor_pointer().child(icon("ipad/trash").size(px(18.))).on_click(cx.listener(move |this, _, _, cx| this.packs_delete(pack, cx)))))
                    .into_any_element()
            } else {
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(div().font_family(theme::MONO).text_size(px(11.)).text_color(rgb(p.faint)).child(mb(pack.size())))
                    .child(sheet::pill_button(id("get"), t("vp-download"), true, Some("download")).on_click(cx.listener(move |this, _, _, cx| {
                        let with = if pack == Pack::Live { vec![Pack::Zh, Pack::Live] } else { vec![pack] };
                        this.packs_download(&with, cx);
                    })))
                    .into_any_element()
            };
            div()
                .min_h(px(60.))
                .flex()
                .items_center()
                .gap(px(12.))
                .px(px(16.))
                .when(pack == Pack::Live, |d| d.pl(px(32.)))
                .child(div().flex_1().min_w_0().flex().flex_col().gap(px(2.)).py(px(8.)).child(div().flex().items_baseline().gap_2().text_size(px(15.)).font_weight(FontWeight::MEDIUM).child(pack_name(pack)).when(pack != Pack::Live, |d| d.child(div().text_size(px(13.)).font_weight(FontWeight::NORMAL).text_color(rgb(p.muted)).child(pack_tag(pack))))).child(div().text_size(px(12.5)).text_color(rgb(p.faint)).child(pack_hint(pack))))
                .child(right)
                .into_any_element()
        };
        let (have, rest): (Vec<Pack>, Vec<Pack>) = [Pack::Zh, Pack::Live, Pack::Spk, Pack::En, Pack::Ja].into_iter().partition(|pack| self.packs.usable(*pack));
        let here: u64 = Pack::ALL.iter().map(|pack| self.packs.places.on_disk(*pack)).sum();
        let mut out = vec![];
        if !have.is_empty() {
            let rows: Vec<AnyElement> = have.into_iter().map(|pack| row(pack, cx)).collect();
            out.push(div().flex().flex_col().gap(px(8.)).child(sheet::label(t("vp-on-this-ipad"), Some(div().child(ta!("vp-on-ipad", size = mb(here))).into_any_element()))).child(sheet::group(rows)).child(div().px(px(4.)).text_size(px(12.5)).text_color(rgb(p.faint)).child(t("vp-delete-zh-note"))).into_any_element());
        }
        if !rest.is_empty() {
            let rows: Vec<AnyElement> = rest.into_iter().map(|pack| row(pack, cx)).collect();
            out.push(div().flex().flex_col().gap(px(8.)).child(sheet::label(t("vp-available"), None)).child(sheet::group(rows)).into_any_element());
        }
        let failed = self.packs.failed.is_some();
        out.push(
            div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .mt(px(-8.))
                .px(px(4.))
                .when(failed, |d| d.child(div().id("vp-ipad-set-retry").flex().gap_2().items_center().text_size(px(13.)).text_color(rgb(p.warning)).child(icon("ipad/cloud-off").size(px(15.))).child(div().flex_1().child(t("vp-unreachable"))).child(div().text_color(rgb(p.accent)).font_weight(FontWeight::SEMIBOLD).cursor_pointer().child(t("vp-retry"))).on_click(cx.listener(|this, _, _, cx| this.packs_retry(None, cx)))))
                .child(div().text_size(px(12.5)).line_height(gpui_kit::relative(1.45)).text_color(rgb(p.faint)).child(format!("{} {}", t("vp-src-note"), ta!("vp-bundled", size = mb(alcyo_models::embedding().size)))))
                .into_any_element(),
        );
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_read_as_the_design_writes_them() {
        assert_eq!(mb(239_549_735), "240 MB");
        assert_eq!(mb(797_000_000), "797 MB");
        assert_eq!(mb(1_362_000_000), "1.36 GB");
    }

    #[test]
    fn a_language_needs_its_own_pack() {
        assert_eq!(pack_for_language("zh-CN"), Some(Pack::Zh));
        assert_eq!(pack_for_language("ja-JP"), Some(Pack::Ja));
        assert_eq!(pack_for_language("auto"), None);
    }

    #[test]
    fn the_time_left_rounds_up() {
        let got = Got { packs: vec![(Pack::Zh, 0, 84_000_000)], speed: 8_400_000, ..Default::default() };
        assert_eq!(time_left(&got).as_deref(), Some("10 s"));
        let got = Got { packs: vec![(Pack::En, 0, 665_000_000)], speed: 8_400_000, ..Default::default() };
        assert_eq!(time_left(&got).as_deref(), Some("2 min"));
    }
}
