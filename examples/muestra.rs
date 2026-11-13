//! Renders a sample sheet of every brush preset to a PNG, without opening a window.
//! A path ending in the project extension saves a project file instead.
//! Usage: cargo run --example muestra -- salida.png

use sharkpaint::engine::{write_png, BrushPreset, Document, StrokePoint, PROJECT_EXTENSION};

fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| "muestra.png".into());
    let mut doc = Document::new(900, 640);
    let colors = [[40, 40, 48], [20, 30, 90], [210, 60, 90], [30, 140, 190]];
    for (i, preset) in BrushPreset::ALL.iter().enumerate() {
        let brush = preset.settings();
        let y0 = 90.0 + i as f32 * 150.0;
        // A wave whose pressure rises and falls, like a quick pen flick.
        let pt = |t: f32| {
            let x = 60.0 + t * 780.0;
            let y = y0 + (t * std::f32::consts::TAU * 2.0).sin() * 40.0;
            StrokePoint::new(x, y, (t * std::f32::consts::PI).sin().powf(0.7))
        };
        doc.begin_stroke(&brush, colors[i], false, pt(0.0));
        for s in 1..=400 {
            doc.stroke_to(pt(s as f32 / 400.0));
        }
        doc.end_stroke();
    }
    // Erase a diagonal band through everything.
    let eraser = BrushPreset::Marcador.settings();
    doc.begin_stroke(&eraser, [0, 0, 0], true, StrokePoint::new(650.0, 20.0, 1.0));
    doc.stroke_to(StrokePoint::new(760.0, 620.0, 1.0));
    doc.end_stroke();

    if std::path::Path::new(&path).extension().is_some_and(|e| e == PROJECT_EXTENSION) {
        doc.save(path.as_ref()).expect("save project");
    } else {
        let rgba = doc.render_rgba8(doc.bounds());
        write_png(path.as_ref(), doc.width as u32, doc.height as u32, &rgba).expect("write png");
    }
    println!("escrito {path}");
}
