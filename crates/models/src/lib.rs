//! The board models too large for the installer: what can be downloaded, and downloading it.
//!
//! Every file is on Alcyo's own mirror (models.alcyo.app, a Cloudflare R2 bucket filled by the models-mirror
//! workflow) as well as upstream. The mirror and ModelScope are raced and the faster is taken; Hugging Face
//! and the others follow only if those fail. Sources are tried in turn, keeping what has arrived: the same
//! file from any source is verified the same way. Unfinished data waits in `<name>.part`, so a cancelled or broken download resumes. The file
//! only takes its real name after its size and SHA-256 match, so a damaged download is never used.

pub mod net;
use anyhow::{anyhow, bail, Context, Result};
use sha2::{Digest, Sha256};
#[cfg(unix)]
use std::ffi::CString;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// Which model to use, in the same file the older app reads.
const SELECTED: &str = "selected.txt";
/// Room left over after a download, so the disk does not end up completely full.
const SPACE_MARGIN: u64 = 1_000_000_000;
const CHUNK: usize = 1 << 20;
const STALL: Duration = Duration::from_secs(30);

#[derive(Clone, Debug)]
pub struct Source {
    pub name: String,
    pub url: String,
    /// Skip any system proxy: domestic mirrors are far slower through one.
    pub direct: bool,
    /// Raced against the other preferred sources at the start, the fastest taken. The rest are fallbacks,
    /// tried in their order only once the preferred ones have failed.
    pub preferred: bool,
}

/// Alcyo's own copy of every model and voice file, served from Cloudflare R2.
pub const MIRROR: &str = "https://models.alcyo.app";

/// Where a file lives on the mirror: `<kind>/<first 16 hex of its SHA-256>/<file>`. The hash in the key means
/// a key never changes content, so the mirror can be cached forever, and a new upstream file takes a new key.
pub fn mirror_key(kind: &str, sha256: &str, file: &str) -> String {
    format!("{kind}/{}/{file}", &sha256[..16.min(sha256.len())])
}

impl Source {
    /// The file on Alcyo's mirror, through the system's proxy like any foreign site.
    pub fn mirror(key: &str) -> Self {
        Self { name: "Alcyo".into(), url: format!("{MIRROR}/{key}"), direct: false, preferred: true }
    }

    pub fn modelscope(path: &str) -> Self {
        Self { name: "ModelScope".into(), url: format!("https://modelscope.cn/models/{path}"), direct: true, preferred: true }
    }

    /// A Hugging Face path, on hf-mirror and then Hugging Face itself.
    pub fn huggingface(path: &str) -> [Self; 2] {
        [
            Self { name: "hf-mirror".into(), url: format!("https://hf-mirror.com/{path}"), direct: false, preferred: false },
            Self { name: "Hugging Face".into(), url: format!("https://huggingface.co/{path}"), direct: false, preferred: false },
        ]
    }
}

#[derive(Clone, Debug)]
pub struct Model {
    pub id: &'static str,
    pub label: &'static str,
    pub file: String,
    pub size: u64,
    pub sha256: String,
    /// What it falls short on, in English and Chinese. A model with a caveat is never recommended.
    pub note: Option<(&'static str, &'static str)>,
    pub sources: Vec<Source>,
}

/// The mirror's copy and ModelScope's, raced, then hf-mirror and Hugging Face.
fn sources(key: String, modelscope: &str, huggingface: &str) -> Vec<Source> {
    let mut all = vec![Source::mirror(&key), Source::modelscope(modelscope)];
    all.extend(Source::huggingface(huggingface));
    all
}

/// A board model's entry; its mirror key is under `board/`.
fn board(id: &'static str, label: &'static str, file: &str, size: u64, sha256: &str, note: Option<(&'static str, &'static str)>, modelscope: &str, huggingface: &str) -> Model {
    Model {
        id,
        label,
        file: file.into(),
        size,
        sha256: sha256.into(),
        note,
        sources: sources(mirror_key("board", sha256, file), modelscope, huggingface),
    }
}

/// The best boards first, as the first one present is used when none was chosen; among those without a
/// caveat, smallest first. Qwen's files and hashes are the older app's, so a model it downloaded is reused.
pub fn catalog() -> Vec<Model> {
    vec![
        // The board model by default since #306: on the dev set 0.735 item F1 against Qwen3.5-4B's 0.609, and
        // 0.46 against 0.17 on a real meeting, as steady between passes, at the same speed. It reads no images.
        // Hugging Face pinned to the revision measured; ModelScope has no revisions, and the hash holds it.
        board(
            "qwen3-4b-2507",
            "Qwen3-4B",
            "Qwen3-4B-Instruct-2507-Q4_K_M.gguf",
            2_497_281_120,
            "3605803b982cb64aead44f6c1b2ae36e3acdb41d8e46c8a94c6533bc4c67e597",
            None,
            "unsloth/Qwen3-4B-Instruct-2507-GGUF/resolve/master/Qwen3-4B-Instruct-2507-Q4_K_M.gguf",
            "unsloth/Qwen3-4B-Instruct-2507-GGUF/resolve/a06e946bb6b655725eafa393f4a9745d460374c9/Qwen3-4B-Instruct-2507-Q4_K_M.gguf",
        ),
        // Kept for reading images: its vision add-on understands a picture, not only its text.
        board(
            "qwen3.5-4b",
            "Qwen3.5-4B",
            "Qwen3.5-4B-Q4_K_M.gguf",
            2_740_937_888,
            "00fe7986ff5f6b463e62455821146049db6f9313603938a70800d1fb69ef11a4",
            Some(("Reads images with its vision add-on; draws boards less well than Qwen3-4B", "可以装看图插件读懂图片；画板书不如 Qwen3-4B")),
            "unsloth/Qwen3.5-4B-GGUF/resolve/master/Qwen3.5-4B-Q4_K_M.gguf",
            "unsloth/Qwen3.5-4B-GGUF/resolve/main/Qwen3.5-4B-Q4_K_M.gguf",
        ),
    ]
}

/// The model offered first: the first without a caveat. Every model in the catalog runs on any machine, so
/// graphics memory no longer decides it (Qwen3.5-9B, the one that needed 6 GB, was dropped).
pub fn recommended() -> &'static str {
    let models = catalog();
    models.iter().find(|m| m.note.is_none()).unwrap_or(&models[0]).id
}

/// What lets a board model read images, by the board model's file name. It is downloaded the same way
/// and never chosen as a model itself.
pub fn vision_for(model_file: &str) -> Option<Model> {
    if !model_file.starts_with("Qwen3.5-4B") {
        return None;
    }
    let (file, sha256) = ("Qwen3.5-4B-mmproj-F16.gguf", "cd88edcf8d031894960bb0c9c5b9b7e1fea6ebee02b9f7ce925a00d12891f864");
    let repo = "unsloth/Qwen3.5-4B-GGUF";
    Some(Model {
        id: "qwen3.5-4b-vision",
        label: "Vision add-on · Qwen3.5-4B",
        file: file.into(),
        size: 672_423_616,
        sha256: sha256.into(),
        note: None,
        sources: sources(mirror_key("vision", sha256, file), &format!("{repo}/resolve/master/mmproj-F16.gguf"), &format!("{repo}/resolve/main/mmproj-F16.gguf")),
    })
}

/// What finds things in past meetings by what they mean: EmbeddingGemma 300M, never chosen as a board model.
/// Every package carries it (`alcyo-tools fetch search`); a checkout without it downloads it when meeting
/// search is turned on. It is used and shipped under the Gemma Terms of Use (assets/licenses/gemma-*).
pub fn embedding() -> Model {
    let (file, sha256) = ("embeddinggemma-300M-Q8_0.gguf", "b5ce9d77a3fc4b3b39ccb5643c36777911cc4eb46a66962eadfa3f5f60490d63");
    let path = format!("ggml-org/embeddinggemma-300M-GGUF/resolve/{{}}/{file}");
    Model {
        id: "embeddinggemma-300m",
        label: "Meeting search · EmbeddingGemma 300M",
        file: file.to_string(),
        size: 333_590_944,
        sha256: sha256.to_string(),
        note: None,
        sources: sources(mirror_key("search", sha256, file), &path.replace("{}", "master"), &path.replace("{}", "main")),
    }
}

pub fn installed(model: &Model, dir: &Path) -> bool {
    dir.join(&model.file).is_file()
}

/// The file the user chose, or else the first one present.
pub fn current(dir: &Path) -> Option<PathBuf> {
    let chosen = fs::read_to_string(dir.join(SELECTED)).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    // A choice since taken out of the catalog falls back to what is in it, as if nothing had been chosen.
    let listed = |file: &String| catalog().iter().any(|m| &m.file == file);
    if let Some(path) = chosen.filter(listed).map(|file| dir.join(file)).filter(|p| p.is_file()) {
        return Some(path);
    }
    catalog().iter().find(|m| installed(m, dir)).map(|m| dir.join(&m.file))
}

pub fn select(model: &Model, dir: &Path) -> Result<()> {
    fs::create_dir_all(dir)?;
    fs::write(dir.join(SELECTED), &model.file).context("saving the chosen model")
}

fn part_of(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".part");
    PathBuf::from(name)
}

/// Bytes free on the disk holding `dir`.
#[cfg(windows)]
pub fn free_space(dir: &Path) -> Result<u64> {
    use std::os::windows::ffi::OsStrExt;
    let wide: Vec<u16> = dir.as_os_str().encode_wide().chain([0]).collect();
    let mut free = 0_u64;
    // SAFETY: `wide` is a NUL-terminated path and `free` is written by the call.
    let ok = unsafe { windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(wide.as_ptr(), &mut free, std::ptr::null_mut(), std::ptr::null_mut()) };
    if ok == 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(free)
}

/// Bytes free on the disk holding `dir`.
#[cfg(unix)]
pub fn free_space(dir: &Path) -> Result<u64> {
    let path = CString::new(dir.as_os_str().as_bytes())?;
    // SAFETY: `path` is a valid C string and `stat` is written by the call before being read.
    unsafe {
        let mut stat: libc::statvfs = std::mem::zeroed();
        if libc::statvfs(path.as_ptr(), &mut stat) != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(u64::from(stat.f_bavail).saturating_mul(u64::from(stat.f_frsize)))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Progress {
    pub done: u64,
    pub total: u64,
    /// Bytes per second over the last moments; 0 until known.
    pub speed: u64,
    pub source: Option<String>,
    /// Checking what an earlier attempt left, before asking for the rest.
    pub verifying: bool,
    /// Trying each source, to take the fastest.
    pub testing: bool,
}

#[derive(Debug)]
pub enum Outcome {
    Finished(PathBuf),
    Cancelled,
}

/// A client for one source, through the proxy the user chose (see [`net`]). A source marked direct skips the
/// system's proxy, as a mirror in China is faster without it.
fn agent(url: &str, direct: bool) -> ureq::Agent {
    net::agent_for(url, direct, Duration::from_secs(20)).timeout_read(STALL).build()
}

/// How much of the file a source is asked for to tell how fast it is, and how long it may take.
const PROBE_BYTES: u64 = 1 << 20;
const PROBE_TIME: Duration = Duration::from_secs(5);

/// The sources in the order to try them: the preferred ones fastest first, then the fallbacks in the catalog's
/// order. Each preferred source is asked at once for a little of the file from `from` on, through the proxy it
/// would download through, and timed from the request to the last byte, so a slow start counts. One that fails
/// or sends nothing goes after the other preferred ones, still tried if they drop. Fallbacks are never asked
/// until then: Hugging Face is slow or out of reach where the mirror and ModelScope are not.
fn fastest_first<'a>(sources: &'a [Source], from: u64, cancel: &AtomicBool) -> Vec<&'a Source> {
    let (preferred, fallbacks): (Vec<&Source>, Vec<&Source>) = sources.iter().partition(|s| s.preferred);
    let speeds: Vec<u64> = if preferred.len() < 2 {
        vec![0; preferred.len()]
    } else {
        std::thread::scope(|scope| {
            let probes: Vec<_> = preferred
                .iter()
                .map(|source| {
                    scope.spawn(move || -> u64 {
                        let started = Instant::now();
                        let agent = net::agent_for(&source.url, source.direct, PROBE_TIME).timeout(PROBE_TIME).build();
                        let Ok(response) = agent.get(&source.url).set("Range", &format!("bytes={from}-{}", from + PROBE_BYTES - 1)).call() else { return 0 };
                        let mut reader = response.into_reader().take(PROBE_BYTES);
                        let (mut got, mut buffer) = (0_u64, vec![0; 64 * 1024]);
                        while started.elapsed() < PROBE_TIME && !cancel.load(Ordering::Relaxed) {
                            match reader.read(&mut buffer) {
                                Ok(0) | Err(_) => break,
                                Ok(n) => got += n as u64,
                            }
                        }
                        (got as f64 / started.elapsed().as_secs_f64().max(0.001)) as u64
                    })
                })
                .collect();
            probes.into_iter().map(|p| p.join().unwrap_or(0)).collect()
        })
    };
    let mut order: Vec<usize> = (0..preferred.len()).collect();
    // Stable, so sources equally fast, or all failing, keep the catalog's order.
    order.sort_by_key(|&i| std::cmp::Reverse(speeds[i]));
    order.into_iter().map(|i| preferred[i]).chain(fallbacks).collect()
}

/// Keep `dir` out of the iPad's backups, to iCloud or a computer: models are gigabytes that can be downloaded again,
/// and Apple asks apps not to back such files up. Elsewhere it does nothing.
pub fn exclude_from_backup(dir: &Path) -> Result<()> {
    #[cfg(target_os = "ios")]
    {
        use std::ffi::c_void;
        type Ref = *const c_void;
        #[link(name = "CoreFoundation", kind = "framework")]
        extern "C" {
            static kCFAllocatorDefault: Ref;
            static kCFBooleanTrue: Ref;
            static kCFURLIsExcludedFromBackupKey: Ref;
            fn CFURLCreateFromFileSystemRepresentation(allocator: Ref, path: *const u8, length: isize, is_directory: u8) -> Ref;
            fn CFURLSetResourcePropertyForKey(url: Ref, key: Ref, value: Ref, error: *mut Ref) -> u8;
            fn CFRelease(object: Ref);
        }
        let path = dir.as_os_str().as_bytes();
        // SAFETY: the path is valid for its length; the URL is released once used, and no error object is asked for.
        let done = unsafe {
            let url = CFURLCreateFromFileSystemRepresentation(kCFAllocatorDefault, path.as_ptr(), path.len() as isize, 1);
            if url.is_null() {
                bail!("{} is not a path the system takes", dir.display());
            }
            let done = CFURLSetResourcePropertyForKey(url, kCFURLIsExcludedFromBackupKey, kCFBooleanTrue, std::ptr::null_mut());
            CFRelease(url);
            done
        };
        if done == 0 {
            bail!("could not keep {} out of backups", dir.display());
        }
    }
    #[cfg(not(target_os = "ios"))]
    let _ = dir;
    Ok(())
}

/// Download a model, resuming anything left by an earlier attempt. `report` runs a few times a second.
pub fn download(model: &Model, dir: &Path, cancel: &AtomicBool, report: &mut dyn FnMut(Progress)) -> Result<Outcome> {
    let path = dir.join(&model.file);
    if path.is_file() {
        return Ok(Outcome::Finished(path));
    }
    fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    // Whatever is downloaded into it, and the unfinished part too.
    if let Err(e) = exclude_from_backup(dir) {
        eprintln!("{e:#}");
    }
    let part = part_of(&path);

    // Resuming means hashing what is already there, so a large leftover takes a moment.
    let mut hasher = Sha256::new();
    let mut have = 0_u64;
    if let Ok(mut file) = File::open(&part) {
        report(Progress { done: 0, total: model.size, speed: 0, source: None, verifying: true, testing: false });
        let mut buffer = vec![0; CHUNK];
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Ok(Outcome::Cancelled);
            }
            let n = file.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            hasher.update(&buffer[..n]);
            have += n as u64;
        }
    }
    if have > model.size {
        fs::remove_file(&part)?;
        hasher = Sha256::new();
        have = 0;
    }
    let needed = (model.size - have) + SPACE_MARGIN;
    let free = free_space(dir)?;
    if needed > free {
        let gb = |n: u64| n as f64 / 1e9;
        bail!("Not enough disk space for {}: it needs {:.1} GB and {:.1} GB is free.", model.label, gb(needed), gb(free));
    }

    let mut failure = anyhow!("all download sources failed");
    let mut buffer = vec![0; CHUNK];
    // Every start, a resumed one too, tries each source first: which is fastest changes with the network
    // and the proxy, and a source is kept to the end unless it fails.
    let sources = if have < model.size {
        if model.sources.iter().filter(|s| s.preferred).count() > 1 {
            report(Progress { done: have, total: model.size, speed: 0, source: None, verifying: false, testing: true });
        }
        fastest_first(&model.sources, have, cancel)
    } else {
        model.sources.iter().collect()
    };
    if cancel.load(Ordering::Relaxed) {
        return Ok(Outcome::Cancelled);
    }
    for source in sources {
        if have >= model.size {
            break;
        }
        if cancel.load(Ordering::Relaxed) {
            return Ok(Outcome::Cancelled);
        }
        report(Progress { done: have, total: model.size, speed: 0, source: Some(source.name.clone()), verifying: false, testing: false });
        let mut request = agent(&source.url, source.direct).get(&source.url);
        if have > 0 {
            request = request.set("Range", &format!("bytes={have}-"));
        }
        let response = match request.call() {
            Ok(response) => response,
            Err(e) => {
                failure = anyhow!("{}: {e}", source.name);
                continue;
            }
        };
        // A source that ignores the range sends the whole file, so start over from it.
        let append = have > 0 && response.status() == 206;
        if have > 0 && !append {
            hasher = Sha256::new();
            have = 0;
        }
        let mut file = OpenOptions::new().create(true).write(true).append(append).truncate(!append).open(&part).with_context(|| format!("writing {}", part.display()))?;
        let mut reader = response.into_reader();
        let (mut last_report, mut window_start, mut window_bytes, mut speed) = (Instant::now(), Instant::now(), 0_u64, 0_u64);
        loop {
            if cancel.load(Ordering::Relaxed) {
                file.flush()?;
                return Ok(Outcome::Cancelled);
            }
            let n = match reader.read(&mut buffer) {
                Ok(n) => n,
                // Keep what arrived and let the next source continue from there.
                Err(e) => {
                    failure = anyhow!("{}: {e}", source.name);
                    break;
                }
            };
            if n == 0 {
                break;
            }
            file.write_all(&buffer[..n]).with_context(|| format!("writing {}", part.display()))?;
            hasher.update(&buffer[..n]);
            have += n as u64;
            window_bytes += n as u64;
            if window_start.elapsed() >= Duration::from_secs(1) {
                speed = (window_bytes as f64 / window_start.elapsed().as_secs_f64()) as u64;
                window_start = Instant::now();
                window_bytes = 0;
            }
            if last_report.elapsed() >= Duration::from_millis(200) {
                last_report = Instant::now();
                report(Progress { done: have, total: model.size, speed, source: Some(source.name.clone()), verifying: false, testing: false });
            }
        }
        file.sync_all()?;
    }
    if have < model.size {
        return Err(failure.context(format!("{} stopped at {have} of {} bytes; the next attempt continues from there", model.label, model.size)));
    }
    let actual: String = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect();
    if have != model.size || actual != model.sha256 {
        // Bad data must not be resumed from, or every retry would fail the same way.
        fs::remove_file(&part)?;
        bail!("{} failed verification and was discarded. Please try again.", model.label);
    }
    fs::rename(&part, &path).with_context(|| format!("finishing {}", path.display()))?;
    Ok(Outcome::Finished(path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::net::{TcpListener, TcpStream};
    use std::sync::atomic::AtomicU32;
    use std::sync::{Arc, Mutex};

    static NEXT: AtomicU32 = AtomicU32::new(0);

    struct Dir(PathBuf);

    impl Dir {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("alcyo-models-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// How the test server treats a request.
    #[derive(Clone, Copy)]
    enum Behaviour {
        Honest,
        /// Answers with the whole file even when asked for a range.
        IgnoresRange,
        /// Cuts the connection after this many bytes of the body.
        Breaks(usize),
        Missing,
        /// Sends slowly: a short pause after every 16 KB.
        Slow,
    }

    fn serve(body: Vec<u8>, behaviour: Behaviour) -> (String, Arc<Mutex<Vec<Option<usize>>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/model", listener.local_addr().unwrap());
        let ranges = Arc::new(Mutex::new(vec![]));
        let seen = ranges.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                answer(stream, &body, behaviour, &seen);
            }
        });
        (url, ranges)
    }

    fn answer(mut stream: TcpStream, body: &[u8], behaviour: Behaviour, seen: &Mutex<Vec<Option<usize>>>) {
        let mut range = None;
        let reader = BufReader::new(stream.try_clone().unwrap());
        for line in reader.lines().map_while(Result::ok) {
            if line.is_empty() {
                break;
            }
            if let Some(v) = line.to_lowercase().strip_prefix("range: bytes=") {
                range = v.split('-').next().and_then(|from| from.parse::<usize>().ok());
            }
        }
        seen.lock().unwrap().push(range);
        let (status, slice) = match (behaviour, range) {
            (Behaviour::Missing, _) => ("404 Not Found", &body[..0]),
            (Behaviour::Honest | Behaviour::Breaks(_) | Behaviour::Slow, Some(from)) => ("206 Partial Content", &body[from..]),
            _ => ("200 OK", body),
        };
        let header = format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", slice.len());
        stream.write_all(header.as_bytes()).unwrap();
        let sent = if let Behaviour::Breaks(n) = behaviour { &slice[..n.min(slice.len())] } else { slice };
        if let Behaviour::Slow = behaviour {
            for chunk in sent.chunks(16 * 1024) {
                if stream.write_all(chunk).is_err() {
                    return;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            return;
        }
        let _ = stream.write_all(sent);
    }

    fn model(body: &[u8], urls: &[&str]) -> Model {
        Model {
            id: "test",
            label: "Test",
            file: "model.gguf".into(),
            size: body.len() as u64,
            sha256: Sha256::digest(body).iter().map(|b| format!("{b:02x}")).collect(),
            note: None,
            sources: urls.iter().enumerate().map(|(i, u)| Source { name: format!("source {i}"), url: (*u).to_string(), direct: true, preferred: true }).collect(),
        }
    }

    fn body() -> Vec<u8> {
        (0..3_000_000_u32).map(|i| (i % 251) as u8).collect()
    }

    fn fetch(m: &Model, dir: &Path) -> Result<Outcome> {
        download(m, dir, &AtomicBool::new(false), &mut |_| {})
    }

    #[test]
    fn the_fastest_source_is_tried_first_from_where_a_download_stands() {
        let data = body();
        let (slow, slow_ranges) = serve(data.clone(), Behaviour::Slow);
        let (fast, fast_ranges) = serve(data.clone(), Behaviour::Honest);
        let (gone, _) = serve(data.clone(), Behaviour::Missing);
        let m = model(&data, &[&gone, &slow, &fast]);
        let order: Vec<&str> = fastest_first(&m.sources, 500_000, &AtomicBool::new(false)).iter().map(|s| s.url.as_str()).collect();
        assert_eq!(order, [fast.as_str(), slow.as_str(), gone.as_str()], "fastest first, a missing one last");
        assert_eq!(*fast_ranges.lock().unwrap(), [Some(500_000)], "asked from where the download stands");
        assert_eq!(*slow_ranges.lock().unwrap(), [Some(500_000)]);
        // A download asks the fast one for the file after trying them all, the slow one never again.
        let dir = Dir::new();
        fetch(&m, &dir.0).unwrap();
        assert_eq!(fast_ranges.lock().unwrap().len(), 3, "tried twice, then the download");
        assert_eq!(slow_ranges.lock().unwrap().len(), 2, "only tried");
        assert_eq!(fs::read(dir.0.join(&m.file)).unwrap(), data);
    }

    #[test]
    fn fallbacks_are_never_raced_and_come_after_the_preferred_sources() {
        let data = body();
        let (slow, _) = serve(data.clone(), Behaviour::Slow);
        let (fast, fast_ranges) = serve(data.clone(), Behaviour::Honest);
        let (fallback, fallback_ranges) = serve(data.clone(), Behaviour::Honest);
        let mut m = model(&data, &[&fallback, &slow, &fast]);
        m.sources[0].preferred = false;
        let order: Vec<&str> = fastest_first(&m.sources, 0, &AtomicBool::new(false)).iter().map(|s| s.url.as_str()).collect();
        assert_eq!(order, [fast.as_str(), slow.as_str(), fallback.as_str()]);
        assert!(fallback_ranges.lock().unwrap().is_empty(), "a fallback is not asked while a preferred source works");
        // With one preferred source there is nothing to race: it goes first, asked only for the download.
        m.sources[1].preferred = false;
        let dir = Dir::new();
        let before = fast_ranges.lock().unwrap().len();
        fetch(&m, &dir.0).unwrap();
        assert_eq!(fast_ranges.lock().unwrap().len(), before + 1);
        assert!(fallback_ranges.lock().unwrap().is_empty());
    }

    #[test]
    fn every_file_is_on_the_mirror_first_and_modelscope_races_it() {
        let all: Vec<Model> = catalog().into_iter().flat_map(|m| [vision_for(&m.file), Some(m)]).flatten().chain([embedding()]).collect();
        for m in &all {
            let names: Vec<(&str, bool)> = m.sources.iter().map(|s| (s.name.as_str(), s.preferred)).collect();
            assert_eq!(names, [("Alcyo", true), ("ModelScope", true), ("hf-mirror", false), ("Hugging Face", false)], "{}", m.id);
            assert!(m.sources[0].url.starts_with(&format!("{MIRROR}/")), "{}", m.sources[0].url);
            assert!(m.sources[0].url.ends_with(&format!("/{}/{}", &m.sha256[..16], m.file)), "{}", m.sources[0].url);
        }
        assert_eq!(mirror_key("board", &catalog()[0].sha256, &catalog()[0].file), "board/3605803b982cb64a/Qwen3-4B-Instruct-2507-Q4_K_M.gguf");
    }

    #[test]
    fn a_download_arrives_verified_under_its_real_name() {
        let (dir, data) = (Dir::new(), body());
        let (url, _) = serve(data.clone(), Behaviour::Honest);
        let m = model(&data, &[&url]);
        let Outcome::Finished(path) = fetch(&m, &dir.0).unwrap() else { panic!("cancelled") };
        assert_eq!(fs::read(&path).unwrap(), data);
        assert!(!part_of(&path).exists());
        assert!(installed(&m, &dir.0));
    }

    #[test]
    fn a_leftover_resumes_with_a_range_request() {
        let (dir, data) = (Dir::new(), body());
        let m = model(&data, &[]);
        fs::write(part_of(&dir.0.join(&m.file)), &data[..1_000_000]).unwrap();
        let (url, ranges) = serve(data.clone(), Behaviour::Honest);
        let m = model(&data, &[&url]);
        fetch(&m, &dir.0).unwrap();
        assert_eq!(*ranges.lock().unwrap(), [Some(1_000_000)]);
        assert_eq!(fs::read(dir.0.join(&m.file)).unwrap(), data);
    }

    #[test]
    fn a_source_ignoring_the_range_starts_the_file_over() {
        let (dir, data) = (Dir::new(), body());
        let m = model(&data, &[]);
        fs::write(part_of(&dir.0.join(&m.file)), &data[..1_000_000]).unwrap();
        let (url, _) = serve(data.clone(), Behaviour::IgnoresRange);
        let m = model(&data, &[&url]);
        fetch(&m, &dir.0).unwrap();
        assert_eq!(fs::read(dir.0.join(&m.file)).unwrap(), data);
    }

    #[test]
    fn a_broken_source_hands_over_to_the_next_keeping_what_arrived() {
        let (dir, data) = (Dir::new(), body());
        // The one that breaks is the faster, so it is tried first.
        let (bad, _) = serve(data.clone(), Behaviour::Breaks(1_500_000));
        let (good, ranges) = serve(data.clone(), Behaviour::Slow);
        let m = model(&data, &[&bad, &good]);
        fetch(&m, &dir.0).unwrap();
        let asked = ranges.lock().unwrap().last().copied().flatten().unwrap();
        assert!(asked > 0, "the next source must continue, not restart");
        assert_eq!(fs::read(dir.0.join(&m.file)).unwrap(), data);
    }

    #[test]
    fn a_missing_source_is_skipped() {
        let (dir, data) = (Dir::new(), body());
        let (bad, _) = serve(vec![], Behaviour::Missing);
        let (good, _) = serve(data.clone(), Behaviour::Honest);
        fetch(&model(&data, &[&bad, &good]), &dir.0).unwrap();
        assert!(dir.0.join("model.gguf").is_file());
    }

    #[test]
    fn when_every_source_fails_the_progress_is_kept_and_the_error_says_so() {
        let (dir, data) = (Dir::new(), body());
        let (bad, _) = serve(data.clone(), Behaviour::Breaks(1_000_000));
        let m = model(&data, &[&bad]);
        let error = fetch(&m, &dir.0).unwrap_err();
        assert!(format!("{error:#}").contains("continues"), "{error:#}");
        let part = part_of(&dir.0.join(&m.file));
        assert!(fs::metadata(&part).unwrap().len() >= 1_000_000);
        assert!(!dir.0.join(&m.file).exists());
    }

    #[test]
    fn corrupt_data_is_discarded_never_installed() {
        let (dir, data) = (Dir::new(), body());
        let mut wrong = data.clone();
        wrong[100] ^= 0xff;
        let (url, _) = serve(wrong, Behaviour::Honest);
        let m = model(&data, &[&url]);
        assert!(fetch(&m, &dir.0).is_err());
        assert!(!dir.0.join(&m.file).exists());
        assert!(!part_of(&dir.0.join(&m.file)).exists(), "a bad leftover would poison every retry");
    }

    #[test]
    fn cancelling_keeps_what_arrived_for_later() {
        let (dir, data) = (Dir::new(), body());
        let (url, _) = serve(data.clone(), Behaviour::Honest);
        let m = model(&data, &[&url]);
        let cancel = AtomicBool::new(true);
        assert!(matches!(download(&m, &dir.0, &cancel, &mut |_| {}).unwrap(), Outcome::Cancelled));
        assert!(!dir.0.join(&m.file).exists());
        fetch(&m, &dir.0).unwrap();
        assert_eq!(fs::read(dir.0.join(&m.file)).unwrap(), data);
    }

    #[test]
    fn not_enough_disk_space_is_said_before_downloading() {
        let (dir, data) = (Dir::new(), body());
        let (url, ranges) = serve(data.clone(), Behaviour::Honest);
        let mut m = model(&data, &[&url]);
        m.size = u64::MAX / 4;
        let error = fetch(&m, &dir.0).unwrap_err();
        assert!(error.to_string().contains("disk space"), "{error}");
        assert!(ranges.lock().unwrap().is_empty(), "nothing should be fetched");
    }

    #[test]
    fn the_choice_of_model_follows_the_file_then_what_is_present() {
        let dir = Dir::new();
        assert_eq!(current(&dir.0), None);
        let models = catalog();
        fs::write(dir.0.join(&models[1].file), b"x").unwrap();
        assert_eq!(current(&dir.0), Some(dir.0.join(&models[1].file)));
        fs::write(dir.0.join(&models[0].file), b"x").unwrap();
        assert_eq!(current(&dir.0), Some(dir.0.join(&models[0].file)));
        select(&models[1], &dir.0).unwrap();
        assert_eq!(current(&dir.0), Some(dir.0.join(&models[1].file)));
    }

    #[test]
    fn the_recommended_model_draws_the_best_boards() {
        // Qwen3-4B draws the best boards of the small models (#306); Qwen3.5-4B is there for reading images.
        assert_eq!(recommended(), "qwen3-4b-2507");
    }

    #[test]
    fn a_saved_choice_of_the_dropped_9b_falls_back_to_the_default() {
        let dir = Dir::new();
        fs::write(dir.0.join("Qwen3.5-9B-Q4_K_M.gguf"), b"x").unwrap();
        fs::write(dir.0.join(SELECTED), "Qwen3.5-9B-Q4_K_M.gguf").unwrap();
        assert_eq!(current(&dir.0), None, "the 9B file left on disk is not used");
        fs::write(dir.0.join(&catalog()[0].file), b"x").unwrap();
        assert_eq!(current(&dir.0), Some(dir.0.join(&catalog()[0].file)));
        assert!(vision_for("Qwen3.5-9B-Q4_K_M.gguf").is_none());
    }

    #[test]
    fn a_choice_no_longer_listed_falls_back_to_the_catalog() {
        let dir = Dir::new();
        let models = catalog();
        fs::write(dir.0.join("Retired-1B-Q4_K_M.gguf"), b"x").unwrap();
        fs::write(dir.0.join(SELECTED), "Retired-1B-Q4_K_M.gguf").unwrap();
        assert_eq!(current(&dir.0), None);
        fs::write(dir.0.join(&models[1].file), b"x").unwrap();
        assert_eq!(current(&dir.0), Some(dir.0.join(&models[1].file)));
    }

    #[test]
    fn the_search_model_is_an_add_on_and_never_a_board_model() {
        let m = embedding();
        assert!(m.sources.iter().any(|s| s.url.starts_with("https://modelscope.cn/models/ggml-org/embeddinggemma-300M-GGUF/resolve/master/")));
        assert!(catalog().iter().all(|c| c.file != m.file));
    }

    #[test]
    fn only_models_that_can_see_have_a_vision_add_on() {
        let has = |id: &str| vision_for(&catalog().into_iter().find(|m| m.id == id).unwrap().file).is_some();
        assert!(has("qwen3.5-4b"));
        assert!(!has("qwen3-4b-2507"), "Qwen3-4B reads no images");
    }

    #[test]
    fn every_download_has_its_own_file_name() {
        let mut names: Vec<String> = catalog().into_iter().flat_map(|m| [vision_for(&m.file).map(|v| v.file), Some(m.file)]).flatten().collect();
        let total = names.len();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), total);
    }
}
