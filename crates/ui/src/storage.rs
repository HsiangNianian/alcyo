//! What the app keeps on disk, item by item, for Settings › Storage (#357): how much each takes, whether it can be
//! deleted, and deleting it, one at a time or all at once.
//!
//! Everything is found on disk rather than listed in advance: every `.gguf` beside the models is a row of its own
//! (a board model, an image reader, the search model), whatever model it is, as are downloads left unfinished. What
//! came with the app (the speech models, the bundled search model) is shown as built in and is not deleted. Models
//! and caches can always be fetched or built again; meetings are the person's own, and are deleted only when asked
//! for by name.
//!
//! Nothing here touches the window: `survey` and `delete` are slow on a big disk and run off the UI thread.

use crate::compile_cache;
use std::path::PathBuf;

/// Where the app keeps things, and what came with it.
#[derive(Clone, Debug)]
pub struct Places {
    /// The data folder: meetings, the inbox, search indexes, the compile cache.
    pub data: PathBuf,
    /// The models' folder.
    pub models: PathBuf,
    /// The speech models that came with the app.
    pub voice: PathBuf,
    /// The voice packs downloaded, each a row.
    pub packs: Option<alcyo_models::voice::Places>,
    /// The meeting search model that came with the app.
    pub search: PathBuf,
    /// The app's own temporary folder (the iPad's `tmp`), listed only when `temp_is_own`.
    pub temp: PathBuf,
    pub temp_is_own: bool,
    /// Metal's caches (the iPad); see compile_cache.
    pub metal: Vec<PathBuf>,
}

/// What an item is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A board model's file, by its name.
    BoardModel(String),
    /// A board model's image reader (its projector).
    ImageReader(String),
    /// The meeting search model.
    SearchModel,
    /// A download left unfinished.
    Unfinished(String),
    /// What the graphics compiled (compile_cache).
    CompileCache(compile_cache::Kind),
    /// What came with the app for speech: the voice detector.
    Speech,
    /// A voice pack downloaded, by its id.
    VoicePack(String),
    /// The meeting search indexes, made again from the meetings.
    SearchIndex,
    /// The meetings, with their sound and pages, and the inbox.
    Meetings,
    /// Temporary files, as exports waiting to be shared.
    Temporary,
    /// The app log and the events behind a diagnostic bundle (crates/diag), and the models' hashes kept for it.
    Logs,
}

/// One row of Storage.
#[derive(Clone, Debug)]
pub struct Item {
    pub kind: Kind,
    /// What makes it up on disk.
    pub paths: Vec<PathBuf>,
    pub bytes: u64,
    /// Came with the app: shown, never deleted.
    pub built_in: bool,
    /// For a compile cache entry, the entry itself, to clear it as compile_cache does.
    pub cache: Option<compile_cache::Entry>,
}

impl Item {
    pub fn deletable(&self) -> bool {
        !self.built_in && self.bytes > 0
    }
}

/// Everything the app keeps, item by item, with its size.
pub fn survey(places: &Places) -> Vec<Item> {
    let mut out = vec![];
    let item = |kind, paths: Vec<PathBuf>, built_in| {
        let bytes = paths.iter().map(|p| compile_cache::size(p)).sum();
        Item { kind, paths, bytes, built_in, cache: None }
    };
    // Each model file, as found.
    let mut files: Vec<PathBuf> = std::fs::read_dir(&places.models).map(|e| e.flatten().map(|e| e.path()).filter(|p| p.is_file()).collect()).unwrap_or_default();
    files.sort();
    let search_file = alcyo_models::embedding().file;
    let readers: Vec<String> = alcyo_models::catalog().iter().filter_map(|m| alcyo_models::vision_for(&m.file)).map(|m| m.file).collect();
    for path in files {
        let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
        let kind = if name.ends_with(".part") {
            Kind::Unfinished(name.trim_end_matches(".part").to_string())
        } else if !name.ends_with(".gguf") {
            continue;
        } else if name == search_file {
            Kind::SearchModel
        } else if readers.contains(&name) || name.contains("mmproj") {
            Kind::ImageReader(name)
        } else {
            Kind::BoardModel(name)
        };
        out.push(item(kind, vec![path], false));
    }
    if places.search.join(&search_file).is_file() {
        out.push(item(Kind::SearchModel, vec![places.search.join(&search_file)], true));
    }
    for entry in compile_cache::entries(&compile_cache::root(&places.models), &places.metal) {
        out.push(Item { kind: Kind::CompileCache(entry.kind.clone()), paths: std::iter::once(entry.path.clone()).chain(entry.more.clone()).collect(), bytes: entry.bytes, built_in: false, cache: Some(entry) });
    }
    if let Some(packs) = &places.packs {
        for pack in alcyo_models::voice::Pack::ALL {
            let bytes = packs.on_disk(pack);
            // Live captions are said with Chinese, in its row.
            if bytes == 0 || pack == alcyo_models::voice::Pack::Live && packs.on_disk(alcyo_models::voice::Pack::Zh) > 0 {
                continue;
            }
            let folders = std::iter::once(pack).chain((pack == alcyo_models::voice::Pack::Zh).then_some(alcyo_models::voice::Pack::Live));
            let paths: Vec<PathBuf> = folders.map(|p| packs.packs.join(p.folder())).filter(|p| p.exists()).collect();
            let bytes = bytes + if pack == alcyo_models::voice::Pack::Zh { packs.on_disk(alcyo_models::voice::Pack::Live) } else { 0 };
            out.push(Item { kind: Kind::VoicePack(pack.id().to_string()), paths, bytes, built_in: false, cache: None });
        }
    }
    if places.voice.is_dir() && places.packs.as_ref().is_none_or(|p| p.packs != places.voice) {
        out.push(item(Kind::Speech, vec![places.voice.clone()], true));
    }
    let index = places.data.join("search");
    if index.is_dir() {
        out.push(item(Kind::SearchIndex, vec![index], false));
    }
    let meetings: Vec<PathBuf> = ["meetings", "inbox"].iter().map(|d| places.data.join(d)).filter(|p| p.exists()).collect();
    if !meetings.is_empty() {
        out.push(item(Kind::Meetings, meetings, false));
    }
    let logs = alcyo_diag::logs_dir(&places.data);
    if logs.is_dir() {
        out.push(item(Kind::Logs, vec![logs], false));
    }
    let temp = temporary(places);
    if !temp.is_empty() {
        out.push(item(Kind::Temporary, temp, false));
    }
    out
}

/// The app's temporary files: everything in its own temporary folder (the iPad's), as exports waiting to be shared.
fn temporary(places: &Places) -> Vec<PathBuf> {
    // A desktop writes exports where the person chose, and its temporary folder is everyone's: nothing to list there.
    if !places.temp_is_own {
        return vec![];
    }
    let Ok(entries) = std::fs::read_dir(&places.temp) else { return vec![] };
    let mut out: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    out.sort();
    out
}

/// Delete an item, and say how much space that gave back. Built-in items are left alone.
pub fn delete(item: &Item) -> std::io::Result<u64> {
    if item.built_in {
        return Ok(0);
    }
    if let Some(entry) = &item.cache {
        compile_cache::clear(entry)?;
        return Ok(item.bytes);
    }
    for path in &item.paths {
        if path.is_dir() {
            std::fs::remove_dir_all(path)?;
        } else if path.exists() {
            std::fs::remove_file(path)?;
        }
    }
    Ok(item.bytes)
}

/// What "Delete all data" takes: everything that can be deleted, the meetings only when asked for, and nothing that
/// is in use (`in_use` says which).
pub fn delete_all_selection<'a>(items: &'a [Item], meetings: bool, in_use: &dyn Fn(&Item) -> bool) -> Vec<&'a Item> {
    items.iter().filter(|item| item.deletable() && (meetings || item.kind != Kind::Meetings) && !in_use(item)).collect()
}

/// A size as Settings says it.
pub fn human(bytes: u64) -> String {
    match bytes {
        b if b >= 1_000_000_000 => format!("{:.1} GB", b as f64 / 1e9),
        b if b >= 1_000_000 => format!("{:.0} MB", b as f64 / 1e6),
        b if b >= 1_000 => format!("{:.0} KB", b as f64 / 1e3),
        b => format!("{b} B"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn scratch(name: &str) -> Places {
        let root = std::env::temp_dir().join(format!("alcyo-storage-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let places = Places {
            data: root.join("data"),
            models: root.join("data/models"),
            voice: root.join("app/voice"),
            packs: None,
            search: root.join("app/search"),
            temp: root.join("tmp"),
            temp_is_own: true,
            metal: vec![],
        };
        for dir in [&places.models, &places.voice, &places.search, &places.temp] {
            std::fs::create_dir_all(dir).unwrap();
        }
        places
    }

    fn write(path: &Path, bytes: usize) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, vec![7u8; bytes]).unwrap();
    }

    /// Every model file found is a row with its own size, whatever model it is; what came with the app is built in;
    /// meetings, their sound and the inbox add up to one row.
    #[test]
    fn every_item_is_found_and_sized() {
        let p = scratch("survey");
        write(&p.models.join("Qwen3-4B-Instruct-2507-Q4_K_M.gguf"), 5000);
        write(&p.models.join("some-future-model.gguf"), 3000);
        write(&p.models.join("Qwen3.5-4B-mmproj-F16.gguf"), 2000);
        write(&p.models.join("Qwen3.5-4B-Q4_K_M.gguf.part"), 1000);
        // Qwen3.5-9B was dropped from the catalog; a copy downloaded before stays a row, to delete.
        write(&p.models.join("Qwen3.5-9B-Q4_K_M.gguf"), 6000);
        write(&p.models.join("Qwen3.5-9B-mmproj-F16.gguf"), 900);
        write(&p.models.join("selected.txt"), 10);
        write(&p.voice.join("sense-voice/model.onnx"), 4000);
        write(&p.search.join(alcyo_models::embedding().file), 700);
        write(&p.data.join("meetings/v2/a.json"), 100);
        write(&p.data.join("meetings/v2/a/sound-1.opus"), 900);
        write(&p.data.join("inbox/card.json"), 50);
        write(&p.data.join("search/a.index"), 60);
        write(&p.temp.join("export.pdf"), 80);
        write(&p.data.join("logs/app-20261126.log"), 70);
        write(&p.data.join("logs/events-20261126.jsonl"), 30);
        compile_cache::note_ready(&compile_cache::root(&p.models), Path::new("some-future-model.gguf"), "1.0").unwrap();
        let items = survey(&p);
        let find = |kind: &Kind| items.iter().find(|i| &i.kind == kind).unwrap_or_else(|| panic!("no {kind:?} in {items:?}"));
        assert_eq!(find(&Kind::BoardModel("some-future-model.gguf".into())).bytes, 3000, "a model the app does not know is listed too");
        assert_eq!(find(&Kind::BoardModel("Qwen3-4B-Instruct-2507-Q4_K_M.gguf".into())).bytes, 5000);
        assert_eq!(find(&Kind::ImageReader("Qwen3.5-4B-mmproj-F16.gguf".into())).bytes, 2000);
        assert_eq!(find(&Kind::Unfinished("Qwen3.5-4B-Q4_K_M.gguf".into())).bytes, 1000);
        assert!(find(&Kind::BoardModel("Qwen3.5-9B-Q4_K_M.gguf".into())).deletable(), "a dropped model is still listed to delete");
        assert!(find(&Kind::ImageReader("Qwen3.5-9B-mmproj-F16.gguf".into())).deletable());
        assert!(find(&Kind::Speech).built_in && find(&Kind::Speech).bytes == 4000);
        assert!(find(&Kind::SearchModel).built_in);
        assert_eq!(find(&Kind::Meetings).bytes, 1050, "meetings, their sound and the inbox");
        assert_eq!(find(&Kind::SearchIndex).bytes, 60);
        assert_eq!(find(&Kind::Temporary).bytes, 80);
        assert!(find(&Kind::Logs).bytes == 100 && find(&Kind::Logs).deletable(), "the logs are a row of their own, to delete");
        assert!(items.iter().any(|i| i.kind == Kind::CompileCache(compile_cache::Kind::Model("some-future-model.gguf".into()))));
        assert!(!items.iter().any(|i| i.paths.iter().any(|p| p.ends_with("selected.txt"))), "the choice of model is not an item");
        let _ = std::fs::remove_dir_all(p.data.parent().unwrap());
    }

    /// Delete all takes what can be deleted, keeps the meetings unless they are ticked, never what came with the app
    /// or what is in use; deleting says how much it gave back.
    #[test]
    fn delete_all_keeps_meetings_unless_asked_and_what_is_in_use() {
        let p = scratch("all");
        write(&p.models.join("a.gguf"), 5000);
        write(&p.models.join("b.gguf"), 3000);
        write(&p.voice.join("x.onnx"), 4000);
        write(&p.data.join("meetings/v2/m.json"), 100);
        let items = survey(&p);
        let in_use = |item: &Item| item.kind == Kind::BoardModel("a.gguf".into());
        let kinds = |chosen: Vec<&Item>| chosen.into_iter().map(|i| i.kind.clone()).collect::<Vec<_>>();
        assert_eq!(kinds(delete_all_selection(&items, false, &in_use)), vec![Kind::BoardModel("b.gguf".into())]);
        assert_eq!(kinds(delete_all_selection(&items, true, &in_use)), vec![Kind::BoardModel("b.gguf".into()), Kind::Meetings]);
        let freed: u64 = delete_all_selection(&items, true, &|_| false).into_iter().map(|i| delete(i).unwrap()).sum();
        assert_eq!(freed, 8100);
        assert!(!p.models.join("a.gguf").exists() && !p.data.join("meetings").exists());
        assert!(p.voice.join("x.onnx").exists(), "built in, kept");
        let _ = std::fs::remove_dir_all(p.data.parent().unwrap());
    }

    /// Each downloaded voice pack is a row to delete, Chinese with its live captions; the voice detector that came
    /// with the app stays built in.
    #[test]
    fn voice_packs_are_rows_of_their_own() {
        let mut p = scratch("packs");
        let packs = p.models.join("voice");
        p.packs = Some(alcyo_models::voice::Places { packs: packs.clone(), bundled: p.voice.clone() });
        write(&p.voice.join("silero_vad.onnx"), 300);
        write(&packs.join("sense-voice/model.int8.onnx"), 1000);
        write(&packs.join("draft-zh/encoder.int8.onnx.part"), 500);
        write(&packs.join("speaker/campplus_zh_en.onnx"), 200);
        let items = survey(&p);
        let find = |id: &str| items.iter().find(|i| i.kind == Kind::VoicePack(id.into()));
        let zh = find("zh").expect("Chinese");
        assert!(zh.bytes == 1500 && zh.deletable(), "with its live captions, unfinished as they are: {zh:?}");
        assert!(find("live").is_none(), "in Chinese's row");
        assert_eq!(find("spk").map(|i| i.bytes), Some(200));
        assert!(find("en").is_none(), "nothing of English is here");
        assert!(items.iter().any(|i| i.kind == Kind::Speech && i.built_in && i.bytes == 300));
        let _ = std::fs::remove_dir_all(p.data.parent().unwrap());
    }

    #[test]
    fn sizes_read_as_people_say_them() {
        assert_eq!(human(2_500_000_000), "2.5 GB");
        assert_eq!(human(330_000_000), "330 MB");
        assert_eq!(human(1_200), "1 KB");
        assert_eq!(human(12), "12 B");
    }
}
