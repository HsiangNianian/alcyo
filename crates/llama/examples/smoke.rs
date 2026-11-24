//! Start the built-in server, ask it once, stop it, and do it all again, timing each step:
//!
//!   cargo run --release -p alcyo-llama --features mac --example smoke -- <model.gguf> [chat|embed] [server args…]
//!
//! It proves on a Mac what the iPad does (crates/llama/src/lib.rs). `embed` asks for an embedding, for a small
//! embedding model; `chat` (the default) for a short reply.

use anyhow::{bail, Context, Result};
use serde_json::json;
use std::time::{Duration, Instant};

const PORT: u16 = 18_437;

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let model = args.next().context("usage: smoke <model.gguf> [chat|embed] [server args…]")?;
    let kind = args.next().unwrap_or_else(|| "chat".into());
    let extra: Vec<String> = args.collect();
    println!("memory: physical {:?} bytes, available {:?} bytes", alcyo_llama::physical_memory(), alcyo_llama::available_memory());
    for round in 1..=2 {
        println!("round {round}");
        let mut server_args = vec!["-m".to_string(), model.clone(), "--host".into(), "127.0.0.1".into(), "--port".into(), PORT.to_string(), "-np".into(), "1".into()];
        if kind == "embed" {
            server_args.push("--embeddings".into());
        }
        server_args.extend(extra.iter().cloned());
        let started = Instant::now();
        let mut server = alcyo_llama::Server::start(&server_args)?;
        let health = format!("http://127.0.0.1:{PORT}/health");
        loop {
            if let Some(code) = server.exited() {
                bail!("the server exited with {code} while starting");
            }
            if ureq::get(&health).timeout(Duration::from_secs(2)).call().is_ok() {
                break;
            }
            if started.elapsed() > Duration::from_secs(600) {
                bail!("not up after 10 minutes");
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        println!("  up (model loaded) in {:.2} s", started.elapsed().as_secs_f64());

        let asked = Instant::now();
        if kind == "embed" {
            let reply: serde_json::Value = ureq::post(&format!("http://127.0.0.1:{PORT}/v1/embeddings")).send_json(json!({ "input": "a board drawn from speech" }))?.into_json()?;
            let size = reply["data"][0]["embedding"].as_array().map_or(0, Vec::len);
            anyhow::ensure!(size > 0, "no embedding in {reply}");
            println!("  embedding of {size} numbers in {:.2} s", asked.elapsed().as_secs_f64());
        } else {
            let body = json!({ "messages": [{ "role": "user", "content": "Say hello in five words." }], "max_tokens": 24, "temperature": 0 });
            let reply: serde_json::Value = ureq::post(&format!("http://127.0.0.1:{PORT}/v1/chat/completions")).send_json(body)?.into_json()?;
            let text = reply["choices"][0]["message"]["content"].as_str().unwrap_or_default();
            anyhow::ensure!(!text.is_empty(), "no reply in {reply}");
            println!("  reply in {:.2} s ({}): {text:?}", asked.elapsed().as_secs_f64(), reply["timings"]);
        }

        let stopping = Instant::now();
        let code = server.stop()?;
        println!("  stopped in {:.2} s, exit {code}", stopping.elapsed().as_secs_f64());
        anyhow::ensure!(ureq::get(&health).timeout(Duration::from_secs(1)).call().is_err(), "still answering after it stopped");
    }

    // Stopped while it loads: it gives up the load rather than finishing it first.
    let mut server_args = vec!["-m".to_string(), model, "--host".into(), "127.0.0.1".into(), "--port".into(), PORT.to_string()];
    if kind == "embed" {
        server_args.push("--embeddings".into());
    }
    server_args.extend(extra);
    let server = alcyo_llama::Server::start(&server_args)?;
    std::thread::sleep(Duration::from_millis(200));
    let stopping = Instant::now();
    let code = server.stop()?;
    println!("stopped while loading in {:.2} s, exit {code}", stopping.elapsed().as_secs_f64());
    println!("ok");
    Ok(())
}
