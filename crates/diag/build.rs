//! The commit this copy was built from, for a diagnostic bundle's manifest: CI's `GITHUB_SHA`, or what git says of
//! the checkout, or "unknown" in a source archive.

use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok().filter(|o| o.status.success())?;
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string()).filter(|s| !s.is_empty())
}

fn main() {
    println!("cargo:rerun-if-env-changed=GITHUB_SHA");
    // Built again when a commit is made or checked out.
    for path in ["HEAD", "logs/HEAD"] {
        if let Some(file) = git(&["rev-parse", "--git-path", path]) {
            println!("cargo:rerun-if-changed={file}");
        }
    }
    let commit = std::env::var("GITHUB_SHA").ok().map(|sha| sha.chars().take(12).collect()).or_else(|| git(&["rev-parse", "--short=12", "HEAD"])).unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=ALCYO_GIT_COMMIT={commit}");
}
