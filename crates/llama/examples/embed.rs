//! Load an embedding model in this process, embed a few passages, and say how much memory each step took and how
//! near the passages are to the first; given a llama-server's address serving the same model with `--embeddings`,
//! also how near each vector is to the server's:
//!
//!   ALCYO_LLAMA_LIB_DIR=<build> cargo run --release -p alcyo-llama --example embed -- <model.gguf> [http://127.0.0.1:<port>]
//!
//! It checks on a computer what meeting search does on the iPad (crates/llama/src/embed.rs).

use anyhow::{Context, Result};
use serde_json::json;
use std::time::Instant;

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let model = args.next().context("usage: embed <model.gguf> [server url]")?;
    let server = args.next();
    let texts: Vec<String> = [
        "task: search result | query: when are we shipping now?",
        "title: Release review | text: We moved the ship date from Friday to Tuesday.",
        "title: Release review | text: Load testing has to finish before launch.",
        "title: Lunch | text: The canteen serves noodles on Wednesdays.",
        "title: 物业例会 | text: 地下车库的车位按先来后到分配。",
        "title: 发布评审 | text: 我们把发布日期从周五改到了周二。",
    ]
    .map(String::from)
    .into_iter()
    // Several hundred tokens, more than a sixteenth of the context.
    .chain(std::iter::once(format!("title: Planning | text: {}", "We went through the budget and the schedule once more. ".repeat(50))))
    .collect();

    println!("file: {} MiB", std::fs::metadata(&model)?.len() >> 20);
    println!("before: {}", memory());
    let started = Instant::now();
    let mut embedder = alcyo_llama::Embedder::load(model.as_ref())?;
    println!("loaded in {:.2} s: {}", started.elapsed().as_secs_f64(), memory());
    let started = Instant::now();
    let vectors = embedder.embed(&texts)?;
    println!("{} passages in {:.3} s: {}", texts.len(), started.elapsed().as_secs_f64(), memory());
    let many: Vec<String> = (0..64).map(|i| format!("title: Planning | text: Item {i} of the plan: the team reviewed the budget, the schedule and the risks for part {i} in detail.")).collect();
    let started = Instant::now();
    embedder.embed(&many)?;
    println!("{} passages in {:.3} s: {}", many.len(), started.elapsed().as_secs_f64(), memory());
    let long = vec!["the budget and the schedule ".repeat(400); 4];
    let started = Instant::now();
    embedder.embed(&long)?;
    println!("{} passages cut to the context in {:.3} s: {}", long.len(), started.elapsed().as_secs_f64(), memory());

    println!("dims {}; cosine to the query:", embedder.dims());
    for (text, v) in texts.iter().zip(&vectors).skip(1) {
        println!("  {:.3}  {text}", cosine(&vectors[0], v));
    }
    if let Some(base) = server {
        let reply: serde_json::Value = ureq::post(&format!("{}/v1/embeddings", base.trim_end_matches('/'))).send_json(json!({ "input": texts }))?.into_json()?;
        println!("cosine to llama-server's vector of the same text:");
        for (i, v) in vectors.iter().enumerate() {
            let theirs: Vec<f32> = reply["data"][i]["embedding"].as_array().context("no embedding in the reply")?.iter().map(|x| x.as_f64().unwrap_or(0.0) as f32).collect();
            let cut = |v: &[f32], n: usize| v[..n.min(v.len())].to_vec();
            println!("  full {:.6}, first 512 {:.6}", cosine(v, &theirs), cosine(&cut(v, 512), &cut(&theirs, 512)));
        }
    }
    drop(embedder);
    println!("dropped: {}", memory());
    // Meeting search drops the model after a few idle minutes and loads it again when asked: what it holds
    // afterwards should not grow round by round.
    for round in 2..=4 {
        let mut embedder = alcyo_llama::Embedder::load(model.as_ref())?;
        embedder.embed(&many)?;
        drop(embedder);
        println!("round {round}, dropped: {}", memory());
    }
    Ok(())
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let norm = |v: &[f32]| v.iter().map(|x| x * x).sum::<f32>().sqrt();
    dot / (norm(a) * norm(b))
}

/// What the process holds now and at most so far, where the system says (Linux).
fn memory() -> String {
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    let field = |name: &str| status.lines().find(|l| l.starts_with(name)).map(|l| l[name.len()..].trim().to_string()).unwrap_or_else(|| "?".into());
    format!("rss {}, peak {}, anonymous {}, file {}", field("VmRSS:"), field("VmHWM:"), field("RssAnon:"), field("RssFile:"))
}
