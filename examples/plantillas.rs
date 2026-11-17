//! Writes the drawing templates: projects with named layers ready to draw on
//! and a group of construction guides above them.
//!
//! - `plantilla-personaje.spaint`: 1080×1920 (vertical video), a figure four
//!   heads tall with the areas a phone app covers marked.
//! - `plantilla-expresiones.spaint`: 2160×2160, nine heads to practise faces.
//!
//! Each project also gets a PNG preview beside it.
//! Usage: cargo run --example plantillas -- [carpeta]

use std::path::{Path, PathBuf};

use sharkpaint::engine::{
    render_text, write_png, BlendMode, BrushPreset, Document, LayerProps, Shape, StrokePoint, TextFont, TextSpec,
    BUILTIN_FONT,
};

/// Construction lines.
const GUIDE: [u8; 3] = [58, 123, 213];
/// Features of the character: fins and tail.
const FEATURE: [u8; 3] = [43, 47, 122];
/// Areas the phone app covers.
const COVERED: [u8; 3] = [179, 38, 63];
/// Lines between the cells of the expression sheet.
const GRID: [u8; 3] = [189, 176, 230];

/// Width of the guide lines, in pixels.
const LINE: f32 = 4.0;

fn main() {
    let folder = PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| "plantillas".into()));
    std::fs::create_dir_all(&folder).expect("create the output folder");
    save(&character(), &folder.join("plantilla-personaje"));
    save(&expressions(), &folder.join("plantilla-expresiones"));
}

/// The layers to draw on, bottom first, with the guide layer on top and
/// selected. Returns the index of the sketch layer.
fn drawing_layers(doc: &mut Document) -> usize {
    let mut sketch = 0;
    let layers: [(&str, f32, BlendMode, bool); 6] = [
        ("Color", 1.0, BlendMode::Normal, false),
        ("Sombra", 0.6, BlendMode::Multiply, true),
        ("Brillos", 1.0, BlendMode::Add, true),
        ("Boceto", 0.5, BlendMode::Normal, false),
        ("Línea", 1.0, BlendMode::Normal, false),
        ("Efectos", 1.0, BlendMode::Normal, false),
    ];
    for (i, (name, opacity, blend, clip)) in layers.into_iter().enumerate() {
        if i > 0 {
            doc.add_layer();
        }
        let props = LayerProps { opacity, blend, clip, ..LayerProps::named(name) };
        doc.set_layer_props(doc.active_layer(), props, false);
        if name == "Boceto" {
            sketch = doc.active_layer();
        }
    }
    doc.add_layer();
    doc.set_layer_props(doc.active_layer(), LayerProps::named("Guías"), false);
    let id = doc.group_active_layer().expect("group the guides");
    let mut group = doc.group(id).expect("the new group").clone();
    group.name = "Guías (ocultar antes de exportar)".into();
    group.opacity = 0.7;
    doc.set_group(group, false);
    sketch
}

fn ellipse(doc: &mut Document, cx: f32, cy: f32, rx: f32, ry: f32, color: [u8; 3]) {
    doc.draw_shape(Shape::Ellipse, (cx - rx, cy - ry), (cx + rx, cy + ry), color, LINE, false, 1.0);
}

fn line(doc: &mut Document, from: (f32, f32), to: (f32, f32), color: [u8; 3]) {
    doc.draw_shape(Shape::Line, from, to, color, LINE, false, 1.0);
}

fn polyline(doc: &mut Document, points: &[(f32, f32)], color: [u8; 3]) {
    for pair in points.windows(2) {
        line(doc, pair[0], pair[1], color);
    }
}

/// A smooth curve through the points (Catmull-Rom), drawn with an even ink line.
fn curve(doc: &mut Document, points: &[(f32, f32)], color: [u8; 3]) {
    let mut brush = BrushPreset::Tinta.settings();
    brush.size = LINE;
    brush.pressure_size = false;
    brush.pressure_flow = false;
    brush.smoothing = 0.0;
    let at = |i: isize| points[i.clamp(0, points.len() as isize - 1) as usize];
    let mut first = true;
    for i in 0..points.len() as isize - 1 {
        let (p0, p1, p2, p3) = (at(i - 1), at(i), at(i + 1), at(i + 2));
        for s in 0..=24 {
            let t = s as f32 / 24.0;
            let (t2, t3) = (t * t, t * t * t);
            let f = |a: f32, b: f32, c: f32, d: f32| {
                0.5 * (2.0 * b + (c - a) * t + (2.0 * a - 5.0 * b + 4.0 * c - d) * t2 + (3.0 * b - a - 3.0 * c + d) * t3)
            };
            let p = StrokePoint::new(f(p0.0, p1.0, p2.0, p3.0), f(p0.1, p1.1, p2.1, p3.1), 1.0);
            if first {
                doc.begin_stroke(&brush, color, false, p);
                first = false;
            } else {
                doc.stroke_to(p);
            }
        }
    }
    doc.end_stroke();
}

/// Adds a text layer (it joins the guides group) and goes back to the guides.
fn label(doc: &mut Document, text: &str, x: i32, y: i32, size: f32, color: [u8; 3]) {
    let guides = doc.active_layer();
    let font = TextFont::builtin();
    let (width, height, rgba) = render_text(&font, text, size, color).expect("draw the label");
    let spec = TextSpec { content: text.into(), font: BUILTIN_FONT.into(), size, color, x, y };
    doc.add_text_layer(spec, width, height, &rgba);
    doc.set_active_layer(guides);
}

/// A head seen from the front: skull, chin, centre line, eye line and the
/// two fins on top. `r` is the radius of the skull.
fn head(doc: &mut Document, cx: f32, cy: f32, r: f32) {
    ellipse(doc, cx, cy, r, r, GUIDE);
    // The chin: a soft point a little below the circle.
    curve(doc, &[(cx - r * 0.92, cy + r * 0.38), (cx - r * 0.45, cy + r * 1.0), (cx, cy + r * 1.18)], GUIDE);
    curve(doc, &[(cx + r * 0.92, cy + r * 0.38), (cx + r * 0.45, cy + r * 1.0), (cx, cy + r * 1.18)], GUIDE);
    // Centre line and eye line: in this style the eyes sit low and wide.
    line(doc, (cx, cy - r * 1.05), (cx, cy + r * 1.25), GUIDE);
    line(doc, (cx - r * 1.05, cy + r * 0.15), (cx + r * 1.05, cy + r * 0.15), GUIDE);
    // Fin-ears: a shark fin on each side of the top of the head, swept
    // outwards, with a rounded front edge and a hollow back edge.
    for side in [-1.0f32, 1.0] {
        let base_in = (cx + side * r * 0.3, cy - r * 0.95);
        let tip = (cx + side * r * 1.15, cy - r * 1.5);
        let base_out = (cx + side * r * 0.86, cy - r * 0.5);
        curve(doc, &[base_in, (cx + side * r * 0.55, cy - r * 1.4), tip], FEATURE);
        curve(doc, &[tip, (cx + side * r * 0.9, cy - r * 1.0), base_out], FEATURE);
    }
}

fn character() -> Document {
    let (w, h) = (1080usize, 1920usize);
    let mut doc = Document::new(w, h);
    let sketch = drawing_layers(&mut doc);
    let cx = w as f32 / 2.0;

    // The app's buttons cover about the top 8 % and bottom 20 % of the video.
    let (top, bottom) = (h as f32 * 0.08, h as f32 * 0.80);
    doc.draw_shape(Shape::Rectangle, (0.0, 0.0), (w as f32, top), COVERED, 1.0, true, 0.10);
    doc.draw_shape(Shape::Rectangle, (0.0, bottom), (w as f32, h as f32), COVERED, 1.0, true, 0.10);
    line(&mut doc, (0.0, top), (w as f32, top), COVERED);
    line(&mut doc, (0.0, bottom), (w as f32, bottom), COVERED);

    // Four heads from y0 to y1.
    let (y0, y1) = (200.0f32, 1500.0f32);
    let unit = (y1 - y0) / 4.0;
    for i in 0..=4 {
        let y = y0 + unit * i as f32;
        line(&mut doc, (40.0, y), (180.0, y), GUIDE);
    }

    // Head: the skull fills the first unit down to the chin.
    let r = unit * 0.46;
    let head_cy = y0 + r;
    head(&mut doc, cx, head_cy, r);

    // Neck.
    let chin = head_cy + r * 1.18;
    line(&mut doc, (cx - 22.0, chin - 10.0), (cx - 22.0, chin + 35.0), GUIDE);
    line(&mut doc, (cx + 22.0, chin - 10.0), (cx + 22.0, chin + 35.0), GUIDE);

    // Body as two beans: chest down to the waist, hips down to the crotch.
    let shoulders = y0 + unit * 1.2;
    let waist = y0 + unit * 1.85;
    let crotch = y0 + unit * 2.5;
    ellipse(&mut doc, cx, (shoulders + waist) / 2.0, 145.0, (waist - shoulders) / 2.0 + 20.0, GUIDE);
    ellipse(&mut doc, cx, (waist + crotch) / 2.0 + 5.0, 165.0, (crotch - waist) / 2.0 + 15.0, GUIDE);
    line(&mut doc, (cx - 165.0, shoulders), (cx + 165.0, shoulders), GUIDE);

    for side in [-1.0f32, 1.0] {
        // Arm: shoulder, elbow at the waist, wrist at the hips, mitten hand.
        let shoulder = (cx + side * 150.0, shoulders + 15.0);
        let elbow = (cx + side * 190.0, waist + 10.0);
        let wrist = (cx + side * 200.0, crotch - 30.0);
        polyline(&mut doc, &[shoulder, elbow, wrist], GUIDE);
        ellipse(&mut doc, elbow.0, elbow.1, 14.0, 14.0, GUIDE);
        ellipse(&mut doc, wrist.0, wrist.1 + 38.0, 40.0, 42.0, GUIDE);
        // Leg: wide at the hip, narrowing to a small foot.
        let foot = y1 - 25.0;
        curve(&mut doc, &[(cx + side * 160.0, waist + 120.0), (cx + side * 120.0, crotch + unit * 0.6), (cx + side * 75.0, foot)], GUIDE);
        curve(&mut doc, &[(cx + side * 12.0, crotch), (cx + side * 22.0, crotch + unit * 0.6), (cx + side * 25.0, foot)], GUIDE);
        ellipse(&mut doc, cx + side * 55.0, y1 - 12.0, 42.0, 18.0, GUIDE);
    }

    // Shark tail from behind the right hip.
    let tail = [(cx + 150.0, waist + 110.0), (cx + 270.0, crotch + 20.0), (cx + 340.0, crotch + 150.0), (cx + 360.0, crotch + 260.0)];
    curve(&mut doc, &tail, FEATURE);
    // Tail fin: two lobes with a notch between them.
    let end = tail[3];
    let (lobe_a, notch, lobe_b) = ((end.0 - 95.0, end.1 + 95.0), (end.0 + 10.0, end.1 + 55.0), (end.0 + 105.0, end.1 + 70.0));
    curve(&mut doc, &[end, (end.0 - 55.0, end.1 + 35.0), lobe_a], FEATURE);
    curve(&mut doc, &[lobe_a, (end.0 - 35.0, end.1 + 65.0), notch], FEATURE);
    curve(&mut doc, &[notch, (end.0 + 55.0, end.1 + 55.0), lobe_b], FEATURE);
    curve(&mut doc, &[lobe_b, (end.0 + 55.0, end.1 + 25.0), end], FEATURE);

    for (i, name) in ["1 cabeza", "2 torso", "3 cadera", "4 piernas"].iter().enumerate() {
        let y = y0 + unit * i as f32 + unit / 2.0 - 18.0;
        label(&mut doc, name, 40, y as i32, 34.0, FEATURE);
    }
    label(&mut doc, "Zona tapada por la app", 40, (top - 52.0) as i32, 34.0, COVERED);
    label(&mut doc, "Zona tapada por la app (botones y texto)", 40, (bottom + 20.0) as i32, 34.0, COVERED);

    doc.set_active_layer(sketch);
    doc
}

fn expressions() -> Document {
    let side = 2160usize;
    let cell = side as f32 / 3.0;
    let mut doc = Document::new(side, side);
    let sketch = drawing_layers(&mut doc);
    for i in 1..3 {
        let p = cell * i as f32;
        line(&mut doc, (p, 0.0), (p, side as f32), GRID);
        line(&mut doc, (0.0, p), (side as f32, p), GRID);
    }
    let names = ["Feliz", "Triste", "Enojada", "Sorprendida", "Presumida", "Muerta de risa", "Confundida", "Avergonzada", "Sin alma"];
    for (i, name) in names.iter().enumerate() {
        let (col, row) = ((i % 3) as f32, (i / 3) as f32);
        let (cx, cy) = (cell * (col + 0.5), cell * (row + 0.5) - 20.0);
        head(&mut doc, cx, cy, 170.0);
        label(&mut doc, name, (cell * col + 30.0) as i32, (cell * (row + 1.0) - 70.0) as i32, 44.0, FEATURE);
    }
    doc.set_active_layer(sketch);
    doc
}

fn save(doc: &Document, stem: &Path) {
    let project = stem.with_extension(sharkpaint::engine::PROJECT_EXTENSION);
    doc.save(&project).expect("save the project");
    let preview = stem.with_extension("png");
    let rgba = doc.render_rgba8(doc.bounds());
    write_png(&preview, doc.width as u32, doc.height as u32, &rgba).expect("write the preview");
    println!("escrito {} y {}", project.display(), preview.display());
}
