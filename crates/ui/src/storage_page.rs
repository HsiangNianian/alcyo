//! Settings › Storage (#357), on the iPad and the desktop: what the app keeps, item by item, each to delete by itself,
//! and "Delete all data" behind a confirmation in which the meetings are a choice of their own, off unless ticked. The
//! items and their sizes are found off the window's thread (crate::storage). A child of `app`.
//!
//! What is listed is looked at again whenever the page comes into view, and whenever something elsewhere may have
//! changed what is kept (`storage_changed`): a compile cache cleared on the Board model page, another model chosen,
//! a model downloaded, a model's first start compiled. The Board model page reads the disk as it draws, so what is
//! deleted here shows there by itself.

use super::*;
use crate::storage::{self, Item, Kind};
use gpui_kit::{AnyElement, ElementId, FontWeight};

/// Where Storage stands.
#[derive(Default)]
pub(crate) struct StorageState {
    /// What was found, once looked at.
    items: Option<Vec<Item>>,
    scanning: bool,
    /// Something is being deleted.
    busy: bool,
    /// How much the last deletion gave back.
    freed: Option<u64>,
    /// The confirmation for "Delete all data" is up, with the meetings ticked or not.
    confirm: Option<bool>,
    note: Option<String>,
    /// Counts what may have changed on disk since; a look begun before the last change is taken again.
    changed: u64,
    /// The `changed` the items were looked at for.
    looked: Option<u64>,
    /// The window's frames, and the last one the page was drawn in: a page not drawn in the frame before has just
    /// come into view.
    frame: u64,
    drawn: u64,
}

impl StorageState {
    /// Something may have changed what is kept: look again once the page is drawn.
    pub(crate) fn changed(&mut self) {
        self.changed += 1;
    }

    /// A frame of the window begins.
    pub(crate) fn next_frame(&mut self) {
        self.frame += 1;
    }

    /// The page is drawn: whether to look at what is kept now, because it was never looked at, has changed since,
    /// or the page has just come into view.
    fn due(&mut self) -> bool {
        if self.drawn != self.frame && self.drawn + 1 != self.frame {
            self.changed += 1;
            // What was freed on an earlier visit is no news now.
            self.freed = None;
            self.note = None;
        }
        self.drawn = self.frame;
        !self.scanning && self.looked != Some(self.changed)
    }

    /// A look begins; it stands for what was there at the change it returns.
    fn begin(&mut self) -> u64 {
        self.scanning = true;
        self.changed
    }

    /// A look begun at `at` is back. When something changed meanwhile, `due` asks for another.
    fn found(&mut self, at: u64, items: Vec<Item>) {
        self.scanning = false;
        self.items = Some(items);
        self.looked = Some(at);
    }
}

impl Workbench {
    fn storage_places(&self) -> storage::Places {
        let data = self.models.parent().unwrap_or(&self.models).to_path_buf();
        storage::Places {
            metal: compile_cache::system_dirs(&data),
            data,
            models: self.models.clone(),
            voice: self.voice_dir.clone(),
            packs: Some(self.packs.places.clone()),
            search: self.search_dir.clone(),
            temp: std::env::temp_dir(),
            temp_is_own: cfg!(target_os = "ios"),
        }
    }

    /// Something elsewhere may have changed what is kept: Storage looks again when it is next drawn, or at once
    /// when it is on screen.
    pub(crate) fn storage_changed(&mut self, cx: &mut Context<Self>) {
        self.storage.changed();
        cx.notify();
    }

    /// Look at what is kept again, off the window's thread, when the page is drawn and that is due.
    fn scan_storage(&mut self, cx: &mut Context<Self>) {
        if !self.storage.due() {
            return;
        }
        let at = self.storage.begin();
        let places = self.storage_places();
        let found = cx.background_executor().spawn(async move { storage::survey(&places) });
        cx.spawn(async move |this, cx| {
            let items = found.await;
            let _ = this.update(cx, |this, cx| {
                this.storage.found(at, items);
                cx.notify();
            });
        })
        .detach();
    }

    /// Whether an item is in use now: the board model running, or a compile cache it uses.
    fn storage_in_use(&self, item: &Item) -> bool {
        let (running, busy) = match self.session().local_model_up() {
            Ok(model) => (model, false),
            Err(()) => (None, true),
        };
        match &item.kind {
            Kind::BoardModel(file) | Kind::ImageReader(file) => busy || running.as_ref().and_then(|p| p.file_name()).is_some_and(|n| n.to_string_lossy() == *file),
            Kind::CompileCache(_) => item.cache.as_ref().is_some_and(|entry| compile_cache::in_use(entry, running.as_deref(), busy)),
            _ => false,
        }
    }

    /// What to call an item.
    fn storage_name(item: &Item) -> String {
        let label = |file: &str| alcyo_models::catalog().into_iter().find(|m| m.file == file).map_or_else(|| file.trim_end_matches(".gguf").to_string(), |m| m.label.to_string());
        match &item.kind {
            Kind::BoardModel(file) => label(file),
            Kind::ImageReader(file) => ta!("storage-image-reader", name = file.trim_end_matches(".gguf").to_string()),
            Kind::SearchModel => t("storage-search-model").to_string(),
            Kind::Unfinished(file) => ta!("storage-unfinished", name = label(file)),
            Kind::CompileCache(compile_cache::Kind::Model(file)) => ta!("storage-compile-cache-of", name = label(file)),
            Kind::CompileCache(compile_cache::Kind::Older) => t("compile-cache-older").to_string(),
            Kind::CompileCache(compile_cache::Kind::System) => t("compile-cache-system").to_string(),
            Kind::Speech => t("storage-speech").to_string(),
            Kind::VoicePack(id) => ta!("vp-storage-name", name = alcyo_models::voice::Pack::from_id(id).map_or("", voice_packs::pack_name).to_string()),
            Kind::SearchIndex => t("storage-search-index").to_string(),
            Kind::Meetings => t("storage-meetings").to_string(),
            Kind::Temporary => t("storage-temporary").to_string(),
            Kind::Logs => t("storage-logs").to_string(),
        }
    }

    /// What an item is, under its name.
    fn storage_hint(item: &Item) -> &'static str {
        match &item.kind {
            Kind::BoardModel(_) => t("storage-hint-board-model"),
            Kind::ImageReader(_) => t("storage-hint-image-reader"),
            Kind::SearchModel | Kind::Speech if item.built_in => t("storage-built-in"),
            Kind::SearchModel => t("storage-hint-search-model"),
            Kind::Unfinished(_) => t("storage-hint-unfinished"),
            Kind::CompileCache(_) => t("storage-hint-compile-cache"),
            Kind::Speech => t("storage-built-in"),
            Kind::VoicePack(_) => t("vp-storage-downloaded"),
            Kind::SearchIndex => t("storage-hint-search-index"),
            Kind::Meetings => t("storage-hint-meetings"),
            Kind::Temporary => t("storage-hint-temporary"),
            Kind::Logs => t("storage-hint-logs"),
        }
    }

    /// Stop the local model if one runs, through the session's own pause (as when the iPad goes to the background),
    /// then do `then` once it has given its memory and files back; let it go again after. Gives up after half a minute.
    fn with_local_stopped(&mut self, cx: &mut Context<Self>, then: impl FnOnce(&mut Self, &mut Context<Self>) + 'static) {
        if matches!(self.session().local_model_up(), Ok(None)) {
            then(self, cx);
            return;
        }
        self.session().pause_local_because("storage delete");
        cx.spawn(async move |this, cx| {
            let started = Instant::now();
            loop {
                cx.background_executor().timer(Duration::from_millis(200)).await;
                let stopped = this.update(cx, |this, _| matches!(this.session().local_model_up(), Ok(None))).unwrap_or(true);
                if stopped || started.elapsed() > Duration::from_secs(30) {
                    break;
                }
            }
            let _ = this.update(cx, |this, cx| {
                then(this, cx);
                this.session().resume_local();
            });
        })
        .detach();
    }

    /// Delete `items` off the window's thread, then say how much came back and look again. A board model deleted
    /// means the model in use may now be another, or none.
    fn storage_delete(&mut self, items: Vec<Item>, forget_secrets: bool, cx: &mut Context<Self>) {
        let (packs, items): (Vec<Item>, Vec<Item>) = items.into_iter().partition(|i| matches!(i.kind, Kind::VoicePack(_)));
        for item in packs {
            if let Kind::VoicePack(id) = &item.kind {
                if let Some(pack) = alcyo_models::voice::Pack::from_id(id) {
                    self.packs_delete(pack, cx);
                }
            }
        }
        self.storage.busy = true;
        self.storage.freed = None;
        self.storage.note = None;
        let meetings = items.iter().any(|i| i.kind == Kind::Meetings);
        let models = items.iter().any(|i| matches!(i.kind, Kind::BoardModel(_) | Kind::ImageReader(_)));
        let done = cx.background_executor().spawn(async move {
            let mut freed = 0;
            let mut failed = None;
            for item in &items {
                match storage::delete(item) {
                    Ok(bytes) => freed += bytes,
                    Err(e) => failed = Some(format!("{e:#}")),
                }
            }
            (freed, failed)
        });
        cx.spawn(async move |this, cx| {
            let (freed, failed) = done.await;
            let _ = this.update(cx, |this, cx| {
                this.storage.busy = false;
                this.storage.freed = Some(freed);
                this.storage.note = failed;
                if forget_secrets {
                    // The API key and the proxy password, from the keychain (or the encrypted file without one).
                    this.form.saved.api_key.clear();
                    this.form.saved.proxy_password.clear();
                    if let Err(e) = this.form.saved.save() {
                        this.storage.note = Some(format!("{e:#}"));
                    }
                }
                if meetings {
                    this.library = this.store.as_ref().and_then(|s| s.list().ok()).unwrap_or_default();
                    this.search = None;
                }
                if models || forget_secrets {
                    this.apply_backend();
                }
                this.storage.changed();
                cx.notify();
            });
        })
        .detach();
    }

    /// Delete one item, stopping the local model first when it is that model's.
    fn storage_delete_one(&mut self, kind: Kind, cx: &mut Context<Self>) {
        // A voice pack goes through the packs, which stop keeping it (and Chinese takes its live captions).
        if let Kind::VoicePack(id) = &kind {
            if let Some(pack) = alcyo_models::voice::Pack::from_id(id) {
                self.packs_delete(pack, cx);
            }
            return;
        }
        let Some(item) = self.storage.items.as_ref().and_then(|items| items.iter().find(|i| i.kind == kind)).cloned() else { return };
        if !item.deletable() {
            return;
        }
        if self.storage_in_use(&item) {
            self.with_local_stopped(cx, move |this, cx| this.storage_delete(vec![item], false, cx));
        } else {
            self.storage_delete(vec![item], false, cx);
        }
    }

    /// "Delete all data", confirmed: the local model stops first, then everything that can be deleted goes, the
    /// meetings only when ticked, and the secrets kept for the remote model and the proxy.
    fn storage_delete_all(&mut self, meetings: bool, cx: &mut Context<Self>) {
        self.storage.confirm = None;
        let Some(items) = self.storage.items.clone() else { return };
        self.with_local_stopped(cx, move |this, cx| {
            let chosen: Vec<Item> = storage::delete_all_selection(&items, meetings, &|_| false).into_iter().cloned().collect();
            this.storage_delete(chosen, true, cx);
        });
    }

    /// The rows, as (item, name, what it is or that it is in use, its size, whether in use), for both pages. A model's
    /// compile cache that only notes the model was built (the iPad's; its compiled code is counted in Metal's row) says
    /// so as the Board model page does, and no size of a few bytes.
    fn storage_rows(&self) -> Vec<(Item, String, String, Option<String>, bool)> {
        self.storage.items.clone().unwrap_or_default().into_iter().map(|item| {
            let used = self.storage_in_use(&item);
            let name = Self::storage_name(&item);
            let noted = item.cache.as_ref().filter(|entry| compile_cache::only_noted(entry, cfg!(target_os = "ios")));
            let (hint, size) = match noted {
                Some(entry) => (Self::compile_cache_detail(entry), None),
                None if used && !item.built_in => (t("storage-in-use").to_string(), Some(storage::human(item.bytes))),
                None => (Self::storage_hint(&item).to_string(), Some(storage::human(item.bytes))),
            };
            (item, name, hint, size, used)
        }).collect()
    }

    /// Storage on the iPad: groups of rounded rows, pills, and the confirmation as a card of its own.
    pub(super) fn sheet_storage(&mut self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        self.scan_storage(cx);
        let p = pal();
        let rows = self.storage_rows();
        let total: u64 = rows.iter().map(|(i, ..)| i.bytes).sum();
        let busy = self.storage.busy;
        let row = |i: usize, (item, name, hint, size, used): (Item, String, String, Option<String>, bool), cx: &mut Context<Self>| {
            let kind = item.kind.clone();
            let right: AnyElement = if item.built_in {
                div().flex_none().text_size(px(13.5)).text_color(rgb(p.faint)).child(t("storage-built-in")).into_any_element()
            } else if used && matches!(item.kind, Kind::CompileCache(_)) {
                div().flex_none().text_size(px(13.5)).text_color(rgb(p.faint)).child(t("compile-cache-in-use")).into_any_element()
            } else if item.deletable() && !busy {
                ipad_dialogs::pill(ElementId::Name(format!("storage-delete-{i}").into()), if used { t("storage-stop-and-delete") } else { t("delete") }, ipad_dialogs::Look::Ending, None)
                    .h(px(34.))
                    .px(px(14.))
                    .text_size(px(13.5))
                    .on_click(cx.listener(move |this, _, _, cx| this.storage_delete_one(kind.clone(), cx)))
                    .into_any_element()
            } else {
                div().into_any_element()
            };
            div()
                .min_h(px(60.))
                .flex()
                .items_center()
                .gap(px(12.))
                .px(px(16.))
                .py(px(8.))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .child(div().text_size(px(15.)).font_weight(FontWeight::MEDIUM).overflow_hidden().text_ellipsis().whitespace_nowrap().child(name))
                        .child(div().text_size(px(12.5)).text_color(rgb(p.faint)).child(size.map_or_else(|| hint.clone(), |size| format!("{size} · {hint}")))),
                )
                .child(right)
                .into_any_element()
        };
        let group = |rows: Vec<AnyElement>| {
            div().flex().flex_col().rounded(px(16.)).bg(rgb(p.chrome)).overflow_hidden().children(rows.into_iter().enumerate().map(|(i, r)| div().when(i > 0, |d| d.border_t_1().border_color(rgb(p.line))).child(r)))
        };
        let caption = |text: String| div().px(px(4.)).font_family(theme::MONO).text_size(px(10.5)).text_color(rgb(p.faint)).child(text.to_uppercase());
        let mut out = vec![];
        let status = match (&self.storage.items, self.storage.freed, &self.storage.note) {
            (_, _, Some(note)) => Some((note.clone(), true)),
            (_, Some(freed), None) => Some((ta!("storage-freed", size = storage::human(freed)), false)),
            (None, ..) => Some((t("storage-looking").to_string(), false)),
            _ if busy => Some((t("storage-deleting").to_string(), false)),
            _ => None,
        };
        out.push(div().flex().items_start().gap(px(16.)).px(px(4.)).child(div().flex_1().min_w_0().text_size(px(14.)).line_height(gpui_kit::relative(1.45)).text_color(rgb(p.muted)).child(t("storage-intro"))).child(div().flex_none().font_family(theme::MONO).text_size(px(13.)).text_color(rgb(p.text)).child(storage::human(total))).into_any_element());
        if let Some((text, failed)) = status {
            out.push(div().px(px(4.)).text_size(px(13.)).text_color(rgb(if failed { p.danger } else { p.accent })).child(text).into_any_element());
        }
        let sections: [(&str, fn(&Kind) -> bool); 4] = [
            (t("storage-models"), |k| matches!(k, Kind::BoardModel(_) | Kind::ImageReader(_) | Kind::SearchModel | Kind::Unfinished(_) | Kind::Speech | Kind::VoicePack(_))),
            (t("compile-cache"), |k| matches!(k, Kind::CompileCache(_))),
            (t("storage-your-data"), |k| matches!(k, Kind::Meetings)),
            (t("storage-other"), |k| matches!(k, Kind::SearchIndex | Kind::Temporary | Kind::Logs)),
        ];
        let mut index = 0;
        for (title, belongs) in sections {
            let mine: Vec<AnyElement> = rows.iter().filter(|(i, ..)| belongs(&i.kind)).cloned().map(|r| {
                index += 1;
                row(index, r, cx)
            }).collect();
            if !mine.is_empty() {
                out.push(div().flex().flex_col().gap(px(8.)).child(caption(title.to_string())).child(group(mine)).into_any_element());
            }
        }
        // Delete all, behind its confirmation.
        out.push(match self.storage.confirm {
            None => div()
                .flex()
                .justify_center()
                .pt(px(8.))
                .child(ipad_dialogs::pill("storage-delete-all", t("storage-delete-all"), ipad_dialogs::Look::Ending, Some("ipad/trash")).when(!busy && self.storage.items.is_some(), |d| d.on_click(cx.listener(|this, _, _, cx| {
                    this.storage.confirm = Some(false);
                    cx.notify();
                }))))
                .into_any_element(),
            Some(meetings) => self.storage_confirm(meetings, true, cx),
        });
        out
    }

    /// The confirmation for "Delete all data": what goes, the meetings as a box to tick, and the secrets.
    fn storage_confirm(&self, meetings: bool, ipad: bool, cx: &mut Context<Self>) -> AnyElement {
        let p = pal();
        let check = div()
            .id("storage-confirm-meetings")
            .min_h(px(44.))
            .flex()
            .items_center()
            .gap(px(12.))
            .cursor_pointer()
            .child(div().flex_none().size(px(22.)).rounded(px(6.)).border_2().border_color(rgb(if meetings { p.danger } else { p.faint })).when(meetings, |d| d.bg(rgb(p.danger)).flex().items_center().justify_center().text_color(rgb(0xff_ff_ff)).child(icon("ipad/check").size(px(14.)))))
            .child(div().flex_1().text_size(px(14.5)).child(t("storage-confirm-meetings")))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.storage.confirm = Some(!meetings);
                cx.notify();
            }));
        let lines = [t("storage-confirm-models"), t("storage-confirm-secrets"), if meetings { t("storage-confirm-meetings-go") } else { t("storage-confirm-meetings-stay") }];
        let cancel = |this: &mut Self, cx: &mut Context<Self>| {
            this.storage.confirm = None;
            cx.notify();
        };
        let actions: Vec<AnyElement> = if ipad {
            vec![
                ipad_dialogs::pill("storage-confirm-cancel", t("cancel"), ipad_dialogs::Look::Plain, None).on_click(cx.listener(move |this, _, _, cx| cancel(this, cx))).into_any_element(),
                ipad_dialogs::pill("storage-confirm-go", t("storage-delete-all"), ipad_dialogs::Look::Ending, None).on_click(cx.listener(move |this, _, _, cx| this.storage_delete_all(meetings, cx))).into_any_element(),
            ]
        } else {
            vec![
                button("storage-confirm-cancel", t("cancel")).on_click(cx.listener(move |this, _, _, cx| cancel(this, cx))).into_any_element(),
                Quiet::new("storage-confirm-go").danger().label(t("storage-delete-all")).on_click(cx.listener(move |this, _, _, cx| this.storage_delete_all(meetings, cx))).into_any_element(),
            ]
        };
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .p(px(18.))
            .rounded(px(if ipad { 18. } else { 3. }))
            .border_1()
            .border_color(alpha_of(p.danger, 0.4))
            .bg(alpha_of(p.danger, if theme::is_dark() { 0.12 } else { 0.05 }))
            .child(div().text_size(px(16.)).font_weight(FontWeight::SEMIBOLD).child(t("storage-confirm-title")))
            .children(lines.into_iter().map(|line| div().flex().gap(px(8.)).text_size(px(13.5)).line_height(gpui_kit::relative(1.4)).text_color(rgb(p.muted)).child(div().flex_none().child("·")).child(div().flex_1().min_w_0().child(line))))
            .child(check)
            .child(div().flex().justify_end().gap(px(10.)).children(actions))
            .into_any_element()
    }

    /// Storage on the desktop, in its settings' own style: rows of name and what it is, the size and a button.
    pub(super) fn desktop_storage(&mut self, cx: &mut Context<Self>) -> AnyElement {
        self.scan_storage(cx);
        let rows = self.storage_rows();
        let total: u64 = rows.iter().map(|(i, ..)| i.bytes).sum();
        let busy = self.storage.busy;
        let lines = rows.into_iter().enumerate().map(|(i, (item, name, hint, size, used))| {
            let kind = item.kind.clone();
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap_6()
                .px_4()
                .py_3()
                .border_b_1()
                .border_color(rgb(pal().line))
                .child(div().flex_1().min_w_0().flex().flex_col().gap(px(2.)).child(div().text_ui().child(name)).child(div().text_small().text_color(rgb(pal().muted)).child(hint)))
                .child(div().font_family(theme::MONO).text_small().text_color(rgb(pal().muted)).child(size.unwrap_or_default()))
                .child(if item.built_in || (used && matches!(item.kind, Kind::CompileCache(_))) {
                    div().w(px(110.)).into_any_element()
                } else {
                    button(("storage-delete", i), if used { t("storage-stop-and-delete") } else { t("delete") })
                        .disabled(busy || !item.deletable())
                        .on_click(cx.listener(move |this, _, _, cx| this.storage_delete_one(kind.clone(), cx)))
                        .into_any_element()
                })
        });
        let status = match (&self.storage.items, self.storage.freed, &self.storage.note) {
            (_, _, Some(note)) => Some(note.clone()),
            (_, Some(freed), None) => Some(ta!("storage-freed", size = storage::human(freed))),
            (None, ..) => Some(t("storage-looking").to_string()),
            _ if busy => Some(t("storage-deleting").to_string()),
            _ => None,
        };
        div()
            .flex()
            .flex_col()
            .gap_4()
            .pt_6()
            .child(div().flex().gap_6().child(div().flex_1().min_w_0().text_ui().text_color(rgb(pal().muted)).child(t("storage-intro"))).child(div().font_family(theme::MONO).text_ui().child(storage::human(total))))
            .when_some(status, |d, s| d.child(div().text_small().text_color(rgb(pal().accent)).child(s)))
            .child(div().flex().flex_col().border_1().border_color(rgb(pal().line)).children(lines))
            .child(match self.storage.confirm {
                None => div()
                    .child(Quiet::new("storage-delete-all").danger().label(t("storage-delete-all")).disabled(busy || self.storage.items.is_none()).on_click(cx.listener(|this, _, _, cx| {
                        this.storage.confirm = Some(false);
                        cx.notify();
                    })))
                    .into_any_element(),
                Some(meetings) => self.storage_confirm(meetings, false, cx),
            })
            .into_any_element()
    }
}

fn alpha_of(color: u32, a: f32) -> Hsla {
    let mut c: Hsla = rgb(color).into();
    c.a = a;
    c
}

#[cfg(test)]
mod tests {
    use super::StorageState;

    /// Draw the page in the next frame, and say whether it looked again.
    fn draw(state: &mut StorageState) -> bool {
        state.next_frame();
        state.due()
    }

    /// The page looks at what is kept when it first shows, again whenever it comes back into view, and again when
    /// something elsewhere changed it (a cache cleared on the Board model page), but not at every frame.
    #[test]
    fn storage_looks_again_when_shown_or_changed() {
        let mut state = StorageState::default();
        assert!(draw(&mut state), "first shown");
        let at = state.begin();
        assert!(!draw(&mut state), "a look is under way");
        state.found(at, vec![]);
        assert!(!draw(&mut state), "nothing changed while it stayed in view");
        // Settings › Board model is shown for a while and clears a cache there.
        state.next_frame();
        state.next_frame();
        state.changed();
        assert!(draw(&mut state), "back on Storage after a change elsewhere");
        look(&mut state);
        // Away and back with nothing changed: the disk is looked at anyway, as anything could have.
        state.next_frame();
        assert!(draw(&mut state), "coming back into view looks again");
        look(&mut state);
        // A change while the page is in view (a model's first start compiled) is seen at once.
        state.changed();
        assert!(draw(&mut state));
        // Drawn twice in one frame is still one look.
        let at = state.begin();
        state.found(at, vec![]);
        assert!(!state.due() && !state.due());
    }

    /// A look, from beginning to end, with nothing found.
    fn look(state: &mut StorageState) {
        let at = state.begin();
        state.found(at, vec![]);
    }

    /// A look begun before a change is not taken for what is there after it.
    #[test]
    fn a_look_begun_before_a_change_is_taken_again() {
        let mut state = StorageState::default();
        assert!(draw(&mut state));
        let at = state.begin();
        state.changed();
        state.found(at, vec![]);
        assert!(draw(&mut state), "the look came back stale");
        look(&mut state);
        assert!(!draw(&mut state));
    }
}
