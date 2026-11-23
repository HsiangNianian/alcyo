//! On the iPad, Apple's speech recognition is built into the app: apple/speech.swift as a static library,
//! the same code that is the `alcyo-speech` program on a Mac (see src/system.rs). Nothing to do elsewhere.

use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=apple/speech.swift");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("ios") {
        return;
    }
    let simulator = env::var("TARGET").is_ok_and(|t| t.ends_with("-sim"));
    let sdk = if simulator { "iphonesimulator" } else { "iphoneos" };
    let min_os = env::var("IPHONEOS_DEPLOYMENT_TARGET").unwrap_or_else(|_| "17.0".into());
    let arch = match env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("aarch64") => "arm64",
        Ok(other) => panic!("no iOS speech library for {other}"),
        Err(_) => panic!("no target architecture"),
    };
    let target = if simulator { format!("{arch}-apple-ios{min_os}-simulator") } else { format!("{arch}-apple-ios{min_os}") };
    let sdk_path = output(Command::new("xcrun").args(["--sdk", sdk, "--show-sdk-path"]));
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let library = out.join("libalcyo_speech.a");
    let status = Command::new("xcrun")
        .args(["swiftc", "-O", "-parse-as-library", "-swift-version", "5", "-D", "ALCYO_SPEECH_LIBRARY", "-emit-library", "-static", "-module-name", "AlcyoSpeech"])
        .args(["-target", &target, "-sdk", &sdk_path])
        .arg("apple/speech.swift")
        .arg("-o")
        .arg(&library)
        .status()
        .expect("running swiftc");
    assert!(status.success(), "swiftc could not build the speech library");
    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-lib=static=alcyo_speech");
    // The Swift libraries the objects ask for by name: the SDK's, and the toolchain's back-deployment ones.
    println!("cargo:rustc-link-search=native={sdk_path}/usr/lib/swift");
    let swiftc = output(Command::new("xcrun").args(["--sdk", sdk, "-f", "swiftc"]));
    if let Some(toolchain) = PathBuf::from(swiftc).parent().and_then(|bin| bin.parent()) {
        println!("cargo:rustc-link-search=native={}", toolchain.join("lib/swift").join(sdk).display());
    }
    for framework in ["Speech", "AVFoundation", "Foundation"] {
        println!("cargo:rustc-link-lib=framework={framework}");
    }
}

fn output(command: &mut Command) -> String {
    let out = command.output().expect("running xcrun");
    assert!(out.status.success(), "xcrun failed: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}
