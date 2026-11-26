//! What the graphics compiled for each local model, kept so a model's next start is not its first.
//!
//! Each model has a folder of its own, named by its file, under `<data>/shader-cache/models/`: the graphics driver's
//! cache for that model's server where the driver keeps one in a folder it is told (NVIDIA and Mesa, on Linux and
//! Windows), and a note, `ready`, of the app versions a start of it got all the way through with. Settings list what
//! is there, whatever models those are, so a model added later shows up by itself.
//!
//! Anything else under `shader-cache/` was left by versions that kept one cache for every model, and is listed as
//! that. On the iPad the model's graphics code is built from source as it starts (llama.cpp's Metal library is
//! embedded as source, and no binary archive is kept), and the system caches what it compiled in the app's own
//! `Library/Caches`, in folders named `com.apple.metal` (directly there, or under a folder of the app's bundle id).
//! That cache serves every model at once, so it is one shared row with its real size; a model's own row there is
//! only the note that it was built, and says no size.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// The note of a start that got all the way through, in a model's folder.
pub const READY: &str = "ready";
/// Where the models' folders are, under the cache's root.
const MODELS: &str = "models";

/// The cache's root, beside the models' folder.
pub fn root(models: &Path) -> PathBuf {
    models.parent().unwrap_or(models).join("shader-cache")
}

/// The folder for the model in `model` (its path or its file's name).
pub fn model_dir(root: &Path, model: &Path) -> PathBuf {
    root.join(MODELS).join(model.file_name().unwrap_or(model.as_os_str()))
}

/// Whether a start of `model` got all the way through before: with this app `version` when one is given (the iPad's
/// graphics code comes with each version of the app), or with any version when `None` (a desktop driver keeps what it
/// compiled across versions).
pub fn ready(root: &Path, model: &Path, version: Option<&str>) -> bool {
    std::fs::read_to_string(model_dir(root, model).join(READY)).is_ok_and(|noted| match version {
        Some(version) => noted.lines().any(|line| line.trim() == version),
        None => true,
    })
}

/// Note that a start of `model` got all the way through with this app `version`. The folder is made here too: on the
/// iPad no server program makes it first.
pub fn note_ready(root: &Path, model: &Path, version: &str) -> std::io::Result<()> {
    let dir = model_dir(root, model);
    let path = dir.join(READY);
    let noted = std::fs::read_to_string(&path).unwrap_or_default();
    if noted.lines().any(|line| line.trim() == version) {
        return Ok(());
    }
    std::fs::create_dir_all(&dir)?;
    std::fs::write(path, format!("{noted}{version}\n"))
}

/// What an entry of the cache is for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    /// One model's, by its file's name.
    Model(String),
    /// Kept for every model at once, by versions before the cache was split by model.
    Older,
    /// Metal's own, which the iPad's system keeps for every model at once.
    System,
}

/// One entry of the cache, as Settings lists it.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub kind: Kind,
    /// What is on disk for it: a folder, or for `Older` the cache's root (its model folders aside).
    pub path: PathBuf,
    /// Further folders that make up the same entry: Metal's cache where the system keeps it in more than one.
    pub more: Vec<PathBuf>,
    pub bytes: u64,
    /// When a start of it last got all the way through, or else when it last changed.
    pub built: Option<SystemTime>,
    /// The app versions it was finished with, newest last.
    pub versions: Vec<String>,
}

/// Everything in the cache under `root`, model by model as found on disk, then what is shared. `system` is Metal's
/// own cache folders where there are some (the iPad).
pub fn entries(root: &Path, system: &[PathBuf]) -> Vec<Entry> {
    let mut out = vec![];
    if let Ok(dirs) = std::fs::read_dir(root.join(MODELS)) {
        let mut models: Vec<_> = dirs.flatten().filter(|d| d.file_type().is_ok_and(|t| t.is_dir())).collect();
        models.sort_by_key(|d| d.file_name());
        for dir in models {
            let path = dir.path();
            let note = path.join(READY);
            let versions = std::fs::read_to_string(&note).map(|s| s.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect()).unwrap_or_default();
            let built = std::fs::metadata(&note).or_else(|_| std::fs::metadata(&path)).and_then(|m| m.modified()).ok();
            out.push(Entry { kind: Kind::Model(dir.file_name().to_string_lossy().into_owned()), bytes: size(&path), path, more: vec![], built, versions });
        }
    }
    // What older versions kept at the root, for every model.
    let older: Vec<PathBuf> = std::fs::read_dir(root).map(|e| e.flatten().filter(|e| e.file_name() != MODELS).map(|e| e.path()).collect()).unwrap_or_default();
    let bytes: u64 = older.iter().map(|p| size(p)).sum();
    if !older.is_empty() {
        let built = older.iter().filter_map(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok()).max();
        out.push(Entry { kind: Kind::Older, path: root.to_path_buf(), more: vec![], bytes, built, versions: vec![] });
    }
    let found: Vec<&PathBuf> = system.iter().filter(|s| s.is_dir()).collect();
    if let Some((first, rest)) = found.split_first() {
        let built = found.iter().filter_map(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok()).max();
        let bytes = found.iter().map(|p| size(p)).sum();
        out.push(Entry { kind: Kind::System, path: first.to_path_buf(), more: rest.iter().map(|p| p.to_path_buf()).collect(), bytes, built, versions: vec![] });
    }
    out
}

/// Remove an entry from disk. For `Older` that is everything at the root but the models' folders.
pub fn clear(entry: &Entry) -> std::io::Result<()> {
    match entry.kind {
        Kind::Older => {
            for item in std::fs::read_dir(&entry.path)?.flatten().filter(|e| e.file_name() != MODELS) {
                remove(&item.path())?;
            }
            Ok(())
        }
        // Metal's folders are the system's: what is in them goes, the folders stay.
        Kind::System => {
            for dir in std::iter::once(&entry.path).chain(&entry.more) {
                for item in std::fs::read_dir(dir)?.flatten() {
                    remove(&item.path())?;
                }
            }
            Ok(())
        }
        Kind::Model(_) => remove(&entry.path),
    }
}

/// Whether clearing `entry` would pull a cache from under a model that is running: its own folder while it runs,
/// and what every model shares while any runs. `running` is the running model's path, `busy` that one is starting
/// or stopping and which cannot be told.
pub fn in_use(entry: &Entry, running: Option<&Path>, busy: bool) -> bool {
    match &entry.kind {
        Kind::Model(file) => busy || running.and_then(Path::file_name).is_some_and(|name| name.to_string_lossy() == *file),
        Kind::Older | Kind::System => busy || running.is_some(),
    }
}

/// Metal's cache folders on the iPad: every folder named `com.apple.metal` in the app's `Library/Caches`, there or a
/// level or two down (the system has kept it under a folder of the bundle id too). `data` is the app's data folder
/// in `Library/Application Support`. None elsewhere.
pub fn system_dirs(data: &Path) -> Vec<PathBuf> {
    if !cfg!(target_os = "ios") {
        return vec![];
    }
    let Some(library) = data.ancestors().find(|p| p.file_name().is_some_and(|n| n == "Library")) else { return vec![] };
    metal_dirs(&library.join("Caches"), 3)
}

/// Folders named `com.apple.metal` at most `depth` levels under `dir`.
fn metal_dirs(dir: &Path, depth: usize) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else { return vec![] };
    let mut out = vec![];
    for entry in entries.flatten().filter(|e| e.file_type().is_ok_and(|t| t.is_dir())) {
        if entry.file_name() == "com.apple.metal" {
            out.push(entry.path());
        } else if depth > 1 {
            out.extend(metal_dirs(&entry.path(), depth - 1));
        }
    }
    out.sort();
    out
}

fn remove(path: &Path) -> std::io::Result<()> {
    if path.is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    }
}

/// How much is on disk under `path`, a file or a folder.
pub fn size(path: &Path) -> u64 {
    let Ok(meta) = std::fs::symlink_metadata(path) else { return 0 };
    if !meta.is_dir() {
        return meta.len();
    }
    std::fs::read_dir(path).map(|entries| entries.flatten().map(|e| size(&e.path())).sum()).unwrap_or(0)
}

/// Versions before this one kept a single note at the root, a line `<version> <model file>` for each start the iPad
/// finished (the desktop's noted no model). Each line moves to its model's folder, and the old note goes.
pub fn adopt_older_note(root: &Path) {
    let old = root.join(READY);
    let Ok(noted) = std::fs::read_to_string(&old) else { return };
    for line in noted.lines() {
        if let Some((version, model)) = line.trim().split_once(' ').filter(|(_, m)| !m.trim().is_empty()) {
            if let Err(e) = note_ready(root, Path::new(model.trim()), version) {
                eprintln!("could not move the note for {model}: {e}");
                return;
            }
        }
    }
    let _ = std::fs::remove_file(old);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("alcyo-compile-cache-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Any model found on disk is listed, not only the ones the app knows by name, each with its own size and the
    /// versions it was finished with.
    #[test]
    fn every_model_on_disk_is_listed_by_itself() {
        let root = scratch("list");
        note_ready(&root, Path::new("/m/Qwen3-4B-Q4_K_M.gguf"), "0.34.1").unwrap();
        note_ready(&root, Path::new("/m/Qwen3-4B-Q4_K_M.gguf"), "0.35.0").unwrap();
        note_ready(&root, Path::new("some-future-model.gguf"), "0.35.0").unwrap();
        std::fs::write(model_dir(&root, Path::new("some-future-model.gguf")).join("driver.bin"), vec![0u8; 4000]).unwrap();
        let found = entries(&root, &[]);
        assert_eq!(found.iter().map(|e| e.kind.clone()).collect::<Vec<_>>(), vec![Kind::Model("Qwen3-4B-Q4_K_M.gguf".into()), Kind::Model("some-future-model.gguf".into())]);
        assert_eq!(found[0].versions, vec!["0.34.1", "0.35.0"]);
        assert!(found[1].bytes >= 4000);
        assert!(found.iter().all(|e| e.built.is_some()));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_start_counts_for_its_model_and_version() {
        let root = scratch("ready");
        let model = Path::new("a.gguf");
        assert!(!ready(&root, model, None));
        note_ready(&root, model, "1.0").unwrap();
        assert!(ready(&root, model, Some("1.0")));
        assert!(!ready(&root, model, Some("1.1")), "the iPad compiles again with a new version");
        assert!(ready(&root, model, None), "a desktop driver keeps its cache across versions");
        assert!(!ready(&root, Path::new("b.gguf"), None), "another model starts for the first time");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Clearing one model leaves the others, and what older versions kept is cleared without the models' folders.
    #[test]
    fn clearing_takes_only_what_was_asked() {
        let root = scratch("clear");
        note_ready(&root, Path::new("a.gguf"), "1.0").unwrap();
        note_ready(&root, Path::new("b.gguf"), "1.0").unwrap();
        std::fs::create_dir_all(root.join("mesa_shader_cache")).unwrap();
        std::fs::write(root.join("mesa_shader_cache/index"), b"x").unwrap();
        let found = entries(&root, &[]);
        assert_eq!(found.last().unwrap().kind, Kind::Older);
        clear(&found[0]).unwrap();
        clear(found.last().unwrap()).unwrap();
        let left = entries(&root, &[]);
        assert_eq!(left.iter().map(|e| e.kind.clone()).collect::<Vec<_>>(), vec![Kind::Model("b.gguf".into())]);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_running_model_keeps_its_cache() {
        let entry = |kind| Entry { kind, path: PathBuf::new(), more: vec![], bytes: 0, built: None, versions: vec![] };
        let running = Some(Path::new("/models/a.gguf"));
        assert!(in_use(&entry(Kind::Model("a.gguf".into())), running, false));
        assert!(!in_use(&entry(Kind::Model("b.gguf".into())), running, false));
        assert!(in_use(&entry(Kind::Older), running, false), "shared by the model that runs");
        assert!(!in_use(&entry(Kind::System), None, false));
        assert!(in_use(&entry(Kind::Model("b.gguf".into())), None, true), "starting or stopping: wait");
    }

    /// Metal's cache is found wherever the system put it under Caches, and its folders are counted and cleared as one.
    #[test]
    fn metal_caches_are_found_and_shared() {
        let caches = scratch("metal");
        for dir in ["com.apple.metal", "app.alcyo.board/com.apple.metal"] {
            std::fs::create_dir_all(caches.join(dir)).unwrap();
            std::fs::write(caches.join(dir).join("functions.data"), vec![0u8; 3000]).unwrap();
        }
        std::fs::create_dir_all(caches.join("other")).unwrap();
        let found = metal_dirs(&caches, 3);
        assert_eq!(found.len(), 2);
        let root = caches.join("shader-cache");
        let listed = entries(&root, &found);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].kind, Kind::System);
        assert!(listed[0].bytes >= 6000, "both folders counted");
        clear(&listed[0]).unwrap();
        assert!(found.iter().all(|d| d.is_dir() && std::fs::read_dir(d).unwrap().next().is_none()), "emptied, kept");
        let _ = std::fs::remove_dir_all(&caches);
    }

    #[test]
    fn the_older_single_note_moves_to_each_model() {
        let root = scratch("adopt");
        std::fs::write(root.join(READY), "0.34.0 Qwen3-4B.gguf\n0.34.1 Qwen3.5-4B.gguf\n").unwrap();
        adopt_older_note(&root);
        assert!(ready(&root, Path::new("Qwen3-4B.gguf"), Some("0.34.0")));
        assert!(ready(&root, Path::new("Qwen3.5-4B.gguf"), Some("0.34.1")));
        assert!(!root.join(READY).exists());
        // The desktop's note named no model: it simply goes.
        std::fs::write(root.join(READY), "").unwrap();
        adopt_older_note(&root);
        assert!(!root.join(READY).exists());
        let _ = std::fs::remove_dir_all(&root);
    }
}
