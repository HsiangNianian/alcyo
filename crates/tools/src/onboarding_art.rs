//! The boards in the iPad's first run (crates/ui/art), drawn by the board's own layout and pen, so every mark in
//! them (the star, the crossing out, a circle drawn with the Pencil) is the one the app draws:
//!
//!   onboarding-art <root> <out-dir>
//!
//! Writes `board-light.svg` and `board-dark.svg`: the board on no background, its ink in the light and in the dark
//! palette, with the Pencil's contact dot where the circle around the auth service ends. scripts/onboarding-art.sh
//! lays them on their grounds.

use anyhow::{Context, Result};
use alcyo_board::board_layout::StableLayout;
use alcyo_board::glyphs::Glyphs;
use alcyo_board::graph::{parse_ops, Annotation, Graph};
use alcyo_board::sugiyama::first_diagram;
use alcyo_export::ink::{compose_board, svg, Sources};
use serde_json::Value;
use std::path::PathBuf;

/// What a meeting about a login flow would put on its board, line by line as the model writes it.
const OPS: &str = "E 「User」 -> 「API gateway」 : 「HTTPS」
E 「API gateway」 -> 「Auth service」 : 「rate limit」
E 「Auth service」 -> 「Redis」 : 「refresh」
E 「Auth service」 -> 「MySQL users」 : 「look up account」
I 「User」 人
I 「Redis」 数据库
K 「Redis」 「TTL 7 days」
T 「Decisions」
L 「Decisions」 「Access token expires in 15 min」
L 「Decisions」 「Refresh token in Redis, 7 days」
L 「Decisions」 「Rate limiting at the gateway」
L 「Decisions」 「SMS code login」
M 「Access token expires in 15 min」 星标
M 「SMS code login」 划掉
K 「SMS code login」 「not this release」
Q 「Hook into company SSO?」
T 「To-do」
L 「To-do」 「Leo · rate-limit rules」
L 「To-do」 「Nina · size the Redis cluster」
L 「To-do」 「Sam · login sequence chart」
K 「Auth service」 「review separately next week」";

/// The light palette's inks and the dark palette's, as crates/ui/src/theme.rs has them.
const LIGHT_INKS: [u32; 3] = [0x1e_1e_1e, 0xe0_31_31, 0x19_71_c2];
const DARK_INKS: [u32; 3] = [0xec_ec_e6, 0xff_5a_36, 0x9f_b4_ff];

pub fn run(mut args: impl Iterator<Item = String>) -> Result<()> {
    let root = PathBuf::from(args.next().context("usage: onboarding-art <root> <out-dir>")?);
    let out = PathBuf::from(args.next().context("usage: onboarding-art <root> <out-dir>")?);
    std::fs::create_dir_all(&out)?;
    let mut glyphs = Glyphs::from_checkout(&root)?;
    let mut graph = Graph::default();
    for line in OPS.lines() {
        graph.apply(&parse_ops(line));
    }
    // A circle drawn with the red pen around the auth service, as a hand does: a loose ellipse that overshoots its
    // start a little.
    let laid = StableLayout::default().layout(&graph, &mut first_diagram);
    let auth = graph.nodes.iter().find(|n| n.label == "Auth service").context("no auth service on the board")?;
    let box_ = laid.targets().into_iter().find(|t| t.id == auth.id).context("the auth service was not laid out")?;
    let (cx, cy) = (box_.x + box_.w / 2.0, box_.y + box_.h / 2.0);
    let (rx, ry) = (box_.w / 2.0 + 26.0, box_.h / 2.0 + 22.0);
    let points: Vec<[f64; 2]> = (0..=72)
        .map(|i| {
            let k = f64::from(i) / 72.0;
            let a = -2.4 + k * std::f64::consts::TAU * 1.08;
            let wobble = 1.0 + 0.035 * (k * 9.0).sin() + 0.02 * (k * 23.0).cos();
            [cx + rx * wobble * a.cos(), cy + ry * wobble * a.sin() - 6.0 * k]
        })
        .collect();
    let end = *points.last().expect("points");
    graph.annotate(Annotation { tool: "pen".into(), on: None, points, anchor: None });
    let drawn = compose_board(&graph, &Value::Null, None, &Sources::new(), &mut glyphs).draw();
    let light = svg(&drawn, "Alcyo");
    // The paper is the ground's to give: the board alone, on nothing.
    let bare = strip_paper(&light);
    // The Pencil touching the paper where the circle ends: the app's own contact dot, black with a white ring.
    let dot = |ink: &str, ring: &str| format!(r#"<circle cx="{:.1}" cy="{:.1}" r="7" fill="{ink}" stroke="{ring}" stroke-width="2.5"/></svg>"#, end[0], end[1]);
    std::fs::write(out.join("board-light.svg"), bare.replace("</svg>", &dot("#111111", "#ffffff")))?;
    let mut dark = bare.clone();
    for (l, d) in LIGHT_INKS.iter().zip(DARK_INKS) {
        dark = dark.replace(&format!("#{l:06x}"), &format!("#{d:06x}"));
    }
    std::fs::write(out.join("board-dark.svg"), dark.replace("</svg>", &dot("#f4f4f0", "#000000")))?;
    println!("{{\"bounds\":[{},{},{},{}]}}", drawn.bounds.x0, drawn.bounds.y0, drawn.bounds.x1, drawn.bounds.y1);
    Ok(())
}

/// The picture without its sheet of paper, the first rectangle the SVG draws.
fn strip_paper(svg: &str) -> String {
    match (svg.find("<rect "), svg.find("<rect ").and_then(|at| svg[at..].find("/>").map(|end| at + end + 2))) {
        (Some(start), Some(end)) => format!("{}{}", &svg[..start], &svg[end..]),
        _ => svg.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_paper_goes_and_the_ink_stays() {
        let svg = r##"<svg><title>t</title><rect x="0" y="0" width="9" height="9" fill="#fbfbf9"/><g fill="#1e1e1e"><path d="M0 0Z"/></g></svg>"##;
        let bare = strip_paper(svg);
        assert!(!bare.contains("<rect") && bare.contains("<path"));
    }
}
