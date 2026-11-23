//! Whether mistral.rs can draw Alcyo's board as llama-server does, on the same Mac.
//!
//!   mistralrs-ios-spike <model dir> <model file.gguf> <prompt file> [grammar.lark]
//!     One prompt, three times, with its speed.
//!   mistralrs-ios-spike <model dir> <model file.gguf> replay <replay.jsonl> <out.jsonl>
//!     Every request recorded from Alcyo's board (made by replay.py: the messages, the board grammar as Lark,
//!     llama-server's answer and time), asked again here; each answer and time goes to out.jsonl.
//!
//! Built for aarch64-apple-ios it shows whether mistral.rs builds for the iPad at all.

use anyhow::{Context, Result};
use mistralrs::{Constraint, GgufModelBuilder, Model, RequestBuilder, TextMessageRole};
use serde_json::{json, Value};
use std::io::{BufRead, Write};
use std::time::Instant;

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [dir, file, rest @ ..] = args.as_slice() else {
        anyhow::bail!("usage: mistralrs-ios-spike <model dir> <model file.gguf> (<prompt file> [grammar.lark] | replay <replay.jsonl> <out.jsonl>)");
    };
    let loading = Instant::now();
    let model = GgufModelBuilder::new(dir, vec![file.clone()]).with_logging().build().await?;
    eprintln!("loaded in {:.1} s", loading.elapsed().as_secs_f32());
    match rest {
        [mode, input, output] if mode == "replay" => replay(&model, input, output).await,
        [prompt, grammar @ ..] => once(&model, prompt, grammar.first()).await,
        _ => anyhow::bail!("a prompt file, or replay <in> <out>"),
    }
}

async fn once(model: &Model, prompt: &str, grammar: Option<&String>) -> Result<()> {
    let prompt = std::fs::read_to_string(prompt).context("reading the prompt")?;
    let grammar = grammar.map(std::fs::read_to_string).transpose().context("reading the grammar")?;
    for round in 1..=3 {
        let mut request = RequestBuilder::new().add_message(TextMessageRole::User, &prompt).set_sampler_max_len(160).set_sampler_temperature(0.0);
        if let Some(grammar) = &grammar {
            request = request.set_constraint(Constraint::Lark(grammar.clone()));
        }
        let asked = Instant::now();
        let response = model.send_chat_request(request).await?;
        let usage = &response.usage;
        println!("--- round {round}: {:.2} s, prompt {} tok at {:.0} tok/s, answer {} tok at {:.1} tok/s", asked.elapsed().as_secs_f32(), usage.prompt_tokens, usage.avg_prompt_tok_per_sec, usage.completion_tokens, usage.avg_compl_tok_per_sec);
        println!("{}", response.choices[0].message.content.as_deref().unwrap_or_default());
    }
    Ok(())
}

async fn replay(model: &Model, input: &str, output: &str) -> Result<()> {
    let lines = std::io::BufReader::new(std::fs::File::open(input).context("reading the replay file")?).lines();
    let mut out = std::fs::File::create(output).context("creating the output")?;
    let (mut seconds, mut same, mut count) = (vec![], 0, 0);
    for (n, line) in lines.enumerate() {
        let item: Value = serde_json::from_str(&line?)?;
        let mut request = RequestBuilder::new().set_sampler_max_len(item["max_tokens"].as_u64().unwrap_or(160) as usize).set_sampler_temperature(0.0);
        for message in item["messages"].as_array().context("messages")? {
            let role = match message["role"].as_str() {
                Some("system") => TextMessageRole::System,
                Some("assistant") => TextMessageRole::Assistant,
                _ => TextMessageRole::User,
            };
            request = request.add_message(role, message["content"].as_str().unwrap_or_default());
        }
        request = request.set_constraint(Constraint::Lark(item["lark"].as_str().context("lark")?.to_string()));
        let asked = Instant::now();
        let answer = match model.send_chat_request(request).await {
            Ok(response) => response.choices[0].message.content.clone().unwrap_or_default(),
            Err(e) => format!("ERROR: {e}"),
        };
        let took = asked.elapsed().as_secs_f64();
        let llama = item["llama_content"].as_str().unwrap_or_default();
        let matches = answer.trim() == llama.trim();
        same += usize::from(matches);
        count += 1;
        seconds.push(took);
        writeln!(out, "{}", json!({ "n": n, "mistralrs": answer, "mistralrs_seconds": took, "llama": llama, "llama_seconds": item["llama_seconds"], "same": matches }))?;
        eprintln!("{n}: {took:.2} s{}", if matches { "" } else { " (differs)" });
    }
    seconds.sort_by(f64::total_cmp);
    let pick = |p: f64| seconds.get(((seconds.len() as f64 * p) as usize).min(seconds.len().saturating_sub(1))).copied().unwrap_or(0.0);
    println!("{count} requests: p50 {:.2} s, p95 {:.2} s, total {:.0} s; same answer as llama-server on {same} ({:.0} %)", pick(0.5), pick(0.95), seconds.iter().sum::<f64>(), 100.0 * same as f64 / count.max(1) as f64);
    Ok(())
}
