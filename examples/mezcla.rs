//! Renders the colour-mixing brushes crossing bands of paint, without opening a window.
//! Usage: cargo run --example mezcla -- salida.png

use sharkpaint::engine::{builtin_papers, builtin_tips, write_png, BrushSettings, BrushTip, Document, StrokePoint};

fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| "mezcla.png".into());
    let tips = builtin_tips();
    let papers = builtin_papers();
    let tip = |id: &str| BrushTip::Image(tips.iter().find(|t| t.id == id).expect("tip").clone());
    let brushes = [
        BrushSettings { mix: 0.5, drag: 0.75, flow: 0.7, spacing: 0.03, follow_direction: true, tip: tip("cerdas"), ..BrushSettings::round(48.0) },
        BrushSettings {
            mix: 0.35,
            drag: 0.4,
            flow: 0.25,
            hardness: 0.35,
            spacing: 0.06,
            paper: papers.iter().find(|p| p.id == "acuarela").cloned(),
            grain: 0.35,
            ..BrushSettings::round(60.0)
        },
        BrushSettings { mix: 1.0, drag: 0.5, flow: 0.6, hardness: 0.2, spacing: 0.06, ..BrushSettings::round(50.0) },
        BrushSettings { mix: 1.0, drag: 0.85, flow: 0.75, hardness: 0.6, spacing: 0.04, ..BrushSettings::round(34.0) },
    ];
    let mut doc = Document::new(900, 150 * brushes.len());
    let band = BrushSettings { hardness: 1.0, spacing: 0.1, ..BrushSettings::round(120.0) };
    for (i, color) in [[214, 72, 96], [240, 190, 70], [70, 150, 220], [40, 40, 48]].into_iter().enumerate() {
        let x = 200.0 + 170.0 * i as f32;
        doc.begin_stroke(&band, color, false, StrokePoint::new(x, -100.0, 1.0));
        doc.stroke_to(StrokePoint::new(x, doc.height as f32 + 100.0, 1.0));
        doc.end_stroke();
    }
    for (i, brush) in brushes.iter().enumerate() {
        let y0 = 75.0 + i as f32 * 150.0;
        let pt = |t: f32| StrokePoint::new(40.0 + t * 820.0, y0 + (t * std::f32::consts::TAU * 3.0).sin() * 30.0, 1.0);
        doc.begin_stroke(brush, [60, 170, 110], false, pt(0.0));
        for s in 1..=400 {
            doc.stroke_to(pt(s as f32 / 400.0));
        }
        doc.end_stroke();
    }
    let rgba = doc.render_rgba8(doc.bounds());
    write_png(path.as_ref(), doc.width as u32, doc.height as u32, &rgba).expect("write png");
    println!("escrito {path}");
}
