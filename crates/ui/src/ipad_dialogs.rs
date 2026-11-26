//! Share and Import as the iPad shows them, in the dialog card of ipad.rs (#341): rounded all through, pill buttons,
//! fields and notes as grouped rows are in Settings, and nothing square left from the desktop's panels. A child of
//! `app` beside ipad.rs.

use super::*;
use gpui_kit::{AnyElement, BoxShadow, ElementId, FontWeight};
use std::rc::Rc;

/// How long "Copied" stands in for "Copy link" once the link is on the clipboard.
pub(super) const COPIED_FOR: Duration = Duration::from_secs(2);

fn alpha(color: u32, a: f32) -> Hsla {
    let mut c: Hsla = rgb(color).into();
    c.a = a;
    c
}

/// A grey that reads on the dialog's white, or its dark, as a group of rows does in Settings.
pub(super) fn well() -> Hsla {
    if theme::is_dark() { alpha(0xff_ff_ff, 0.08) } else { alpha(0x76_76_80, 0.1) }
}

/// What a dialog's button does: the main thing to do, something beside it, or ending something.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum Look {
    Main,
    Plain,
    Ending,
}

/// A button as the iPad draws one: a pill, 44 pt tall for a finger.
pub(super) fn pill(id: impl Into<gpui_kit::ElementId>, text: impl Into<gpui_kit::SharedString>, look: Look, glyph: Option<&str>) -> gpui_kit::Stateful<gpui_kit::Div> {
    let p = pal();
    div()
        .id(id)
        .flex_none()
        .h(px(44.))
        .flex()
        .items_center()
        .justify_center()
        .gap(px(8.))
        .px(px(20.))
        .rounded_full()
        .text_size(px(15.))
        .font_weight(FontWeight::SEMIBOLD)
        .whitespace_nowrap()
        .cursor_pointer()
        .map(|d| match look {
            Look::Main => d.bg(rgb(p.text)).text_color(rgb(p.paper)),
            Look::Plain => d.bg(well()).text_color(rgb(p.text)),
            Look::Ending => d.bg(alpha(p.danger, if theme::is_dark() { 0.22 } else { 0.1 })).text_color(rgb(p.danger)),
        })
        .active(|d| d.opacity(0.7))
        .when_some(glyph.map(str::to_string), |d, glyph| d.child(icon(&glyph).size(px(17.))))
        .child(text.into())
}

/// An icon button as the iPad draws one: round, 44 pt, with a soft fill under a finger.
pub(super) fn round_icon(id: impl Into<gpui_kit::ElementId>, glyph: &str) -> gpui_kit::Stateful<gpui_kit::Div> {
    let p = pal();
    div()
        .id(id)
        .flex_none()
        .size(px(44.))
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .cursor_pointer()
        .text_color(rgb(p.text))
        .hover(|d| d.bg(well()))
        .active(|d| d.bg(well()).opacity(0.7))
        .child(icon(glyph).size(px(19.)))
}

/// One choice in an iPad floating menu.
pub(super) struct MenuRow {
    pub(super) label: String,
    pub(super) glyph: Option<&'static str>,
    /// Shown chosen, with a check.
    pub(super) checked: bool,
    /// A hairline above it, between groups.
    pub(super) apart: bool,
    pub(super) act: Rc<dyn Fn(&mut Workbench, &mut Window, &mut Context<Workbench>)>,
}

impl MenuRow {
    pub(super) fn new(label: impl Into<String>, act: impl Fn(&mut Workbench, &mut Window, &mut Context<Workbench>) + 'static) -> Self {
        Self { label: label.into(), glyph: None, checked: false, apart: false, act: Rc::new(act) }
    }

    pub(super) fn glyph(mut self, glyph: &'static str) -> Self {
        self.glyph = Some(glyph);
        self
    }

    pub(super) fn checked(mut self, checked: bool) -> Self {
        self.checked = checked;
        self
    }

    pub(super) fn apart(mut self) -> Self {
        self.apart = true;
        self
    }
}

/// A short note in a rounded box of its own: what to know before going on.
fn note(lines: Vec<&'static str>) -> gpui_kit::Div {
    let p = pal();
    div()
        .flex()
        .gap(px(10.))
        .p(px(14.))
        .rounded(px(16.))
        .bg(alpha(p.danger, if theme::is_dark() { 0.14 } else { 0.06 }))
        .child(div().flex_none().pt(px(1.)).text_color(rgb(p.danger)).child(icon("ipad/lock").size(px(16.))))
        .child(div().flex_1().min_w_0().flex().flex_col().gap(px(6.)).text_size(px(13.)).line_height(gpui_kit::relative(1.4)).text_color(rgb(p.muted)).children(lines))
}

impl Workbench {
    /// A menu as the iPad shows one (#341): a rounded card floating under its button with 44 pt rows, each with its
    /// icon, put away by a choice or a touch anywhere else. `trigger` opens and closes it; `above` opens it upward,
    /// as from a bar along the bottom.
    pub(super) fn ipad_menu(&self, id: &'static str, trigger: gpui_kit::Stateful<gpui_kit::Div>, rows: Vec<MenuRow>, above: bool, cx: &mut Context<Self>) -> gpui_kit::Div {
        let p = pal();
        let dark = theme::is_dark();
        let open = self.ipad.menu == Some(id);
        let trigger = trigger.when(open, |d| d.bg(well())).on_click(cx.listener(move |this, _, _, cx| {
            this.ipad.menu = if this.ipad.menu == Some(id) { None } else { Some(id) };
            cx.notify();
        }));
        let holder = div().relative().flex_none().child(trigger);
        if !open {
            return holder;
        }
        let rows = rows.into_iter().enumerate().map(|(i, row)| {
            let act = row.act.clone();
            div()
                .id(ElementId::Name(format!("{id}-{i}").into()))
                .min_h(px(44.))
                .flex()
                .items_center()
                .gap(px(12.))
                .px(px(14.))
                .text_size(px(15.))
                .text_color(rgb(p.text))
                .cursor_pointer()
                .when(row.apart, |d| d.border_t_1().border_color(rgb(p.line)))
                .hover(|d| d.bg(well()))
                .active(|d| d.bg(well()).opacity(0.8))
                .child(div().flex_none().w(px(20.)).flex().justify_center().text_color(rgb(if row.checked { p.accent } else { p.muted })).when_some(row.glyph, |d, glyph| d.child(icon(glyph).size(px(18.)))).when(row.glyph.is_none() && row.checked, |d| d.child(icon("ipad/check").size(px(16.)))))
                .child(div().flex_1().min_w_0().whitespace_nowrap().when(row.checked, |d| d.font_weight(FontWeight::SEMIBOLD).text_color(rgb(p.accent))).child(row.label))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.ipad.menu = None;
                    act(this, window, cx);
                    cx.notify();
                }))
        });
        let card = div()
            .id(ElementId::Name(format!("{id}-menu").into()))
            .occlude()
            .min_w(px(220.))
            .max_h(px(440.))
            .overflow_y_scroll()
            .rounded(px(14.))
            .bg(rgb(if dark { 0x2c_2c_2e } else { p.surface }))
            .shadow(vec![
                BoxShadow { color: alpha(0, if dark { 0.55 } else { 0.18 }), offset: point(px(0.), px(12.)), blur_radius: px(40.), spread_radius: px(0.), inset: false },
                BoxShadow { color: if dark { alpha(0xff_ff_ff, 0.1) } else { alpha(0, 0.08) }, offset: point(px(0.), px(0.)), blur_radius: px(0.), spread_radius: px(0.5), inset: false },
            ])
            .py(px(4.))
            .children(rows)
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                this.ipad.menu = None;
                cx.notify();
            }))
            .with_animation(ElementId::Name(format!("{id}-menu-in").into()), gpui_kit::Animation::new(Duration::from_millis(180)).with_easing(|t: f32| 1. - (1. - t).powi(3)), move |d, t| d.opacity(t).mt(px(if above { 0. } else { 6. - 6. * (1. - t) })));
        let anchor = if above { gpui_kit::Anchor::BottomRight } else { gpui_kit::Anchor::TopRight };
        let place = if above { div().absolute().bottom_full().right_0().pb(px(8.)) } else { div().absolute().top_full().right_0().pt(px(6.)) };
        holder.child(place.child(gpui_kit::deferred(gpui_kit::anchored().anchor(anchor).snap_to_window_with_margin(px(12.)).child(card)).with_priority(1)))
    }

    /// Copy the share link, and say so on the button for a moment.
    pub(super) fn copy_share_link(&mut self, link: String, cx: &mut Context<Self>) {
        cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(link));
        self.share_copied = Some(Instant::now());
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(COPIED_FOR).await;
            let _ = this.update(cx, |_, cx| cx.notify());
        })
        .detach();
        cx.notify();
    }

    /// Whether the link was copied just now.
    pub(super) fn share_just_copied(&self) -> bool {
        self.share_copied.is_some_and(|at| at.elapsed() < COPIED_FOR)
    }

    /// Share, on the iPad: before sharing, what it does and a short note on who can see it; while sharing, the QR code
    /// in a rounded card on the left, and the address and the access code on the right, then the note and the buttons.
    pub(super) fn ipad_share_body(&self, cx: &mut Context<Self>) -> gpui_kit::Stateful<gpui_kit::Div> {
        let p = pal();
        let privacy = note(vec![t("share-privacy"), t("share-unencrypted")]);
        let Some(sharing) = &self.sharing else {
            return div()
                .id("share-panel")
                .flex()
                .flex_col()
                .gap(px(16.))
                .child(div().text_size(px(15.)).line_height(gpui_kit::relative(1.45)).text_color(rgb(p.muted)).child(t("share-intro")))
                .child(privacy)
                .when_some(self.share_note.clone(), |d, note| d.child(div().text_size(px(13.)).text_color(rgb(p.danger)).child(note)))
                .child(div().flex().justify_end().child(pill("share-start", t("share-start"), Look::Main, Some("ipad/live")).on_click(cx.listener(|this, _, _, cx| this.start_sharing(cx)))));
        };
        let share = sharing.share();
        let networks: Vec<(String, String)> = share
            .networks()
            .iter()
            .map(|n| {
                let name = match n.kind {
                    alcyo_share::Kind::WiFi => t("share-network-wifi").to_string(),
                    alcyo_share::Kind::Wired => t("share-network-wired").to_string(),
                    alcyo_share::Kind::Unknown => n.interface.clone(),
                };
                (name, share.address(n))
            })
            .collect();
        let chosen = self.share_address.min(networks.len().saturating_sub(1));
        let (link, code) = (share.link(&networks[chosen].1), share.code().to_string());
        let scan = if networks.len() > 1 { ta!("share-scan-on", network = networks[chosen].0.clone()) } else { t("share-scan").to_string() };
        let modules = crate::share::qr(&link).unwrap_or_default();
        // The QR code, drawn module by module, black on white whatever the theme, with the quiet margin a reader needs.
        let qr = canvas(
            |_, _, _| (),
            move |bounds: Bounds<Pixels>, (), window: &mut Window, _: &mut App| {
                let n = modules.len().max(1) as f32 + 8.0;
                let side = f32::from(bounds.size.width).min(f32::from(bounds.size.height));
                let unit = (side / n).floor().max(1.0);
                let margin = (side - unit * (n - 8.0)) / 2.0;
                let origin = bounds.origin + point(px(margin), px(margin));
                for (y, row) in modules.iter().enumerate() {
                    for (x, dark) in row.iter().enumerate() {
                        if *dark {
                            window.paint_quad(gpui_kit::fill(Bounds::new(origin + point(px(x as f32 * unit), px(y as f32 * unit)), size(px(unit), px(unit))), rgb(0x000000)));
                        }
                    }
                }
            },
        )
        .size(px(168.));
        let qr_card = div()
            .flex_none()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(8.))
            .child(div().p(px(6.)).rounded(px(20.)).bg(rgb(0xffffff)).border_1().border_color(rgb(p.line)).child(qr))
            .child(div().w(px(180.)).text_center().text_size(px(12.)).text_color(rgb(p.faint)).child(scan));
        // Each network with its address, in one rounded group; a phone on another network chooses its own here.
        let several = networks.len() > 1;
        let addresses = div().flex().flex_col().rounded(px(14.)).bg(well()).overflow_hidden().children(networks.iter().enumerate().map(|(i, (name, address))| {
            let on = i == chosen;
            div()
                .id(("share-address", i))
                .min_h(px(44.))
                .flex()
                .items_center()
                .gap(px(10.))
                .px(px(14.))
                .when(i > 0, |d| d.border_t_1().border_color(rgb(p.line)))
                .when(several && !on, |d| d.cursor_pointer())
                .child(div().w(px(64.)).flex_none().text_size(px(12.5)).text_color(rgb(p.muted)).overflow_hidden().text_ellipsis().whitespace_nowrap().child(name.clone()))
                .child(div().flex_1().min_w_0().font_family(theme::MONO).text_size(px(14.)).whitespace_nowrap().overflow_hidden().text_ellipsis().text_color(rgb(if on { p.text } else { p.faint })).child(address.trim_start_matches("http://").to_string()))
                .when(several && on, |d| d.child(icon("ipad/check").size(px(16.)).text_color(rgb(p.accent))))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.share_address = i;
                    cx.notify();
                }))
        }));
        let watching = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(div().size(px(8.)).rounded_full().bg(rgb(if self.watching > 0 { p.recording } else { p.faint })))
            .child(div().text_size(px(13.)).text_color(rgb(p.muted)).child(if self.watching > 0 { ta!("share-watching", n = self.watching) } else { t("share-none-watching").to_string() }));
        let details = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(div().flex().flex_col().gap(px(6.)).child(div().text_size(px(13.)).text_color(rgb(p.muted)).child(t("share-open"))).child(addresses))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(div().text_size(px(13.)).text_color(rgb(p.muted)).child(t("share-code")))
                    .child(div().font_family(theme::MONO).text_size(px(30.)).font_weight(FontWeight::SEMIBOLD).text_color(rgb(p.text)).child(code)),
            )
            .child(watching);
        let copied = self.share_just_copied();
        let copy = if copied {
            pill("share-copy", t("share-copied"), Look::Plain, None).text_color(rgb(p.accent))
        } else {
            pill("share-copy", t("share-copy-link"), Look::Plain, Some("ipad/copy")).on_click(cx.listener(move |this, _, _, cx| this.copy_share_link(link.clone(), cx)))
        };
        div()
            .id("share-panel")
            .flex()
            .flex_col()
            .gap(px(18.))
            .child(div().flex().gap(px(22.)).child(qr_card).child(details))
            .child(privacy)
            .child(div().flex().justify_end().gap(px(10.)).child(copy).child(pill("share-stop", t("share-stop"), Look::Ending, None).on_click(cx.listener(|this, _, _, cx| this.stop_sharing(cx)))))
    }

    /// Import, on the iPad: a rounded place to choose a file from Files, a rounded field for an arXiv paper with its
    /// button, and the same for each step after (reading, ready to place, a model to fetch first).
    pub(super) fn ipad_import_body(&self, cx: &mut Context<Self>) -> gpui_kit::Stateful<gpui_kit::Div> {
        let p = pal();
        let hint = |text: String| div().text_size(px(13.)).line_height(gpui_kit::relative(1.4)).text_color(rgb(p.muted)).child(text);
        let panel = div().id("import-panel").flex_1().overflow_y_scroll().flex().flex_col().gap(px(14.));
        let buttons = |children: Vec<AnyElement>| div().flex().flex_wrap().justify_end().gap(px(10.)).children(children);
        let can_add_here = self.live.read_only.is_none();
        match &self.import {
            ImportState::NeedsVision { add_on, path } => {
                let (fetch, retry) = (add_on.clone(), path.clone());
                let downloading = self.download.as_ref().filter(|d| d.id == add_on.id);
                let progress = downloading.map(|d| match &d.progress {
                    Some(p) if p.verifying => t("checking-what-was-already-downloaded").to_string(),
                    Some(p) if p.testing => t("finding-the-fastest-source").to_string(),
                    Some(p) => format!("{} / {}", gigabytes(p.done), gigabytes(p.total)),
                    None => t("starting").to_string(),
                });
                let ready = alcyo_models::installed(add_on, &self.models);
                let mut actions = vec![pill("import-cancel", t("cancel"), Look::Plain, None).on_click(cx.listener(|this, _, _, cx| this.stop_import(cx))).into_any_element()];
                if ready {
                    actions.push(pill("read-image", t("read-the-image"), Look::Main, None).on_click(cx.listener(move |this, _, _, cx| this.import_image_file(retry.clone(), cx))).into_any_element());
                } else if downloading.is_none() && self.download.is_none() {
                    actions.push(pill("get-vision", format!("{} · {}", t("download"), gigabytes(add_on.size)), Look::Main, Some("ipad/download")).on_click(cx.listener(move |this, _, _, cx| this.start_download(fetch.clone(), cx))).into_any_element());
                }
                panel
                    .child(div().text_size(px(15.)).text_color(rgb(p.text)).child(t("to-read-the-text-in")))
                    .child(hint(if downloading.is_some() { add_on.label.to_string() } else { t("one-download-and-then-images").to_string() }))
                    .when_some(progress, |d, text| d.child(div().text_size(px(13.)).font_family(theme::MONO).text_color(rgb(p.muted)).child(text)))
                    .child(buttons(actions))
            }
            ImportState::Reading { progress, .. } => panel
                .child(div().text_size(px(15.)).text_color(rgb(p.text)).child(progress.clone()))
                .child(hint(t("you-can-carry-on-with").to_string()))
                .child(buttons(vec![pill("stop-import", t("stop"), Look::Ending, None).on_click(cx.listener(|this, _, _, cx| this.stop_import(cx))).into_any_element()])),
            ImportState::Ready(preview) => {
                let mut actions = vec![pill("import-cancel", t("cancel"), Look::Plain, None)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.import_pages.update(cx, |state, cx| state.set_value("", window, cx));
                        this.stop_import(cx);
                    }))
                    .into_any_element()];
                if can_add_here {
                    actions.push(pill("import-here", t("add-to-this-meeting"), Look::Plain, None).on_click(cx.listener(|this, _, window, cx| this.place_import(false, window, cx))).into_any_element());
                }
                actions.push(pill("import-new", t("new-meeting"), Look::Main, Some("ipad/plus")).on_click(cx.listener(|this, _, window, cx| this.place_import(true, window, cx))).into_any_element());
                panel
                    .child(div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).text_color(rgb(p.text)).child(format!("{} · {}", preview.name, ta!("board-count", n = preview.pages.len()))))
                    .child(hint(preview.ready.clone()))
                    .children(preview.details.iter().map(|d| div().text_size(px(13.)).text_color(rgb(p.warning)).child(d.clone())))
                    .when(preview.pages.len() > 1, |d| {
                        d.child(hint(t("pages-to-import").to_string())).child(div().h(px(44.)).flex().items_center().px(px(14.)).rounded(px(14.)).bg(well()).child(div().flex_1().min_w_0().child(Input::new(&self.import_pages).appearance(false))))
                    })
                    .child(buttons(actions))
            }
            state => panel
                .when_some(if let ImportState::Failed(error) = state { Some(error.clone()) } else { None }, |d, error| d.child(div().text_size(px(13.5)).text_color(rgb(p.danger)).child(error)))
                .child(hint(t("a-pdf-page-or-an").to_string()))
                // Where a file comes from: Files, through the system's picker. The iPad has no drop on the window.
                .child(
                    div()
                        .id("import-pick")
                        .py(px(26.))
                        .px(px(20.))
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap(px(12.))
                        .rounded(px(20.))
                        .border_1()
                        .border_dashed()
                        .border_color(rgb(p.faint))
                        .bg(well())
                        .cursor_pointer()
                        .on_click(cx.listener(|this, _, _, cx| this.pick_import_file(cx)))
                        .child(div().text_color(rgb(p.muted)).child(icon("ipad/import").size(px(28.))))
                        .child(pill("pick-import", t("choose-a-file"), Look::Main, None).on_click(cx.listener(|this, _, _, cx| {
                            cx.stop_propagation();
                            this.pick_import_file(cx);
                        })))
                        .child(div().text_center().text_size(px(12.5)).text_color(rgb(p.faint)).child(t("ipad-import-from-files"))),
                )
                .child(div().pt(px(4.)).text_size(px(13.)).text_color(rgb(p.muted)).child(t("an-arxiv-paper-by-link")))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .child(div().flex_1().min_w_0().h(px(44.)).flex().items_center().px(px(14.)).rounded(px(14.)).bg(well()).child(div().flex_1().min_w_0().child(Input::new(&self.arxiv_input).appearance(false))))
                        .child(pill("import-arxiv", t("download-and-read"), Look::Plain, Some("ipad/download")).on_click(cx.listener(|this, _, window, cx| this.import_arxiv(window, cx)))),
                ),
        }
    }
}
