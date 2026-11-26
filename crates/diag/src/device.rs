//! What a bundle says about where it was made: the app's version and build, the system and the device, its memory
//! and free space, and the model files downloaded, each with its SHA-256 (worked out once and kept, as a board
//! model is gigabytes).

use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Path, PathBuf};

/// The commit this copy was built from.
pub const COMMIT: &str = env!("ALCYO_GIT_COMMIT");

/// Our llama.cpp fork's release the app was built against, by its tag (`alcyo-b11954-3`), from the address
/// `.cargo/config.toml` gives the build.
pub fn llama_fork() -> &'static str {
    option_env!("ALCYO_LLAMA_RELEASE_URL").and_then(|url| url.trim_end_matches('/').rsplit('/').next()).unwrap_or("unknown")
}

/// The build number a package was given (`CFBundleVersion` in its Info.plist, beside the program on the iPad and in
/// `Contents` on a Mac). `None` for a copy that is not a package.
pub fn build_number() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    let plist = [dir.join("Info.plist"), dir.parent()?.join("Info.plist")].into_iter().find(|p| p.is_file())?;
    plist_value(&std::fs::read_to_string(plist).ok()?, "CFBundleVersion")
}

/// A string's value in an XML property list, as the packaging scripts write them.
fn plist_value(text: &str, key: &str) -> Option<String> {
    let after = &text[text.find(&format!("<key>{key}</key>"))?..];
    let start = after.find("<string>")? + "<string>".len();
    let end = after[start..].find("</string>")?;
    Some(after[start..start + end].trim().to_string()).filter(|v| !v.is_empty())
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
fn sysctl_string(name: &std::ffi::CStr) -> Option<String> {
    let mut len: libc::size_t = 0;
    // SAFETY: the first call asks only for the length; the second fills a buffer of that length.
    unsafe {
        if libc::sysctlbyname(name.as_ptr(), std::ptr::null_mut(), &mut len, std::ptr::null_mut(), 0) != 0 || len == 0 {
            return None;
        }
        let mut buf = vec![0u8; len];
        if libc::sysctlbyname(name.as_ptr(), buf.as_mut_ptr().cast(), &mut len, std::ptr::null_mut(), 0) != 0 {
            return None;
        }
        buf.truncate(len);
        Some(String::from_utf8_lossy(&buf).trim_end_matches('\0').to_string())
    }
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
fn sysctl_u64(name: &std::ffi::CStr) -> Option<u64> {
    let mut value: u64 = 0;
    let mut len = std::mem::size_of::<u64>();
    // SAFETY: a u64 of the length given.
    let ok = unsafe { libc::sysctlbyname(name.as_ptr(), (&raw mut value).cast(), &mut len, std::ptr::null_mut(), 0) } == 0;
    ok.then_some(value)
}

/// The system's version, as people say it: "17.6.1", "Arch Linux, kernel 6.17.1".
pub fn os_version() -> String {
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    {
        sysctl_string(c"kern.osproductversion").unwrap_or_default()
    }
    #[cfg(target_os = "linux")]
    {
        let pretty = std::fs::read_to_string("/etc/os-release").ok().and_then(|t| t.lines().find_map(|l| l.strip_prefix("PRETTY_NAME=").map(|v| v.trim_matches('"').to_string())));
        let kernel = std::fs::read_to_string("/proc/sys/kernel/osrelease").ok().map(|k| k.trim().to_string());
        [pretty, kernel].into_iter().flatten().collect::<Vec<_>>().join(", kernel ")
    }
    #[cfg(not(any(target_os = "macos", target_os = "ios", target_os = "linux")))]
    {
        String::new()
    }
}

/// The device: "iPad14,5", "MacBookPro18,3", or what a PC says of itself.
pub fn model() -> Option<String> {
    #[cfg(target_os = "ios")]
    {
        sysctl_string(c"hw.machine")
    }
    #[cfg(target_os = "macos")]
    {
        sysctl_string(c"hw.model")
    }
    #[cfg(target_os = "linux")]
    {
        ["/sys/devices/virtual/dmi/id/product_name", "/proc/device-tree/model"].iter().find_map(|p| std::fs::read_to_string(p).ok()).map(|s| s.trim_end_matches('\0').trim().to_string()).filter(|s| !s.is_empty())
    }
    #[cfg(not(any(target_os = "macos", target_os = "ios", target_os = "linux")))]
    {
        None
    }
}

/// How much memory the device has, in bytes.
pub fn memory() -> Option<u64> {
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    {
        sysctl_u64(c"hw.memsize")
    }
    #[cfg(target_os = "linux")]
    {
        std::fs::read_to_string("/proc/meminfo").ok()?.lines().find_map(|l| l.strip_prefix("MemTotal:")).and_then(|v| v.trim().trim_end_matches("kB").trim().parse::<u64>().ok()).map(|kb| kb * 1024)
    }
    #[cfg(not(any(target_os = "macos", target_os = "ios", target_os = "linux")))]
    {
        None
    }
}

/// How much space is free where `path` is, in bytes.
pub fn free_space(path: &Path) -> Option<u64> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let c = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
        // SAFETY: a zeroed statvfs is valid for the call to fill.
        let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
        if unsafe { libc::statvfs(c.as_ptr(), &mut stat) } != 0 {
            return None;
        }
        #[allow(clippy::unnecessary_cast)]
        Some(stat.f_bavail as u64 * stat.f_frsize as u64)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        None
    }
}

/// Everything about this copy and the device it runs on, for a manifest. `data` is where its data is kept.
pub fn describe(data: &Path) -> Value {
    json!({
        "app": {
            "version": env!("CARGO_PKG_VERSION"),
            "build": build_number(),
            "commit": COMMIT,
            "llama_fork": llama_fork(),
        },
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "os_version": os_version(),
        "device": model(),
        "memory_bytes": memory(),
        "available_memory_bytes": crate::available_memory(),
        "free_storage_bytes": free_space(data),
    })
}

/// A downloaded model file, as a manifest lists it.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelFile {
    pub name: String,
    pub bytes: u64,
    pub sha256: String,
}

impl ModelFile {
    pub fn json(&self) -> Value {
        json!({ "name": self.name, "bytes": self.bytes, "sha256": self.sha256 })
    }
}

fn sha256(path: &Path) -> std::io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// The model files in `dir` (`.gguf`, and what else is not a download left unfinished), each with its SHA-256. A
/// file's hash is kept in `cache` by its name, size and time of change, and worked out again only when one of them
/// changes. Slow the first time: call it off the window's thread.
pub fn model_files(dir: &Path, cache: &Path) -> Vec<ModelFile> {
    let mut known: Map<String, Value> = std::fs::read_to_string(cache).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir).map(|e| e.flatten().map(|e| e.path()).filter(|p| p.is_file() && p.extension().is_some_and(|x| x == "gguf" || x == "onnx" || x == "bin")).collect()).unwrap_or_default();
    files.sort();
    let mut changed = false;
    let mut out = vec![];
    for path in files {
        let Ok(meta) = path.metadata() else { continue };
        let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
        let modified = meta.modified().ok().and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs());
        let stamp = format!("{}:{modified}", meta.len());
        let hash = match known.get(&name) {
            Some(entry) if entry["stamp"] == stamp.as_str() => entry["sha256"].as_str().unwrap_or_default().to_string(),
            _ => {
                let Ok(hash) = sha256(&path) else { continue };
                known.insert(name.clone(), json!({ "stamp": stamp, "sha256": hash }));
                changed = true;
                hash
            }
        };
        out.push(ModelFile { name, bytes: meta.len(), sha256: hash });
    }
    if changed {
        if let Some(parent) = cache.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(cache, Value::Object(known).to_string());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_build_number_is_read_from_the_plist() {
        let plist = "<dict>\n\t<key>CFBundleShortVersionString</key>\n\t<string>0.34.1</string>\n\t<key>CFBundleVersion</key>\n\t<string>412</string>\n</dict>";
        assert_eq!(plist_value(plist, "CFBundleVersion").as_deref(), Some("412"));
        assert_eq!(plist_value(plist, "CFBundleName"), None);
    }

    #[test]
    fn the_fork_is_named_by_its_tag() {
        assert!(!llama_fork().contains('/'));
    }

    /// Hashes are right, kept, and worked out again only for a file that changed.
    #[test]
    fn model_hashes_are_kept_until_a_file_changes() {
        let dir = std::env::temp_dir().join(format!("alcyo-diag-models-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.gguf"), b"abc").unwrap();
        std::fs::write(dir.join("b.gguf.part"), b"unfinished").unwrap();
        std::fs::write(dir.join("selected.txt"), b"a.gguf").unwrap();
        let cache = dir.join("logs/model-hashes.json");
        let found = model_files(&dir, &cache);
        assert_eq!(found, vec![ModelFile { name: "a.gguf".into(), bytes: 3, sha256: "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad".into() }]);
        // A kept hash is believed while the file's size and time stay: proof that it was not read again.
        let mut kept: Value = serde_json::from_str(&std::fs::read_to_string(&cache).unwrap()).unwrap();
        kept["a.gguf"]["sha256"] = json!("kept");
        std::fs::write(&cache, kept.to_string()).unwrap();
        assert_eq!(model_files(&dir, &cache)[0].sha256, "kept");
        std::fs::write(dir.join("a.gguf"), b"abcd").unwrap();
        assert_eq!(model_files(&dir, &cache)[0].sha256, "88d4266fd4e6338d13b845fcf289579d209c897823b9217da3e161936f031589");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
