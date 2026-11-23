//! Whether mistral.rs can draw Alcyo's board: load a GGUF model, ask it one prompt, optionally held to a Lark
//! grammar, and say what it answered and how fast. For comparing with llama-server on the same Mac, and, built
//! for aarch64-apple-ios, whether it builds for the iPad at all.
//!
//!   mistralrs-ios-spike <model dir> <model file.gguf> <prompt file> [grammar.lark]

use anyhow::{Context, Result};
use mistralrs::{Constraint, GgufModelBuilder, RequestBuilder, TextMessageRole};
use std::time::Instant;

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [dir, file, prompt, rest @ ..] = args.as_slice() else {
        anyhow::bail!("usage: mistralrs-ios-spike <model dir> <model file.gguf> <prompt file> [grammar.lark]");
    };
    let prompt = std::fs::read_to_string(prompt).context("reading the prompt")?;
    let grammar = rest.first().map(std::fs::read_to_string).transpose().context("reading the grammar")?;

    let loading = Instant::now();
    let model = GgufModelBuilder::new(dir, vec![file.clone()]).with_logging().build().await?;
    eprintln!("loaded in {:.1} s", loading.elapsed().as_secs_f32());

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
