//! Links llama.cpp's server, built as static libraries by scripts/build-llama-embedded.sh from our fork of llama.cpp
//! (HsiangNianian/llama.cpp, which adds `llama_server_main` and `llama_server_shutdown`).
//!
//! The libraries come from the fork's release named by ALCYO_LLAMA_RELEASE_URL in .cargo/config.toml, built once
//! there, and an archive whose SHA-256 differs from the one in ALCYO_LLAMA_SHA256SUMS fails the build, as with
//! sherpa-onnx. ALCYO_LLAMA_LIB_DIR names a build of one's own instead (a folder with `lib/*.a`), on any platform.
//!
//! The iPad always links it; a Mac only with the `mac` feature; anything else only with ALCYO_LLAMA_LIB_DIR. Without
//! the libraries the crate builds, and starting the server says it is not built in.

use sha2::{Digest, Sha256};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// In the order a linker that reads each archive once needs them: users before what they use.
const LIBS: [&str; 15] = [
    "llama-server-impl",
    "server-context",
    "llama-ui",
    "cpp-httplib",
    "mtmd",
    "llama-common",
    "llama-common-base",
    "vendor-hash",
    "llama",
    "ggml",
    "ggml-metal",
    "ggml-blas",
    "ggml-cpu",
    "ggml-base",
    "ggml-cpu-base",
];

fn main() {
    println!("cargo::rustc-check-cfg=cfg(llama_missing)");
    for var in ["ALCYO_LLAMA_LIB_DIR", "ALCYO_LLAMA_RELEASE_URL", "ALCYO_LLAMA_SHA256SUMS"] {
        println!("cargo:rerun-if-env-changed={var}");
    }
    let os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    let simulator = env::var("CARGO_CFG_TARGET_ABI").is_ok_and(|abi| abi == "sim");
    let dir = match env::var_os("ALCYO_LLAMA_LIB_DIR") {
        Some(dir) => PathBuf::from(dir),
        None => {
            let platform = match (os.as_str(), arch.as_str()) {
                ("ios", "aarch64") if !simulator => "ios-arm64",
                ("macos", "aarch64") if env::var_os("CARGO_FEATURE_MAC").is_some() => "macos-arm64",
                _ => {
                    println!("cargo:rustc-cfg=llama_missing");
                    return;
                }
            };
            fetch(platform)
        }
    };
    link(&dir.join("lib"), &os);
}

fn link(lib: &Path, os: &str) {
    assert!(lib.join("libllama-server-impl.a").is_file(), "no libllama-server-impl.a in {}", lib.display());
    println!("cargo:rustc-link-search=native={}", lib.display());
    for name in LIBS.iter().filter(|name| lib.join(format!("lib{name}.a")).is_file()) {
        println!("cargo:rustc-link-lib=static={name}");
    }
    if os == "ios" || os == "macos" {
        println!("cargo:rustc-link-lib=c++");
        for framework in ["Foundation", "Metal", "MetalKit", "Accelerate"] {
            println!("cargo:rustc-link-lib=framework={framework}");
        }
        // The Metal backend asks `@available(...)` for systems newer than the one built for, which clang answers
        // from its runtime library; rustc links without it.
        let (sdk, runtime) = if os == "ios" { ("iphoneos", "clang_rt.ios") } else { ("macosx", "clang_rt.osx") };
        let resources = Command::new("xcrun").args(["--sdk", sdk, "clang", "--print-resource-dir"]).output().ok().filter(|out| out.status.success());
        if let Some(darwin) = resources.map(|out| PathBuf::from(String::from_utf8_lossy(&out.stdout).trim()).join("lib/darwin")).filter(|dir| dir.join(format!("lib{runtime}.a")).is_file()) {
            println!("cargo:rustc-link-search=native={}", darwin.display());
            println!("cargo:rustc-link-lib=static={runtime}");
        }
    } else {
        println!("cargo:rustc-link-lib=stdc++");
    }
}

/// The release's archive for `platform`, verified and unpacked into OUT_DIR once.
fn fetch(platform: &str) -> PathBuf {
    let release = env::var("ALCYO_LLAMA_RELEASE_URL").expect("ALCYO_LLAMA_RELEASE_URL is not set: see .cargo/config.toml");
    let sums = env::var("ALCYO_LLAMA_SHA256SUMS").expect("ALCYO_LLAMA_SHA256SUMS is not set: see .cargo/config.toml");
    println!("cargo:rerun-if-changed={sums}");
    let name = format!("llama-server-embedded-{platform}.tar.gz");
    let text = fs::read_to_string(&sums).unwrap_or_else(|e| panic!("reading {sums}: {e}"));
    let wanted = text
        .lines()
        .filter_map(|line| line.split_once("  "))
        .find(|(_, file)| file.trim() == name)
        .map(|(sum, _)| sum.trim().to_lowercase())
        .unwrap_or_else(|| panic!("{sums} lists no {name}"));

    let out = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR")).join(format!("llama-{platform}"));
    let marker = out.join(".sha256");
    if fs::read_to_string(&marker).is_ok_and(|s| s.trim() == wanted) {
        return out;
    }
    let _ = fs::remove_dir_all(&out);
    fs::create_dir_all(&out).expect("creating the libraries' folder");
    let archive = out.with_extension("tar.gz");
    let url = format!("{}/{name}", release.trim_end_matches('/'));
    let status = Command::new("curl").args(["-fsSL", "--retry", "5", "-o"]).arg(&archive).arg(&url).status().expect("running curl");
    assert!(status.success(), "downloading {url} failed");
    let got = format!("{:x}", Sha256::digest(fs::read(&archive).expect("reading the download")));
    assert!(got == wanted, "{name} has SHA-256 {got}, but {sums} says {wanted}: not linking it");
    let status = Command::new("tar").arg("-xzf").arg(&archive).arg("-C").arg(&out).status().expect("running tar");
    assert!(status.success(), "unpacking {name} failed");
    let _ = fs::remove_file(&archive);
    fs::write(&marker, &wanted).expect("writing the marker");
    out
}
