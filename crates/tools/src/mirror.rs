//! `models list [--json]`: every file the app and CI download, as the models-mirror workflow copies them to
//! Alcyo's R2 bucket (models.alcyo.app): its key there, size, SHA-256 and where upstream serves it.

use crate::voice_files;
use anyhow::{bail, Result};
use serde_json::json;

pub struct Entry {
    pub key: String,
    pub size: u64,
    pub sha256: String,
    /// Upstream only, the fastest from a GitHub runner first: Hugging Face and GitHub, then the mirrors in China.
    pub upstream: Vec<String>,
}

fn upstream(sources: &[alcyo_models::Source]) -> Vec<String> {
    let rank = |name: &str| match name {
        "Hugging Face" | "GitHub" => 0,
        "ModelScope" => 1,
        _ => 2,
    };
    let mut theirs: Vec<&alcyo_models::Source> = sources.iter().filter(|s| !s.url.starts_with(alcyo_models::MIRROR)).collect();
    theirs.sort_by_key(|s| rank(&s.name));
    theirs.into_iter().map(|s| s.url.clone()).collect()
}

fn mirror_key(model: &alcyo_models::Model) -> String {
    let mirror = model.sources.iter().find(|s| s.url.starts_with(alcyo_models::MIRROR)).expect("every model is on the mirror");
    mirror.url[alcyo_models::MIRROR.len() + 1..].to_string()
}

pub fn entries() -> Vec<Entry> {
    let models = alcyo_models::catalog().into_iter().flat_map(|m| [Some(m.clone()), alcyo_models::vision_for(&m.file)]).flatten().chain([alcyo_models::embedding()]);
    let mut all: Vec<Entry> = models.map(|m| Entry { key: mirror_key(&m), size: m.size, sha256: m.sha256.clone(), upstream: upstream(&m.sources) }).collect();
    all.extend(voice_files::files().iter().map(|f| Entry { key: f.key(), size: f.size, sha256: f.sha256.to_string(), upstream: upstream(&f.sources()) }));
    all
}

pub fn run(mut args: impl Iterator<Item = String>) -> Result<()> {
    let (Some("list"), flag) = (args.next().as_deref(), args.next()) else { bail!("usage: models list [--json]") };
    let all = entries();
    match flag.as_deref() {
        Some("--json") => {
            let list: Vec<_> = all.iter().map(|e| json!({ "key": e.key, "size": e.size, "sha256": e.sha256, "upstream": e.upstream })).collect();
            println!("{}", serde_json::to_string_pretty(&list)?);
        }
        None => {
            for e in &all {
                println!("{:>14}  {}", e.size, e.key);
            }
            println!("{:>14}  {} files", all.iter().map(|e| e.size).sum::<u64>(), all.len());
        }
        Some(other) => bail!("unknown flag {other}"),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_file_has_its_own_key_and_somewhere_upstream() {
        let all = entries();
        let mut keys: Vec<&str> = all.iter().map(|e| e.key.as_str()).collect();
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), all.len());
        // Two board models, the vision add-on, the search model and every voice file.
        assert_eq!(all.len(), 4 + voice_files::files().len());
        for e in &all {
            assert!(!e.upstream.is_empty() && e.upstream.iter().all(|u| !u.starts_with(alcyo_models::MIRROR)), "{}", e.key);
            assert!(e.key.contains(&e.sha256[..16]), "{}", e.key);
            assert!(!e.key.contains("9B"), "the 9B is gone: {}", e.key);
        }
        assert!(all.iter().any(|e| e.key == "voice/47c221ea9b4d4e7f/speaker/nemotron-3-diarization.int8.onnx" && e.upstream[0].starts_with("https://github.com/")));
    }
}
