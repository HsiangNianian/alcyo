//! Start the built-in server, ask it once and stop it, three times over, timing each step and saying how much memory
//! the process holds after each stop, so memory not given back would show as growth:
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
    println!("before: {}", memory());
    for round in 1..=3 {
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
            // A reasoning model may spend so short a reply on its thinking.
            let message = &reply["choices"][0]["message"];
            let text = message["content"].as_str().filter(|t| !t.is_empty()).or_else(|| message["reasoning_content"].as_str()).unwrap_or_default();
            anyhow::ensure!(!text.is_empty(), "no reply in {reply}");
            println!("  reply in {:.2} s ({}): {text:?}", asked.elapsed().as_secs_f64(), reply["timings"]);
        }

        let stopping = Instant::now();
        let code = server.stop()?;
        println!("  stopped in {:.2} s, exit {code}; {}", stopping.elapsed().as_secs_f64(), memory());
        anyhow::ensure!(ureq::get(&health).timeout(Duration::from_secs(1)).call().is_err(), "still answering after it stopped");
    }

    // One slot, as the app runs it, so the second request below waits for the first.
    let mut server_args = vec!["-m".to_string(), model, "--host".into(), "127.0.0.1".into(), "--port".into(), PORT.to_string(), "-np".into(), "1".into()];
    if kind == "embed" {
        server_args.push("--embeddings".into());
    }
    server_args.extend(extra);

    // Stopped while it writes a long reply, with another request waiting behind it: the stop must not wait for the
    // reply, nor for the client to give up on it (it once waited until the client's own time limit, minutes).
    if kind == "chat" {
        let up = |server: &mut alcyo_llama::Server| -> Result<()> {
            let health = format!("http://127.0.0.1:{PORT}/health");
            let started = Instant::now();
            while ureq::get(&health).timeout(Duration::from_secs(2)).call().is_err() {
                if let Some(code) = server.exited() {
                    bail!("the server exited with {code} while starting");
                }
                anyhow::ensure!(started.elapsed() < Duration::from_secs(600), "not up after 10 minutes");
                std::thread::sleep(Duration::from_millis(100));
            }
            Ok(())
        };
        let ask = |tokens: u32| {
            std::thread::spawn(move || {
                let body = json!({ "messages": [{ "role": "user", "content": "Count from one to ten thousand in words." }], "max_tokens": tokens, "temperature": 0, "ignore_eos": true });
                let asked = Instant::now();
                let reply = ureq::post(&format!("http://127.0.0.1:{PORT}/v1/chat/completions")).timeout(Duration::from_secs(300)).send_json(body);
                (asked.elapsed(), reply.map(|_| ()).map_err(|e| e.to_string()))
            })
        };
        let mut server = alcyo_llama::Server::start(&server_args)?;
        up(&mut server)?;
        let (long, queued) = (ask(100_000), ask(16));
        std::thread::sleep(Duration::from_secs(3));
        let stopping = Instant::now();
        let code = server.stop()?;
        let took = stopping.elapsed();
        println!("stopped while answering in {:.2} s, exit {code}", took.as_secs_f64());
        for (name, client) in [("the long request", long), ("the waiting request", queued)] {
            let (after, reply) = client.join().expect("client thread");
            println!("  {name} ended after {:.2} s: {}", after.as_secs_f64(), reply.err().unwrap_or_else(|| "answered".into()));
            anyhow::ensure!(after < Duration::from_secs(60), "{name} was left waiting");
        }
        anyhow::ensure!(took < Duration::from_secs(10), "the stop waited {:.1} s for a reply being written", took.as_secs_f64());

        // A stop given up on at once leaves the server to end by itself; the next start waits for it, then runs.
        let mut server = alcyo_llama::Server::start(&server_args)?;
        up(&mut server)?;
        let long = ask(100_000);
        std::thread::sleep(Duration::from_secs(2));
        let given_up = server.stop_within(Duration::ZERO).expect_err("a stop given no time is given up on");
        anyhow::ensure!(given_up.downcast_ref::<alcyo_llama::StopTimedOut>().is_some(), "not a timed-out stop: {given_up:#}");
        let restarting = Instant::now();
        let mut server = alcyo_llama::Server::start(&server_args)?;
        up(&mut server)?;
        println!("started again after a stop given up on, in {:.2} s", restarting.elapsed().as_secs_f64());
        let _ = long.join();
        server.stop()?;
    }

    // Stopped while it loads: it gives up the load rather than finishing it first.
    let server = alcyo_llama::Server::start(&server_args)?;
    std::thread::sleep(Duration::from_millis(200));
    let stopping = Instant::now();
    let code = server.stop()?;
    println!("stopped while loading in {:.2} s, exit {code}", stopping.elapsed().as_secs_f64());
    println!("ok");
    Ok(())
}

/// What this process holds: resident and, on a Mac, its footprint (what iPadOS counts against an app), in MB.
fn memory() -> String {
    #[cfg(target_os = "macos")]
    {
        extern "C" {
            fn proc_pid_rusage(pid: i32, flavor: i32, buffer: *mut u64) -> i32;
        }
        // rusage_info_v0: a 16-byte UUID, then user and system time, idle and interrupt wakeups, page-ins, wired,
        // resident size and the footprint, each 64 bits.
        let mut info = [0_u64; 16];
        // SAFETY: the buffer is larger than rusage_info_v0, the flavor asked for.
        if unsafe { proc_pid_rusage(std::process::id() as i32, 0, info.as_mut_ptr()) } == 0 {
            return format!("resident {} MB, footprint {} MB", info[8] >> 20, info[9] >> 20);
        }
    }
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    status.lines().find(|l| l.starts_with("VmRSS")).map_or_else(|| "memory unknown".into(), |l| l.split_whitespace().collect::<Vec<_>>().join(" "))
}
