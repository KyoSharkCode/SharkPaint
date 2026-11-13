//! Renders every built-in tip, and a stroke made with it, to a PNG.
//! Usage: cargo run --example puntas -- salida.png

use sharkpaint::engine::{builtin_tips, write_png, BrushSettings, BrushTip, Document, StrokePoint};

fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| "puntas.png".into());
    let tips = builtin_tips();
    let mut doc = Document::new(900, 150 * tips.len());
    for (i, tip) in tips.iter().enumerate() {
        let y = 75.0 + i as f32 * 150.0;
        let stamp = BrushSettings { tip: BrushTip::Image(tip.clone()), spacing: 5.0, ..BrushSettings::round(120.0) };
        doc.begin_stroke(&stamp, [30, 30, 40], false, StrokePoint::new(80.0, y, 1.0));
        doc.end_stroke();
        let brush = BrushSettings {
            tip: BrushTip::Image(tip.clone()),
            spacing: 0.15,
            smoothing: 0.0,
            pressure_size: true,
            min_size: 0.2,
            ..BrushSettings::round(46.0)
        };
        let point = |t: f32| {
            StrokePoint::new(
                190.0 + t * 660.0,
                y + (t * std::f32::consts::TAU * 1.5).sin() * 35.0,
                (t * std::f32::consts::PI).sin().powf(0.6),
            )
        };
        doc.begin_stroke(&brush, [30, 30, 40], false, point(0.0));
        for s in 1..=300 {
            doc.stroke_to(point(s as f32 / 300.0));
        }
        doc.end_stroke();
    }
    let rgba = doc.render_rgba8(doc.bounds());
    write_png(path.as_ref(), doc.width as u32, doc.height as u32, &rgba).expect("write png");
    println!("escrito {path}");
}
