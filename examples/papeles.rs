//! Renders one stroke per built-in paper, pressed harder from left to right.
//! Usage: cargo run --example papeles -- salida.png

use sharkpaint::engine::{builtin_papers, write_png, BrushSettings, Document, StrokePoint};

fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| "papeles.png".into());
    let papers = builtin_papers();
    let mut doc = Document::new(1000, 130 * papers.len());
    for (i, paper) in papers.iter().enumerate() {
        let brush = BrushSettings {
            paper: Some(paper.clone()),
            grain: 1.0,
            hardness: 0.8,
            spacing: 0.06,
            smoothing: 0.0,
            ..BrushSettings::round(70.0)
        };
        let y = 65.0 + i as f32 * 130.0;
        doc.begin_stroke(&brush, [35, 35, 45], false, StrokePoint::new(60.0, y, 0.05));
        for s in 1..=200 {
            let t = s as f32 / 200.0;
            doc.stroke_to(StrokePoint::new(60.0 + t * 880.0, y, 0.05 + 0.95 * t));
        }
        doc.end_stroke();
    }
    let rgba = doc.render_rgba8(doc.bounds());
    write_png(path.as_ref(), doc.width as u32, doc.height as u32, &rgba).expect("write png");
    println!("escrito {path}");
}
