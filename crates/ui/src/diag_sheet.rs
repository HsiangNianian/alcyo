//! Export diagnostic bundle (#360): a sheet that says what goes into one meeting's bundle, lets the sound and the
//! model's words be left out and the time be cut to its last minutes, says about how big it will be, and writes it
//! (crates/diag): saved where chosen on a desktop, handed to the share sheet on the iPad. Opened from the meeting's
//! Export, the iPad's ⋯ menu and a meeting card's menu. On the iPad it is a popover hanging from the toolbar's export
//! button (`ipad_diag_popover`); a desktop keeps the dialog. A child of `app`.

use super::*;
use alcyo_diag::bundle::{self, Options, LAST_MINUTES};
use gpui_kit::{AnyElement, BoxShadow, FontWeight};
use serde_json::{json, Value};

/// Where the sheet stands.
#[derive(Default)]
pub(crate) struct DiagState {
    pub(crate) options: Options,
    /// About how big the bundle will be, and for which choices.
    estimate: Option<(Options, u64)>,
    estimating: bool,
    busy: bool,
    result: Option<Exported>,
    /// The stretch of speech whose drafts were last noted, so a draft is noted once, not as it grows.
    pub(crate) drafted: Option<usize>,
    /// On the iPad's popover, what is always in the bundle is shown (its ⓘ was tapped).
    info: bool,
}

impl Workbench {
    /// How the meeting is set up. Secrets are here by their names only to show whether they are set: the bundle masks
    /// them ([`alcyo_diag::redact`]).
    fn diag_settings(&self) -> Value {
        let s = &self.form.saved;
        let running = match self.session().local_model_up() {
            Ok(model) => json!(model.and_then(|m| m.file_name().map(|n| n.to_string_lossy().into_owned()))),
            Err(()) => json!("starting or stopping"),
        };
        json!({
            "participants": self.live.participants(),
            "speakers_heard": self.live.speakers().len(),
            "speech_language": s.speech_language,
            "system_speech": s.system_speech,
            "listen": s.listen,
            "input": s.input,
            "computer": s.computer,
            "diarize": s.diarize,
            "keep_audio": s.keep_audio,
            "turn_pages": s.turn_pages,
            "glossary_lines": s.glossary.lines().filter(|l| !l.trim().is_empty()).count(),
            "remote_enabled": s.enabled,
            "remote_base_url": s.base_url,
            "remote_model": s.model,
            "api_key": s.api_key,
            "proxy": s.proxy,
            "proxy_url": s.proxy_url,
            "proxy_user": s.proxy_user,
            "proxy_password": s.proxy_password,
            "local_model_running": running,
            "interface_language": s.interface_language,
            "appearance": s.appearance,
            "ipad_layout": crate::platform::ipad_layout(),
        })
    }

    /// Every settings file: the settings folder's, the small files beside the meetings, and the models' choices.
    fn diag_configs(&self) -> Vec<(String, PathBuf)> {
        let mut out: Vec<(String, PathBuf)> = vec![];
        let mut add = |prefix: &str, dir: &std::path::Path, wanted: &dyn Fn(&str) -> bool| {
            let Ok(entries) = std::fs::read_dir(dir) else { return };
            let mut files: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| p.is_file()).collect();
            files.sort();
            for path in files {
                let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
                // The settings folder is the data folder on Apple's systems: each file once.
                if wanted(&name) && !out.iter().any(|(_, p)| *p == path) {
                    out.push((format!("{prefix}{name}"), path));
                }
            }
        };
        if let Ok(dir) = alcyo_agent::settings::config_dir() {
            add("", &dir, &|n| !n.ends_with(".lock") && !n.ends_with(".tmp"));
        }
        add("data/", &self.data, &|n| n.ends_with(".json") || n.ends_with(".txt") || !n.contains('.'));
        add("models/", &self.models, &|n| !n.ends_with(".gguf") && !n.ends_with(".part") && !n.ends_with(".bin") && !n.ends_with(".onnx"));
        out
    }

    /// What the bundle is made from, as far as the window holds it: the rest (the device, the models' hashes, the
    /// boards drawn) is worked out off its thread.
    fn diag_input(&mut self) -> bundle::Input {
        let s = &self.form.saved;
        let secrets = [s.api_key.clone(), s.proxy_password.clone()].into_iter().filter(|s| !s.is_empty()).collect();
        let meeting_dir = self.store.as_ref().map(|store| store.folder().join(self.live.id()));
        let settings = self.diag_settings();
        let configs = self.diag_configs();
        bundle::Input {
            meeting: Value::Object(self.live.snapshot()),
            meeting_dir,
            settings,
            logs: alcyo_diag::logs_dir(&self.data),
            configs,
            secrets,
            boards: vec![],
            environment: Value::Null,
            models: vec![],
            now_ms: alcyo_diag::now_ms(),
            open_now: true,
        }
    }

    /// The sheet came up, or a choice changed: work out again about how big the bundle will be, off the window's thread.
    pub(crate) fn diag_estimate(&mut self, cx: &mut Context<Self>) {
        if self.diag.estimating {
            return;
        }
        self.diag.estimating = true;
        let options = self.diag.options;
        let input = self.diag_input();
        let size = cx.background_executor().spawn(async move { bundle::estimate(&input, &options) });
        cx.spawn(async move |this, cx| {
            let size = size.await;
            let _ = this.update(cx, |this, cx| {
                this.diag.estimating = false;
                this.diag.estimate = Some((options, size));
                // Changed meanwhile: once more, for what is chosen now.
                if this.diag.options != options && this.panel == Panel::Diagnostics {
                    this.diag_estimate(cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn diag_choose(&mut self, change: impl FnOnce(&mut Options), cx: &mut Context<Self>) {
        change(&mut self.diag.options);
        self.diag.result = None;
        self.diag_estimate(cx);
        cx.notify();
    }

    /// Open the sheet: fresh, with its size worked out.
    pub(crate) fn diag_opened(&mut self, cx: &mut Context<Self>) {
        self.diag.result = None;
        self.diag.info = false;
        if self.live.audio().is_empty() {
            self.diag.options.audio = false;
        }
        self.diag_estimate(cx);
    }

    /// Write the bundle: where the person chooses on a desktop, aside and then to the share sheet on the iPad. The
    /// meeting is captured on the click, as an export is.
    fn start_diag_export(&mut self, cx: &mut Context<Self>) {
        if self.diag.busy {
            return;
        }
        let options = self.diag.options;
        let mut input = self.diag_input();
        let images: Vec<Option<alcyo_meetings::PageImage>> = self
            .live
            .board_images()
            .iter()
            .map(|r| {
                r.as_ref().and_then(|r| match &self.store {
                    Some(store) => store.page_image(self.live.id(), r).ok(),
                    None => alcyo_meetings::decode_data_url(r).ok(),
                })
            })
            .collect();
        let (store, id) = (self.store.clone(), self.live.id().to_string());
        let asset = move |name: &str| match &store {
            Some(store) => store.page_image(&id, name).ok(),
            None => alcyo_meetings::decode_data_url(name).ok(),
        };
        let frozen = alcyo_export::freeze(&self.live, !crate::i18n::chinese(), &crate::i18n::line_text, &mut self.ink.borrow_mut().glyphs, self.hanzi.clone(), &images, &asset);
        let name = bundle::file_name(self.live.title(), chrono::Local::now());
        let directory = self.export_dir.clone().unwrap_or_else(|| {
            let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map_or_else(|| self.data.clone(), PathBuf::from);
            Some(home.join("Documents")).filter(|d| d.is_dir()).unwrap_or(home)
        });
        let answer = cx.prompt_for_new_path(&directory, Some(&name));
        alcyo_diag::event("diag.export", json!({ "audio": options.audio, "prompts": options.prompts, "last_minutes": options.last_minutes }));
        self.diag.busy = true;
        self.diag.result = None;
        let (data, models) = (self.data.clone(), self.models.clone());
        cx.spawn(async move |this, cx| {
            let chosen = match answer.await {
                Ok(Ok(chosen)) => chosen,
                Ok(Err(error)) => {
                    let _ = this.update(cx, |this, cx| this.finish_diag(Some(Exported::Failed(format!("{error:#}"))), cx));
                    return;
                }
                Err(_) => None,
            };
            let Some(path) = chosen else {
                let _ = this.update(cx, |this, cx| this.finish_diag(None, cx));
                return;
            };
            let path = if path.extension().is_some_and(|x| x.eq_ignore_ascii_case("zip")) { path } else { path.with_extension("zip") };
            let target = path.clone();
            let written = cx
                .background_executor()
                .spawn(async move {
                    // What was just noted, on disk before it is read.
                    alcyo_diag::flush(Duration::from_secs(2));
                    input.environment = alcyo_diag::device::describe(&data);
                    input.models = alcyo_diag::device::model_files(&models, &alcyo_diag::logs_dir(&data).join("model-hashes.json"));
                    input.boards = frozen.every_board().into_iter().map(|(png, svg)| bundle::Board { png, svg: Some(svg) }).collect();
                    bundle::write(&input, &options, &target)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                let result = match written {
                    Ok(written) => {
                        alcyo_diag::event("diag.exported", json!({ "bytes": written.bytes, "files": written.files }));
                        this.export_dir = path.parent().map(PathBuf::from);
                        crate::platform::share_file(&path);
                        Exported::Saved { path, text_left_out: false }
                    }
                    Err(error) => {
                        eprintln!("diagnostic bundle failed: {error:#}");
                        Exported::Failed(format!("{error:#}"))
                    }
                };
                this.finish_diag(Some(result), cx);
            });
        })
        .detach();
        cx.notify();
    }

    fn finish_diag(&mut self, result: Option<Exported>, cx: &mut Context<Self>) {
        self.diag.busy = false;
        self.diag.result = result;
        cx.notify();
    }

    /// A choice that can be turned off, with what it holds.
    fn diag_switch(&self, id: &'static str, title: &'static str, hint: &'static str, on: bool, enabled: bool, set: fn(&mut Options, bool), cx: &Context<Self>) -> AnyElement {
        let ipad = crate::platform::ipad_layout();
        let p = pal();
        let switch = Switch::new(id).checked(on && enabled).accessibility_label(title);
        let switch = if enabled { switch.on_click(cx.listener(move |this, on: &bool, _, cx| this.diag_choose(|o| set(o, *on), cx))).into_any_element() } else { div().opacity(0.4).child(switch).into_any_element() };
        div()
            .flex()
            .items_center()
            .justify_between()
            .gap_3()
            .py(px(if ipad { 12. } else { 8. }))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(div().when(ipad, |d| d.text_size(px(15.))).when(!ipad, |d| d.text_ui()).child(title))
                    .child(div().text_small().text_color(rgb(p.muted)).child(hint)),
            )
            .child(switch)
            .into_any_element()
    }

    pub(crate) fn diag_panel(&self, cx: &mut Context<Self>) -> gpui_kit::Stateful<gpui_kit::Div> {
        let ipad = crate::platform::ipad_layout();
        let p = pal();
        let o = self.diag.options;
        let heard = !self.live.audio().is_empty();
        // On the iPad, rows grouped on a soft grey as Settings has them (#341); on a desktop, hairlines between.
        let group = |rows: Vec<AnyElement>| {
            let mut list = div().flex().flex_col();
            let last = rows.len().saturating_sub(1);
            for (i, row) in rows.into_iter().enumerate() {
                list = list.child(div().when(ipad, |d| d.px(px(16.))).when(i < last, |d| d.border_b_1().border_color(rgb(p.line))).child(row));
            }
            if ipad { list.rounded(px(16.)).bg(ipad_dialogs::well()) } else { list.border_t_1().border_b_1().border_color(rgb(p.line)) }
        };
        let label = |text: &'static str| div().pt_1().text_small().font_weight(FontWeight::MEDIUM).text_color(rgb(p.muted)).child(text);
        let audio_hint = if heard { t("diag-audio-hint") } else { t("diag-no-audio") };
        let choices = group(vec![
            self.diag_switch("diag-audio", t("diag-audio"), audio_hint, o.audio, heard, |o, on| o.audio = on, cx),
            self.diag_switch("diag-prompts", t("diag-prompts"), t("diag-prompts-hint"), o.prompts, true, |o, on| o.prompts = on, cx),
        ]);
        let always = div()
            .flex()
            .gap(px(10.))
            .when(ipad, |d| d.p(px(14.)).rounded(px(16.)).bg(ipad_dialogs::well()))
            .child(div().flex_none().pt(px(2.)).text_color(rgb(p.accent)).child(icon("ipad/check").size(px(15.))))
            .child(div().flex_1().min_w_0().flex().flex_col().gap(px(2.)).child(div().text_small().font_weight(FontWeight::MEDIUM).child(t("diag-always"))).child(div().text_small().text_color(rgb(p.muted)).line_height(gpui_kit::relative(1.4)).child(t("diag-always-list"))));
        let minutes = div().flex().flex_wrap().gap_2().children(LAST_MINUTES.iter().enumerate().map(|(i, choice)| {
            let chosen = o.last_minutes == *choice;
            let text: String = match choice {
                Some(n) => ta!("diag-last-minutes", n = *n),
                None => t("diag-whole-meeting").to_string(),
            };
            let choice = *choice;
            let pick = cx.listener(move |this, _, _, cx| this.diag_choose(|o| o.last_minutes = choice, cx));
            if ipad {
                ipad_dialogs::pill(("diag-minutes", i), text, if chosen { ipad_dialogs::Look::Main } else { ipad_dialogs::Look::Plain }, None).h(px(36.)).px(px(16.)).text_size(px(14.)).on_click(pick).into_any_element()
            } else {
                Quiet::new(("diag-minutes", i)).outline().label(text).choice(chosen).on_click(pick).into_any_element()
            }
        }));
        let size = match self.diag.estimate {
            Some((for_options, bytes)) if for_options == o || !self.diag.estimating => ta!("diag-estimate", size = crate::storage::human(bytes)),
            _ => t("diag-estimating").to_string(),
        };
        let busy = self.diag.busy;
        let go_label = if busy { t("exporting") } else { t("diag-export") };
        let go = if ipad {
            ipad_dialogs::pill("diag-go", go_label, ipad_dialogs::Look::Main, Some("ipad/export")).when(!busy, |d| d.on_click(cx.listener(|this, _, _, cx| this.start_diag_export(cx)))).when(busy, |d| d.opacity(0.5)).into_any_element()
        } else {
            button("diag-go", go_label).disabled(busy).on_click(cx.listener(|this, _, _, cx| this.start_diag_export(cx))).into_any_element()
        };
        let footer = div().flex().items_center().justify_between().gap_3().pt_1().child(div().text_small().text_color(rgb(p.muted)).child(size)).child(go);
        let result = self.diag.result.as_ref().map(|result| match result {
            Exported::Saved { path, .. } if ipad => {
                let shared = path.clone();
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(div().flex_1().min_w_0().flex().flex_col().gap_1().child(div().text_small().font_weight(FontWeight::MEDIUM).child(t("ipad-export-ready"))).child(div().text_small().text_color(rgb(p.faint)).overflow_hidden().whitespace_nowrap().text_ellipsis().child(name)))
                    .child(ipad_dialogs::pill("diag-share-again", t("ipad-share-again"), ipad_dialogs::Look::Plain, Some("ipad/export")).on_click(move |_, _, _| crate::platform::share_file(&shared)))
                    .into_any_element()
            }
            Exported::Saved { path, .. } => {
                let shown = path.clone();
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(div().text_small().child(ta!("saved-to-path", path = path.display().to_string())))
                    .child(button("diag-reveal", t("show-in-folder")).on_click(cx.listener(move |_, _, _, cx| cx.reveal_path(&shown))))
                    .into_any_element()
            }
            Exported::Failed(error) => div().text_small().text_color(rgb(p.danger)).child(crate::ta!("could-not-export", error = format!("{error}"))).into_any_element(),
        });
        div()
            .id("diag-panel")
            .flex_1()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap_3()
            .child(div().text_small().text_color(rgb(p.muted)).line_height(gpui_kit::relative(1.4)).child(t("diag-intro")))
            .child(choices)
            .child(always)
            .child(label(t("diag-time")))
            .child(minutes)
            .child(footer)
            .when_some(result, |d, result| d.child(result))
    }
}

/// The iPad's popover: how wide it is, and how far its arrow is from its right edge at most.
const POP_W: f32 = 360.;

impl Workbench {
    /// The bundle on the iPad (round 6, C): no modal, a compact popover hanging from the toolbar's export button with
    /// an arrow, as Files and Pages do for export options. The two choices are iOS switches, the time a pull-down
    /// with checkmarks, what is always included folded behind an ⓘ, and Export… a full-width button. A touch
    /// outside puts it away.
    pub(crate) fn ipad_diag_popover(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let p = pal();
        let dark = theme::is_dark();
        let f = ipad::Frame::of(window);
        // Where the export button is: in the floating bar at the top right, the transcript's button after it when
        // wide; in a narrow window, the ⋯ that holds it.
        let button_from_right = match f.mode {
            ipad::Mode::Wide => f.margin + 4. + 44. + 2. + 9. + 2. + 22.,
            ipad::Mode::Upright => f.margin + 4. + 22.,
            ipad::Mode::Compact { .. } => 12. + 22.,
        };
        let top = match f.mode {
            ipad::Mode::Compact { .. } => f.top() + 60.,
            _ => f.top() + 14. + ipad::TARGET,
        } + 10.;
        let right = (button_from_right - 40.).max(12.);
        let arrow_x = (POP_W - (button_from_right - right)).clamp(24., POP_W - 24.);
        let o = self.diag.options;
        let heard = !self.live.audio().is_empty();
        let card_bg: Hsla = if dark { rgb(0x2c_2c_2e).into() } else { rgb(0xf6_f6_f4).into() };
        let cell: Hsla = if dark { alpha(0xff_ff_ff, 0.07) } else { rgb(0xff_ff_ff).into() };
        let group = |rows: Vec<AnyElement>| {
            let last = rows.len().saturating_sub(1);
            div().flex().flex_col().rounded(px(12.)).bg(cell).children(rows.into_iter().enumerate().map(move |(i, row)| div().pl(px(16.)).child(div().pr(px(14.)).when(i < last, |d| d.border_b_1().border_color(rgb(p.line))).child(row))))
        };
        let row = || div().min_h(px(46.)).py(px(8.)).flex().items_center().gap(px(12.));
        let text = |title: &'static str, hint: Option<&'static str>| {
            div().flex_1().min_w_0().flex().flex_col().gap(px(1.)).child(div().text_size(px(15.)).text_color(rgb(p.text)).child(title)).when_some(hint, |d, hint| d.child(div().text_size(px(12.5)).text_color(rgb(p.muted)).child(hint)))
        };
        let audio_on = o.audio && heard;
        let audio = ipad_dialogs::ios_switch("diag-audio", audio_on, heard).when(heard, |d| d.on_click(cx.listener(move |this, _, _, cx| this.diag_choose(|o| o.audio = !audio_on, cx))));
        let prompts_on = o.prompts;
        let prompts = ipad_dialogs::ios_switch("diag-prompts", prompts_on, true).on_click(cx.listener(move |this, _, _, cx| this.diag_choose(|o| o.prompts = !prompts_on, cx)));
        let minutes_text = |choice: Option<u32>| -> String {
            match choice {
                Some(n) => ta!("diag-last-minutes", n = n),
                None => t("diag-whole-meeting").to_string(),
            }
        };
        let times: Vec<ipad_dialogs::MenuRow> = LAST_MINUTES
            .iter()
            .map(|choice| {
                let choice = *choice;
                ipad_dialogs::MenuRow::new(minutes_text(choice), move |this, _, cx| this.diag_choose(|o| o.last_minutes = choice, cx)).checked(o.last_minutes == choice)
            })
            .collect();
        let pull = div()
            .id("diag-time")
            .flex_none()
            .h(px(32.))
            .px(px(6.))
            .flex()
            .items_center()
            .gap(px(4.))
            .rounded(px(8.))
            .cursor_pointer()
            .text_size(px(15.))
            .text_color(rgb(p.accent))
            .child(minutes_text(o.last_minutes))
            .child(icon("ipad/updown").size(px(15.)));
        let choices = group(vec![
            row().child(text(t("diag-audio"), (!heard).then_some(t("diag-no-audio")))).child(audio).into_any_element(),
            row().child(text(t("diag-prompts"), Some(t("diag-prompts-off")))).child(prompts).into_any_element(),
            row().child(text(t("diag-time"), None)).child(self.ipad_menu("diag-time", pull, times, false, cx)).into_any_element(),
        ]);
        let info = self.diag.info;
        let always = group(vec![div()
            .flex()
            .flex_col()
            .child(
                row()
                    .id("diag-info")
                    .cursor_pointer()
                    .child(text(t("diag-always"), None))
                    .child(div().flex_none().text_color(rgb(p.accent)).child(icon("ipad/info").size(px(20.))))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.diag.info = !this.diag.info;
                        cx.notify();
                    })),
            )
            .when(info, |d| {
                d.child(div().pb(px(12.)).flex().flex_col().gap(px(6.)).text_size(px(12.5)).line_height(gpui_kit::relative(1.45)).text_color(rgb(p.muted)).child(t("diag-always-list")).child(t("diag-privacy")))
            })
            .into_any_element()]);
        let size = match self.diag.estimate {
            Some((for_options, bytes)) if for_options == o || !self.diag.estimating => ta!("diag-estimate", size = crate::storage::human(bytes)),
            _ => t("diag-estimating").to_string(),
        };
        let title = self.live.title().to_string();
        let head = div()
            .flex()
            .items_center()
            .gap(px(12.))
            .px(px(4.))
            .pb(px(12.))
            .child(div().flex_none().size(px(40.)).rounded(px(10.)).bg(rgb(if dark { 0xf2_f2_f2 } else { 0x11_11_11 })).text_color(rgb(if dark { 0x11_11_11 } else { 0xff_ff_ff })).flex().items_center().justify_center().child(icon("ipad/zip").size(px(22.))))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(div().text_size(px(16.)).font_weight(FontWeight::SEMIBOLD).text_color(rgb(p.text)).child(t("diag-title")))
                    .child(div().text_size(px(12.5)).text_color(rgb(p.muted)).overflow_hidden().text_ellipsis().whitespace_nowrap().child(if title.is_empty() { size } else { format!("{title} · {size}") })),
            );
        let result = self.diag.result.as_ref().map(|result| match result {
            Exported::Saved { path, .. } => {
                let shared = path.clone();
                group(vec![row()
                    .child(div().flex_none().size(px(28.)).rounded(px(7.)).bg(rgb(0x34_c7_59)).text_color(rgb(0xff_ff_ff)).flex().items_center().justify_center().child(icon("ipad/check").size(px(16.))))
                    .child(text(t("diag-ready"), None))
                    .child(div().id("diag-share-again").flex_none().text_size(px(15.)).text_color(rgb(p.accent)).cursor_pointer().active(|d| d.opacity(0.6)).child(t("ipad-share-again")).on_click(move |_, _, _| crate::platform::share_file(&shared)))
                    .into_any_element()])
                .into_any_element()
            }
            Exported::Failed(error) => div().px(px(4.)).text_size(px(12.5)).text_color(rgb(p.danger)).child(crate::ta!("could-not-export", error = format!("{error}"))).into_any_element(),
        });
        let busy = self.diag.busy;
        let go = div()
            .id("diag-go")
            .h(px(46.))
            .w_full()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(12.))
            .bg(rgb(p.accent))
            .text_color(rgb(0xff_ff_ff))
            .text_size(px(16.))
            .font_weight(FontWeight::SEMIBOLD)
            .child(if busy { t("exporting") } else { t("diag-export-dots") })
            .when(busy, |d| d.opacity(0.5))
            .when(!busy, |d| d.cursor_pointer().active(|d| d.opacity(0.7)).on_click(cx.listener(|this, _, _, cx| this.start_diag_export(cx))));
        let arrow_color = card_bg;
        let arrow = canvas(
            |_, _, _| (),
            move |bounds, (), window: &mut Window, _| {
                let (x, y) = (bounds.origin.x + px(arrow_x), bounds.origin.y);
                let mut pb = PathBuilder::fill();
                pb.move_to(point(x - px(13.), y + px(11.)));
                pb.line_to(point(x, y - px(1.)));
                pb.line_to(point(x + px(13.), y + px(11.)));
                pb.close();
                if let Ok(path) = pb.build() {
                    window.paint_path(path, Background::from(arrow_color));
                }
            },
        )
        .absolute()
        .top(px(-10.))
        .left_0()
        .w(px(POP_W))
        .h(px(12.));
        let card = div()
            .id("diag-popover")
            .occlude()
            .absolute()
            .top(px(top))
            .right(px(right))
            .w(px(POP_W))
            .rounded(px(16.))
            .bg(card_bg)
            .shadow(vec![
                BoxShadow { color: alpha(0, if dark { 0.6 } else { 0.3 }), offset: point(px(0.), px(24.)), blur_radius: px(60.), spread_radius: px(-12.), inset: false },
                BoxShadow { color: if dark { alpha(0xff_ff_ff, 0.1) } else { alpha(0, 0.08) }, offset: point(px(0.), px(0.)), blur_radius: px(0.), spread_radius: px(0.5), inset: false },
            ])
            .on_click(|_, _, cx| cx.stop_propagation())
            .child(arrow)
            .child(
                div()
                    .p(px(14.))
                    .flex()
                    .flex_col()
                    .gap(px(12.))
                    .child(head)
                    .child(choices)
                    .child(always)
                    .when_some(result, |d, result| d.child(result))
                    .child(go),
            )
            .with_animation("diag-popover-in", gpui_kit::Animation::new(Duration::from_millis(200)).with_easing(|t: f32| 1. - (1. - t).powi(3)), |d, t| d.opacity(t));
        div()
            .id("diag-popover-veil")
            .occlude()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .on_click(cx.listener(|this, _, _, cx| {
                this.panel = Panel::Transcript;
                cx.notify();
            }))
            .child(card)
            .into_any_element()
    }
}

fn alpha(color: u32, a: f32) -> Hsla {
    let mut c: Hsla = rgb(color).into();
    c.a = a;
    c
}
