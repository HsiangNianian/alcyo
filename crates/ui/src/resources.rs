//! Finding what ships with the app. An install keeps it in `<prefix>/lib/alcyo`, next to `bin/`, or a Mac app in `Contents/Resources/alcyo`,
//! whichever folder the package was unpacked into. A checkout has them in `resources/`.

use alcyo_board::glyphs::HanziSource;
use serde_json::json;
use std::path::{Path, PathBuf};

pub struct Resources {
    pub hanzi: HanziSource,
    /// `llama-server` and its libraries.
    pub binaries: PathBuf,
    /// The speech models that come with the app: the voice detector in a package (the rest are voice packs,
    /// downloaded into the data folder), every model in a checkout that fetched them.
    pub voice: PathBuf,
    /// The PDF library.
    pub pdfium: PathBuf,
    /// The meeting search model, shipped with every package; a checkout may not have it.
    pub search: PathBuf,
    pub installed: bool,
}

fn installed_dir() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("ALCYO_RESOURCES") {
        return Some(PathBuf::from(dir));
    }
    // Canonical, so a symlink on PATH still leads back to the real install.
    let exe = std::env::current_exe().ok()?.canonicalize().ok()?;
    // <prefix>/bin/alcyo beside <prefix>/lib/alcyo, or inside an app on a Mac.
    // An iPad app is flat: the program and its assets side by side in Alcyo.app. Not "resources": a folder
    // of that name, which the file system takes for "Resources", makes it an old-style bundle to codesign.
    #[cfg(target_os = "ios")]
    return Some(exe.parent()?.join("assets")).filter(|dir| dir.join("glyphs.pack").is_file());
    #[cfg(not(target_os = "ios"))]
    {
        let top = exe.parent()?.parent()?;
        ["lib/alcyo", "Resources/alcyo"].into_iter().map(|d| top.join(d)).find(|dir| dir.join("glyphs.pack").is_file())
    }
}

/// A checkout of the repository above this executable, as when running from `target/`.
fn checkout_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?.canonicalize().ok()?;
    exe.ancestors().find(|d| d.join("crates/ui/Cargo.toml").is_file()).map(Path::to_path_buf)
}

pub fn locate(data: &Path) -> Resources {
    let mut found = locate_in(data);
    // To try the voice packs from a checkout, which has every model: another folder taken as what came with the app.
    if let Some(dir) = std::env::var_os("ALCYO_BUNDLED_VOICE") {
        found.voice = PathBuf::from(dir);
    }
    found
}

fn locate_in(data: &Path) -> Resources {
    if let Some(dir) = installed_dir() {
        return Resources { hanzi: HanziSource::Pack(dir.join("glyphs.pack")), binaries: dir.join("llama"), pdfium: dir.join("pdfium"), voice: dir.join("voice"), search: dir.join("search"), installed: true };
    }
    let root = checkout_dir().unwrap_or_else(|| PathBuf::from("."));
    let bundled = root.join("resources/voice");
    Resources {
        hanzi: HanziSource::Directory(root.join("assets/hanzi")),
        binaries: root.join("resources/llama"),
        pdfium: root.join("target/pdfium"),
        voice: if bundled.join("silero_vad.onnx").is_file() { bundled } else { data.join("models/voice") },
        search: root.join("resources/search"),
        installed: false,
    }
}

impl Resources {
    fn hanzi_path(&self) -> &Path {
        match &self.hanzi {
            HanziSource::Pack(path) | HanziSource::Directory(path) => path,
            // Built in, so always present; the check looks for a file that is.
            HanziSource::PackBytes(_) => Path::new("/"),
        }
    }

    /// Whether everything an install promises is present, as JSON for packaging checks.
    pub fn self_check(&self) -> (bool, String) {
        // One package for every platform since the voice packs (0.35): the speech models are downloaded, so a package
        // promises only the voice detector besides the rest.
        let checks: Vec<(&str, PathBuf)> = vec![
            ("glyphs", self.hanzi_path().to_path_buf()),
            ("llama-server", self.binaries.join(alcyo_agent::LLAMA_SERVER)),
            ("voice activity model", self.voice.join("silero_vad.onnx")),
            ("PDF library", alcyo_import::pdf::library_file(&self.pdfium)),
            // Every package carries it.
            ("meeting search model", self.search.join(alcyo_models::embedding().file)),
        ];
        let mut rows: Vec<_> = checks.iter().map(|(name, path)| json!({ "name": name, "path": path, "ok": path.exists() })).collect();
        // Built into the program, so never missing as a file, but each must load and draw its languages.
        let fonts = alcyo_fonts::check();
        rows.extend(fonts.iter().map(|(name, problem)| json!({ "name": format!("font {name}"), "built in": true, "ok": problem.is_none(), "problem": problem })));
        let ok = checks.iter().all(|(_, path)| path.exists()) && fonts.iter().all(|(_, problem)| problem.is_none());
        let gpus = alcyo_agent::list_gpus(&self.binaries);
        let report = json!({
            "version": alcyo_update::VERSION,
            "installed": self.installed,
            "variant": "all",
            "ok": ok,
            "checks": rows,
            "gpus": gpus.iter().map(|g| json!({ "id": g.id, "name": g.name, "mib": g.total_mib, "discrete": g.discrete })).collect::<Vec<_>>(),
        });
        (ok, report.to_string())
    }
}
