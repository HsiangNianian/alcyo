//! Read a diagnostic bundle the app exported (#360): `diag <bundle.zip> [--stall <seconds>] [--render <root> <out-dir>]`
//! prints the model's lifecycle, what stalled, the errors and how many speakers there were over time; `--render`
//! draws each board after each of its steps, with a checkout's handwriting, as `page-N-step-NNNN.png`.

use alcyo_board::glyphs::Glyphs;
use alcyo_board::steps::apply_step;
use alcyo_board::graph::Graph;
use alcyo_diag::read::{report, summarize, Bundle};
use alcyo_export::ink::{compose_board, png, Sources};
use anyhow::{bail, Context, Result};
use std::path::PathBuf;

const USAGE: &str = "usage: diag <bundle.zip> [--stall <seconds>] [--render <root> <out-dir>]";

pub fn run(mut args: impl Iterator<Item = String>) -> Result<()> {
    let zip = PathBuf::from(args.next().context(USAGE)?);
    let mut stall_s = 10.0_f64;
    let mut render: Option<(PathBuf, PathBuf)> = None;
    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--stall" => stall_s = args.next().context(USAGE)?.parse().context("--stall takes seconds")?,
            "--render" => render = Some((PathBuf::from(args.next().context(USAGE)?), PathBuf::from(args.next().context(USAGE)?))),
            other => bail!("unknown flag {other}; {USAGE}"),
        }
    }
    let bundle = Bundle::open(&zip)?;
    let stall_ms = (stall_s * 1000.0) as u64;
    print!("{}", report(&bundle, &summarize(&bundle, stall_ms), stall_ms));
    if let Some((root, out)) = render {
        let mut glyphs = Glyphs::from_checkout(&root)?;
        std::fs::create_dir_all(&out)?;
        let mut written = 0;
        for (page, steps) in bundle.boards.iter().enumerate() {
            let mut board = Graph::default();
            for (i, step) in steps.iter().enumerate() {
                board = apply_step(&board, step);
                let drawn = compose_board(&board, &serde_json::Value::Null, None, &Sources::new(), &mut glyphs).draw();
                if drawn.strokes.is_empty() {
                    continue;
                }
                std::fs::write(out.join(format!("page-{}-step-{i:04}.png", page + 1)), png(&drawn)?)?;
                written += 1;
            }
        }
        println!("\n{written} pictures in {}", out.display());
    }
    Ok(())
}
