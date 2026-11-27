//! The speech models, about 1.2 GB, and the voice packs they are downloaded in.
//!
//! Every file is listed here: SenseVoice, the Silero VAD, the CAM++ speaker model, Nemotron-3-Diarization for
//! telling speakers apart as they talk, a streaming Chinese zipformer for drafts while someone speaks, Parakeet for
//! English and ReazonSpeech for Japanese. The speaker model is not on ModelScope,
//! so it comes from Hugging Face, pinned to the revision the app was tested with; the diarization model, exported to
//! ONNX by our sherpa-onnx fork's CI, only from that fork's release. Every file is also on Alcyo's mirror (see
//! [`crate::MIRROR`]), tried first and raced against ModelScope where it has the file. The list is in
//! `catalogue.rs`, which CI caches the files by.
//!
//! Only the voice detector comes with the app. The rest is grouped into voice packs (Chinese, its live captions,
//! English, Japanese, telling speakers apart), which the person picks on first run or in Settings › Speech and the
//! app downloads into its data folder with the same downloader as the board models: resumable, every file checked
//! by size and SHA-256 before it takes its name. A checkout or CI keeps every file in `resources/voice`, which still
//! counts: a pack whose files are all there comes with the app.

mod catalogue;
pub use catalogue::{files, VoiceFile};

use crate::{Model, Outcome, Progress, Source};
use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Mutex;

/// What a voice pack download leaves free on the disk: less than a board model's, as its files are smaller.
pub const SPACE_MARGIN: u64 = 100_000_000;

/// A voice pack: speech models downloaded together, on their own.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Pack {
    /// SenseVoice, which also hears English, Japanese, Korean and Cantonese.
    Zh,
    /// The streaming Chinese model that drafts while someone speaks; it comes with Chinese.
    Live,
    /// Parakeet, for English.
    En,
    /// ReazonSpeech, for Japanese.
    Ja,
    /// CAM++ and Nemotron-3-Diarization, for telling speakers apart.
    Spk,
}

impl Pack {
    /// Every pack, in the order they are shown and downloaded.
    pub const ALL: [Pack; 5] = [Pack::Zh, Pack::Live, Pack::En, Pack::Ja, Pack::Spk];

    pub fn id(self) -> &'static str {
        match self {
            Pack::Zh => "zh",
            Pack::Live => "live",
            Pack::En => "en",
            Pack::Ja => "ja",
            Pack::Spk => "spk",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.id() == id)
    }

    /// The pack it only makes sense with: live captions draft Chinese, so they come with it and go with it.
    pub fn needs(self) -> Option<Pack> {
        (self == Pack::Live).then_some(Pack::Zh)
    }

    /// Its folder in a folder of speech models.
    pub fn folder(self) -> &'static str {
        match self {
            Pack::Zh => "sense-voice",
            Pack::Live => "draft-zh",
            Pack::En => "parakeet-en",
            Pack::Ja => "reazon-ja",
            Pack::Spk => "speaker",
        }
    }

    /// The models in it, by name.
    pub fn models(self) -> &'static str {
        match self {
            Pack::Zh => "SenseVoice",
            Pack::Live => "Zipformer",
            Pack::En => "Parakeet",
            Pack::Ja => "ReazonSpeech",
            Pack::Spk => "CAM++ · Nemotron",
        }
    }

    pub fn files(self) -> Vec<VoiceFile> {
        let prefix = format!("{}/", self.folder());
        files().into_iter().filter(|f| f.path().starts_with(&prefix)).collect()
    }

    /// Its size, downloaded.
    pub fn size(self) -> u64 {
        self.files().iter().map(|f| f.size).sum()
    }
}

/// The files that come with every package, in no pack: the voice detector.
pub fn bundled() -> Vec<VoiceFile> {
    files().into_iter().filter(|f| !Pack::ALL.iter().any(|p| f.path().starts_with(&format!("{}/", p.folder())))).collect()
}

/// What a choice of packs downloads: live captions only with Chinese, each pack once, in [`Pack::ALL`]'s order.
pub fn closed(chosen: &[Pack]) -> Vec<Pack> {
    Pack::ALL.into_iter().filter(|p| chosen.contains(p) && p.needs().is_none_or(|n| chosen.contains(&n))).collect()
}

/// The packs ticked at first, by the system's language: its own language (Chinese with its live captions), and
/// telling speakers apart always. No other language is ticked for the person.
pub fn preset(locale: &str) -> Vec<Pack> {
    let locale = locale.to_ascii_lowercase();
    match locale.split(['-', '_', '.']).next().unwrap_or_default() {
        "zh" => vec![Pack::Zh, Pack::Live, Pack::Spk],
        "en" => vec![Pack::En, Pack::Spk],
        "ja" => vec![Pack::Ja, Pack::Spk],
        _ => vec![Pack::Spk],
    }
}

/// The packs a package of 0.34 or earlier came with, by its variant (`packaging/models.sh` then): every one for
/// the full package (an empty variant, "all" or "full"), the one language and the speaker models for the others,
/// none for the Mac's package without models.
pub fn of_variant(variant: &str) -> Vec<Pack> {
    match variant.trim() {
        "en" => vec![Pack::En, Pack::Spk],
        "ja" => vec![Pack::Ja, Pack::Spk],
        "sensevoice" => vec![Pack::Zh, Pack::Spk],
        "none" | "apple" => vec![],
        _ => Pack::ALL.to_vec(),
    }
}

/// The packs to get back on the first start after updating from a version whose package came with its speech
/// models, or None for a first install, which asks. Someone who listened with Apple's recognizer gets nothing. The
/// variant is known when the old package's note of it is still there (an installer that wrote over the old files);
/// otherwise the language chosen for listening tells English or Japanese alone (what the "-en" and "-ja" packages
/// were for), and anything else is taken for the full package, which nearly everyone had.
pub fn after_update(used_before: bool, apple: bool, variant: Option<&str>, language: &str) -> Option<Vec<Pack>> {
    if !used_before {
        return None;
    }
    if apple {
        return Some(vec![]);
    }
    Some(match (variant, language) {
        (Some(variant), _) => of_variant(variant),
        (None, "en-US") => of_variant("en"),
        (None, "ja-JP") => of_variant("ja"),
        (None, _) => of_variant(""),
    })
}

/// Where a pack stands on this device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// Every file downloaded, as the catalogue has it.
    Downloaded,
    /// Came with the app (a checkout's `resources/voice`), so it is not deleted.
    BuiltIn,
    /// Downloaded, but the catalogue has newer files: still used until they arrive.
    Outdated,
    /// Not here, or only some of it.
    Missing,
}

impl Status {
    /// Whether it can hear now.
    pub fn usable(self) -> bool {
        self != Status::Missing
    }
}

/// The record of each downloaded file's hash, beside the packs.
const MANIFEST: &str = "installed.json";
/// What the person chose, beside the packs.
const CHOICE: &str = "packs.json";
/// The manifest is read and written from the download's thread and the window's.
static MANIFEST_LOCK: Mutex<()> = Mutex::new(());

/// Where voice packs are kept, and where the app's own speech models are.
#[derive(Clone, Debug)]
pub struct Places {
    /// Downloaded packs, in the app's data folder, laid out like `resources/voice`.
    pub packs: PathBuf,
    /// The speech models that came with the app: the voice detector in a package, every file in a checkout.
    pub bundled: PathBuf,
}

impl Places {
    /// The folders to find speech models in, downloaded ones first.
    pub fn dirs(&self) -> Vec<PathBuf> {
        if self.packs == self.bundled {
            vec![self.packs.clone()]
        } else {
            vec![self.packs.clone(), self.bundled.clone()]
        }
    }

    fn manifest(&self) -> BTreeMap<String, String> {
        let text = fs::read_to_string(self.packs.join(MANIFEST)).unwrap_or_default();
        serde_json::from_str::<BTreeMap<String, String>>(&text).unwrap_or_default()
    }

    /// Note a file as downloaded and checked.
    fn note(&self, file: &VoiceFile) -> Result<()> {
        let _held = MANIFEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut all = self.manifest();
        all.insert(file.path().to_string(), file.sha256.to_string());
        write_atomically(&self.packs.join(MANIFEST), &serde_json::to_string_pretty(&all)?)
    }

    fn forget(&self, paths: &[&str]) -> Result<()> {
        let _held = MANIFEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut all = self.manifest();
        all.retain(|k, _| !paths.contains(&k.as_str()));
        write_atomically(&self.packs.join(MANIFEST), &serde_json::to_string_pretty(&all)?)
    }

    /// Whether a downloaded file is the catalogue's: there, its size, and checked as its hash when it arrived. A
    /// file put there by hand, never noted, is taken by its size.
    fn current(&self, file: &VoiceFile, manifest: &BTreeMap<String, String>) -> bool {
        let path = self.packs.join(file.path());
        fs::metadata(&path).is_ok_and(|m| m.is_file() && m.len() == file.size) && manifest.get(file.path()).is_none_or(|sha| sha == file.sha256)
    }

    pub fn status(&self, pack: Pack) -> Status {
        let files = pack.files();
        if self.packs != self.bundled && files.iter().all(|f| self.bundled.join(f.path()).is_file()) {
            return Status::BuiltIn;
        }
        let manifest = self.manifest();
        if files.iter().all(|f| self.current(f, &manifest)) {
            return Status::Downloaded;
        }
        // Every file there, some older than the catalogue's: it still hears while the new ones come.
        if files.iter().all(|f| self.packs.join(f.path()).is_file()) {
            return Status::Outdated;
        }
        Status::Missing
    }

    /// The files of a pack still to download, with how much of each is already here from an earlier attempt.
    fn to_fetch(&self, pack: Pack) -> Vec<(VoiceFile, u64)> {
        if self.status(pack) == Status::BuiltIn {
            return vec![];
        }
        let manifest = self.manifest();
        pack.files()
            .into_iter()
            .filter(|f| !self.current(f, &manifest))
            .map(|f| {
                let part = fs::metadata(part_of(&self.packs.join(f.path()))).map_or(0, |m| m.len().min(f.size));
                (f, part)
            })
            .collect()
    }

    /// How much is left to download for `packs`, counting what earlier attempts left.
    pub fn missing_bytes(&self, packs: &[Pack]) -> u64 {
        closed(packs).into_iter().flat_map(|p| self.to_fetch(p)).map(|(f, part)| f.size - part).sum()
    }

    /// The bytes a downloaded pack takes, finished or not.
    pub fn on_disk(&self, pack: Pack) -> u64 {
        if self.status(pack) == Status::BuiltIn {
            return 0;
        }
        pack.files().iter().map(|f| self.packs.join(f.path())).flat_map(|p| [part_of(&p), p]).filter_map(|p| fs::metadata(p).ok()).map(|m| m.len()).sum()
    }

    /// Delete a downloaded pack, and what only makes sense with it (live captions go with Chinese), unfinished
    /// files too. Packs that came with the app are left. Gives back how many bytes went.
    pub fn delete(&self, pack: Pack) -> Result<u64> {
        let mut freed = 0;
        for p in Pack::ALL.into_iter().filter(|p| *p == pack || p.needs() == Some(pack)) {
            if self.status(p) == Status::BuiltIn {
                continue;
            }
            freed += self.on_disk(p);
            let files = p.files();
            for path in files.iter().map(|f| self.packs.join(f.path())) {
                for file in [part_of(&path), path] {
                    if file.exists() {
                        fs::remove_file(&file).with_context(|| format!("deleting {}", file.display()))?;
                    }
                }
            }
            let _ = fs::remove_dir(self.packs.join(p.folder()));
            self.forget(&files.iter().map(|f| f.path()).collect::<Vec<_>>())?;
        }
        Ok(freed)
    }
}

fn part_of(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".part");
    PathBuf::from(name)
}

fn write_atomically(path: &Path, text: &str) -> Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let temp = path.with_extension("json.tmp");
    fs::write(&temp, text).with_context(|| format!("writing {}", temp.display()))?;
    fs::rename(&temp, path).with_context(|| format!("writing {}", path.display()))
}

/// What the person chose about voice packs, kept beside them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Choice {
    /// The packs asked for: kept downloaded, and up to date, until deleted.
    pub wanted: Vec<Pack>,
    /// The first-run question was answered, by downloading or skipping: it is not asked again.
    pub decided: bool,
    /// Packs being brought back after an update from a version that came with them, until they are.
    pub restoring: Vec<Pack>,
    /// The one-time word that they are being brought back was given.
    pub told: bool,
}

impl Choice {
    /// What was chosen, or None before anything was.
    pub fn load(packs: &Path) -> Option<Self> {
        let text = fs::read_to_string(packs.join(CHOICE)).ok()?;
        let v: Value = serde_json::from_str(&text).ok()?;
        let list = |key: &str| -> Vec<Pack> { v[key].as_array().map(|a| a.iter().filter_map(|p| p.as_str().and_then(Pack::from_id)).collect()).unwrap_or_default() };
        Some(Self { wanted: closed(&list("wanted")), decided: v["decided"].as_bool().unwrap_or(true), restoring: list("restoring"), told: v["told"].as_bool().unwrap_or(false) })
    }

    pub fn save(&self, packs: &Path) -> Result<()> {
        let ids = |list: &[Pack]| list.iter().map(|p| p.id()).collect::<Vec<_>>();
        let text = serde_json::to_string_pretty(&json!({ "wanted": ids(&self.wanted), "decided": self.decided, "restoring": ids(&self.restoring), "told": self.told }))?;
        write_atomically(&packs.join(CHOICE), &text)
    }

    /// Ask for more packs, with what they come with.
    pub fn want(&mut self, packs: &[Pack]) {
        let mut all = self.wanted.clone();
        all.extend_from_slice(packs);
        self.wanted = closed(&all);
    }

    /// No longer keep a pack, nor what only makes sense with it.
    pub fn unwant(&mut self, pack: Pack) {
        self.wanted.retain(|p| *p != pack && p.needs() != Some(pack));
        self.restoring.retain(|p| *p != pack && p.needs() != Some(pack));
    }
}

/// How far a download of packs is.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Got {
    /// Each pack being fetched: bytes here and bytes in all.
    pub packs: Vec<(Pack, u64, u64)>,
    /// Bytes a second, lately; 0 until known.
    pub speed: u64,
    /// Where it comes from now.
    pub source: Option<String>,
    /// Checking what an earlier attempt left, or trying the sources for the fastest.
    pub checking: bool,
}

impl Got {
    pub fn done(&self) -> u64 {
        self.packs.iter().map(|p| p.1).sum()
    }

    pub fn total(&self) -> u64 {
        self.packs.iter().map(|p| p.2).sum()
    }

    /// The share done, from 0 to 1.
    pub fn share(&self) -> f32 {
        let total = self.total();
        if total == 0 { 1.0 } else { (self.done() as f64 / total as f64) as f32 }
    }

    /// Whether every file of a pack is here.
    pub fn ready(&self, pack: Pack) -> bool {
        self.packs.iter().any(|(p, done, total)| *p == pack && done >= total)
    }
}

#[derive(Clone, Debug)]
pub enum Event {
    Progress(Got),
    /// A pack has every file, checked: it can hear from the next time listening starts.
    Ready(Pack),
    /// It stopped. What arrived is kept, and the next attempt carries on from there.
    Failed(String),
    Cancelled,
    /// Every pack asked for is here.
    Done,
}

/// The sources of a file with `prefer` alone raced and first, when it has that source: "ModelScope" when the
/// person asks for it after the usual ones failed.
fn preferring(mut sources: Vec<Source>, prefer: Option<&str>) -> Vec<Source> {
    let Some(name) = prefer.filter(|n| sources.iter().any(|s| s.name == *n)) else { return sources };
    sources.sort_by_key(|s| s.name != name);
    for s in &mut sources {
        s.preferred = s.name == name;
    }
    sources
}

/// The download of a voice file, as the board models' downloader takes it.
fn as_model(file: &VoiceFile, prefer: Option<&str>) -> Model {
    Model { id: file.name, label: file.name, file: file.path().to_string(), size: file.size, sha256: file.sha256.to_string(), note: None, sources: preferring(file.sources(), prefer) }
}

/// Download `packs` (with what they come with) one after another, so the first is usable soon, into
/// `places.packs`. Files already here are skipped, an older copy is replaced, and an unfinished one carries on.
/// `prefer` names the one source to use first, as "ModelScope". `report` hears how far it is a few times a
/// second, each pack as it is ready, and how it ended.
pub fn fetch(packs: &[Pack], places: &Places, prefer: Option<&str>, cancel: &AtomicBool, report: &mut dyn FnMut(Event)) {
    let jobs: Vec<(Pack, Vec<(Model, VoiceFile)>)> = closed(packs)
        .into_iter()
        .map(|p| (p, places.to_fetch(p).into_iter().map(|(f, _)| (as_model(&f, prefer), f)).collect()))
        .collect();
    fetch_jobs(jobs, places, cancel, report);
}

fn fetch_jobs(jobs: Vec<(Pack, Vec<(Model, VoiceFile)>)>, places: &Places, cancel: &AtomicBool, report: &mut dyn FnMut(Event)) {
    let mut got = Got { packs: jobs.iter().map(|(p, files)| (*p, 0, files.iter().map(|(m, _)| m.size).sum())).collect(), ..Default::default() };
    report(Event::Progress(got.clone()));
    for (i, (pack, files)) in jobs.iter().enumerate() {
        let mut finished = 0_u64;
        for (model, file) in files {
            let path = places.packs.join(&model.file);
            // An older copy, or one that does not match: the new one takes its place.
            if path.is_file() {
                if let Err(e) = fs::remove_file(&path) {
                    report(Event::Failed(format!("{e:#}")));
                    return;
                }
            }
            let outcome = crate::download_keeping(model, &places.packs, SPACE_MARGIN, cancel, &mut |p: Progress| {
                got.packs[i].1 = finished + p.done;
                // Each file starts without a speed; the last one known stands until there is one.
                if p.speed > 0 {
                    got.speed = p.speed;
                }
                got.source = p.source.clone();
                got.checking = p.verifying || p.testing;
                report(Event::Progress(got.clone()));
            });
            match outcome {
                Ok(Outcome::Finished(_)) => {
                    if let Err(e) = places.note(file) {
                        report(Event::Failed(format!("{e:#}")));
                        return;
                    }
                    finished += model.size;
                    got.packs[i].1 = finished;
                    report(Event::Progress(got.clone()));
                }
                Ok(Outcome::Cancelled) => {
                    report(Event::Cancelled);
                    return;
                }
                Err(e) => {
                    report(Event::Failed(format!("{e:#}")));
                    return;
                }
            }
        }
        report(Event::Ready(*pack));
    }
    report(Event::Done);
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicU32, Ordering};

    static NEXT: AtomicU32 = AtomicU32::new(0);

    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> (Self, Places) {
            let root = std::env::temp_dir().join(format!("alcyo-packs-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
            let places = Places { packs: root.join("data/models/voice"), bundled: root.join("app/voice") };
            fs::create_dir_all(&places.packs).unwrap();
            fs::create_dir_all(&places.bundled).unwrap();
            (Self(root), places)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// A pack's files written as if downloaded: sparse, the right size, noted with the catalogue's hash.
    fn put(places: &Places, pack: Pack) {
        for f in pack.files() {
            let path = places.packs.join(f.path());
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::File::create(&path).unwrap().set_len(f.size).unwrap();
            places.note(&f).unwrap();
        }
    }

    #[test]
    fn every_file_is_in_one_pack_or_comes_with_the_app() {
        let all = files();
        let in_packs: usize = Pack::ALL.iter().map(|p| p.files().len()).sum();
        assert_eq!(in_packs + bundled().len(), all.len());
        assert_eq!(bundled().iter().map(|f| f.path()).collect::<Vec<_>>(), ["silero_vad.onnx"], "only the voice detector comes with the app");
        for p in Pack::ALL {
            assert!(!p.files().is_empty(), "{p:?}");
            assert_eq!(Pack::from_id(p.id()), Some(p));
        }
        // The sizes the first run shows, in MB.
        let mb = |p: Pack| p.size() / 1_000_000;
        assert_eq!([mb(Pack::Zh), mb(Pack::Live), mb(Pack::En), mb(Pack::Ja), mb(Pack::Spk)], [239, 167, 663, 160, 132]);
    }

    #[test]
    fn live_captions_come_and_go_with_chinese() {
        assert_eq!(closed(&[Pack::Live, Pack::Spk]), [Pack::Spk]);
        assert_eq!(closed(&[Pack::Spk, Pack::Live, Pack::Zh, Pack::Zh]), [Pack::Zh, Pack::Live, Pack::Spk]);
        let mut choice = Choice { wanted: vec![Pack::Zh, Pack::Live, Pack::Spk], ..Default::default() };
        choice.unwant(Pack::Zh);
        assert_eq!(choice.wanted, [Pack::Spk]);
    }

    #[test]
    fn the_system_language_ticks_its_own_pack_and_telling_speakers_apart() {
        assert_eq!(preset("zh-CN"), [Pack::Zh, Pack::Live, Pack::Spk]);
        assert_eq!(preset("zh_TW.UTF-8"), [Pack::Zh, Pack::Live, Pack::Spk]);
        assert_eq!(preset("en-US"), [Pack::En, Pack::Spk]);
        assert_eq!(preset("ja-JP"), [Pack::Ja, Pack::Spk]);
        assert_eq!(preset("fr-FR"), [Pack::Spk], "no other language is ticked");
    }

    #[test]
    fn an_update_brings_back_what_the_old_package_came_with() {
        assert_eq!(after_update(false, false, None, "auto"), None, "a first install asks");
        assert_eq!(after_update(true, true, Some(""), "auto"), Some(vec![]), "Apple's recognizer needs nothing");
        assert_eq!(after_update(true, false, None, "auto"), Some(Pack::ALL.to_vec()), "the full package, as nearly everyone had");
        assert_eq!(after_update(true, false, None, "zh-CN"), Some(Pack::ALL.to_vec()));
        // Each old package, by its note.
        assert_eq!(after_update(true, false, Some("full"), "auto"), Some(Pack::ALL.to_vec()));
        assert_eq!(after_update(true, false, Some("all"), "en-US"), Some(Pack::ALL.to_vec()), "the note outranks the language");
        assert_eq!(after_update(true, false, Some("en"), "auto"), Some(vec![Pack::En, Pack::Spk]));
        assert_eq!(after_update(true, false, Some("ja"), "auto"), Some(vec![Pack::Ja, Pack::Spk]));
        assert_eq!(after_update(true, false, Some("sensevoice"), "auto"), Some(vec![Pack::Zh, Pack::Spk]));
        assert_eq!(after_update(true, false, Some("none"), "auto"), Some(vec![]));
        // Without the note, English or Japanese alone is what the language says.
        assert_eq!(after_update(true, false, None, "en-US"), Some(vec![Pack::En, Pack::Spk]));
        assert_eq!(after_update(true, false, None, "ja-JP"), Some(vec![Pack::Ja, Pack::Spk]));
    }

    #[test]
    fn a_pack_is_downloaded_built_in_outdated_or_missing() {
        let (_s, places) = Scratch::new();
        assert_eq!(places.status(Pack::En), Status::Missing);
        put(&places, Pack::En);
        assert_eq!(places.status(Pack::En), Status::Downloaded);
        assert_eq!(places.missing_bytes(&[Pack::En]), 0);
        // The catalogue moved on: the noted hash is not its own.
        let f = &Pack::En.files()[0];
        let mut manifest = places.manifest();
        manifest.insert(f.path().into(), "0".repeat(64));
        fs::write(places.packs.join(MANIFEST), serde_json::to_string(&manifest).unwrap()).unwrap();
        assert_eq!(places.status(Pack::En), Status::Outdated);
        assert!(places.status(Pack::En).usable());
        assert_eq!(places.missing_bytes(&[Pack::En]), f.size);
        // A checkout's resources/voice counts as coming with the app, and is never deleted.
        for f in Pack::Ja.files() {
            let path = places.bundled.join(f.path());
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, b"x").unwrap();
        }
        assert_eq!(places.status(Pack::Ja), Status::BuiltIn);
        assert_eq!(places.delete(Pack::Ja).unwrap(), 0);
        assert!(places.bundled.join(Pack::Ja.files()[0].path()).is_file());
    }

    #[test]
    fn deleting_chinese_deletes_its_live_captions_too() {
        let (_s, places) = Scratch::new();
        for p in [Pack::Zh, Pack::Live, Pack::Spk] {
            put(&places, p);
        }
        let freed = places.delete(Pack::Zh).unwrap();
        assert_eq!(freed, Pack::Zh.size() + Pack::Live.size());
        assert_eq!(places.status(Pack::Zh), Status::Missing);
        assert_eq!(places.status(Pack::Live), Status::Missing);
        assert_eq!(places.status(Pack::Spk), Status::Downloaded);
        assert!(!places.packs.join("draft-zh").exists());
    }

    #[test]
    fn the_choice_is_kept_beside_the_packs() {
        let (_s, places) = Scratch::new();
        assert_eq!(Choice::load(&places.packs), None, "nothing chosen yet: the first run asks");
        let mut choice = Choice { decided: true, ..Default::default() };
        choice.want(&[Pack::Live, Pack::Zh]);
        choice.restoring = vec![Pack::Zh];
        choice.save(&places.packs).unwrap();
        assert_eq!(Choice::load(&places.packs), Some(choice));
    }

    #[test]
    fn modelscope_can_be_asked_for_first() {
        let zh = &Pack::Zh.files()[0];
        let sources = preferring(zh.sources(), Some("ModelScope"));
        assert_eq!(sources[0].name, "ModelScope");
        assert_eq!(sources.iter().filter(|s| s.preferred).count(), 1);
        // A file ModelScope does not have keeps its sources as they were.
        let spk = &Pack::Spk.files()[0];
        assert_eq!(preferring(spk.sources(), Some("ModelScope"))[0].name, "Alcyo");
    }

    /// A server for one file that answers ranges and, if asked, breaks off after `cut` bytes of each answer.
    fn serve(body: Vec<u8>, cut: Option<usize>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/f", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            for mut stream in listener.incoming().flatten() {
                let mut from = 0;
                for line in BufReader::new(stream.try_clone().unwrap()).lines().map_while(Result::ok) {
                    if line.is_empty() {
                        break;
                    }
                    if let Some(v) = line.to_lowercase().strip_prefix("range: bytes=") {
                        from = v.split('-').next().and_then(|n| n.parse().ok()).unwrap_or(0);
                    }
                }
                let slice = &body[from..];
                let status = if from > 0 { "206 Partial Content" } else { "200 OK" };
                let _ = stream.write_all(format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", slice.len()).as_bytes());
                let _ = stream.write_all(&slice[..cut.unwrap_or(slice.len()).min(slice.len())]);
            }
        });
        url
    }

    /// A job for one made-up file of `pack`, served from `url`.
    fn job(pack: Pack, body: &[u8], url: &str) -> (Pack, Vec<(Model, VoiceFile)>) {
        let sha: String = Sha256::digest(body).iter().map(|b| format!("{b:02x}")).collect();
        let sha: &'static str = Box::leak(sha.into_boxed_str());
        let name: &'static str = Box::leak(format!("voice/{}/test.onnx", pack.folder()).into_boxed_str());
        let file = VoiceFile { name, size: body.len() as u64, sha256: sha, modelscope: None, huggingface: None, github: None };
        let model = Model { id: name, label: name, file: file.path().into(), size: file.size, sha256: sha.into(), note: None, sources: vec![Source { name: "test".into(), url: url.into(), direct: true, preferred: true }] };
        (pack, vec![(model, file)])
    }

    fn run(jobs: Vec<(Pack, Vec<(Model, VoiceFile)>)>, places: &Places) -> Vec<Event> {
        let mut events = vec![];
        fetch_jobs(jobs, places, &AtomicBool::new(false), &mut |e| {
            if !matches!(e, Event::Progress(_)) {
                events.push(e);
            }
        });
        events
    }

    #[test]
    fn packs_arrive_one_after_another_into_their_folders() {
        let (_s, places) = Scratch::new();
        let (a, b): (Vec<u8>, Vec<u8>) = ((0..300_000_u32).map(|i| i as u8).collect(), (0..200_000_u32).map(|i| (i * 7) as u8).collect());
        let events = run(vec![job(Pack::Zh, &a, &serve(a.clone(), None)), job(Pack::Spk, &b, &serve(b.clone(), None))], &places);
        assert!(matches!(events.as_slice(), [Event::Ready(Pack::Zh), Event::Ready(Pack::Spk), Event::Done]), "{events:?}");
        assert_eq!(fs::read(places.packs.join("sense-voice/test.onnx")).unwrap(), a);
        assert_eq!(fs::read(places.packs.join("speaker/test.onnx")).unwrap(), b);
        assert!(places.manifest().contains_key("speaker/test.onnx"), "each file is noted once checked");
    }

    #[test]
    fn an_interrupted_download_says_so_keeps_what_came_and_resumes() {
        let (_s, places) = Scratch::new();
        let body: Vec<u8> = (0..2_000_000_u32).map(|i| (i % 253) as u8).collect();
        // Cut off part way: it stops, saying what failed, and keeps the part that came.
        let events = run(vec![job(Pack::En, &body, &serve(body.clone(), Some(700_000)))], &places);
        assert!(matches!(events.as_slice(), [Event::Failed(_)]), "{events:?}");
        let part = places.packs.join("parakeet-en/test.onnx.part");
        assert!(fs::metadata(&part).unwrap().len() >= 700_000);
        // Nothing answers at all, as when offline: the part stays.
        let events = run(vec![job(Pack::En, &body, "http://127.0.0.1:9/f")], &places);
        assert!(matches!(events.as_slice(), [Event::Failed(_)]), "{events:?}");
        assert!(part.is_file());
        // The network back: it carries on from there.
        let events = run(vec![job(Pack::En, &body, &serve(body.clone(), None))], &places);
        assert!(matches!(events.as_slice(), [Event::Ready(Pack::En), Event::Done]), "{events:?}");
        assert_eq!(fs::read(places.packs.join("parakeet-en/test.onnx")).unwrap(), body);
    }

    #[test]
    fn too_little_space_stops_before_anything_is_fetched() {
        let (_s, places) = Scratch::new();
        let body = vec![1_u8; 1000];
        let (pack, mut files) = job(Pack::Ja, &body, "http://127.0.0.1:9/f");
        files[0].0.size = u64::MAX / 4;
        let events = run(vec![(pack, files)], &places);
        assert!(matches!(&events[..], [Event::Failed(e)] if e.contains("disk space")), "{events:?}");
    }
}
