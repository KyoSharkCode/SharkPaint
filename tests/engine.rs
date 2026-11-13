use sharkpaint::engine::{BrushPreset, BrushSettings, Document, IRect, StrokePoint};

fn hard_brush(size: f32) -> BrushSettings {
    BrushSettings {
        size,
        opacity: 1.0,
        flow: 1.0,
        hardness: 1.0,
        spacing: 0.1,
        smoothing: 0.0,
        pressure_size: false,
        pressure_flow: false,
        min_size: 1.0,
    }
}

fn pixel(doc: &Document, x: i32, y: i32) -> [u8; 4] {
    let px = doc.render_rgba8(IRect::new(x, y, x + 1, y + 1));
    [px[0], px[1], px[2], px[3]]
}

fn line(doc: &mut Document, brush: &BrushSettings, color: [u8; 3], erase: bool, a: (f32, f32), b: (f32, f32)) {
    doc.begin_stroke(brush, color, erase, StrokePoint::new(a.0, a.1, 1.0));
    doc.stroke_to(StrokePoint::new(b.0, b.1, 1.0));
    doc.end_stroke();
}

#[test]
fn stroke_paints_and_leaves_the_rest_untouched() {
    let mut doc = Document::new(200, 200);
    line(&mut doc, &hard_brush(20.0), [255, 0, 0], false, (20.0, 100.0), (180.0, 100.0));
    assert_eq!(pixel(&doc, 100, 100), [255, 0, 0, 255]);
    assert_eq!(pixel(&doc, 100, 20), [255, 255, 255, 255]);
    // Continuous coverage along the whole line, no gaps between dabs.
    for x in 20..180 {
        assert_eq!(pixel(&doc, x, 100), [255, 0, 0, 255], "gap at x={x}");
    }
}

#[test]
fn undo_and_redo_restore_exact_pixels() {
    let mut doc = Document::new(200, 200);
    let blank = doc.render_rgba8(doc.bounds());
    line(&mut doc, &hard_brush(20.0), [0, 0, 255], false, (20.0, 50.0), (180.0, 50.0));
    let one = doc.render_rgba8(doc.bounds());
    line(&mut doc, &hard_brush(20.0), [0, 255, 0], false, (100.0, 10.0), (100.0, 190.0));
    let two = doc.render_rgba8(doc.bounds());
    assert_ne!(one, two);

    assert!(doc.undo());
    assert_eq!(doc.render_rgba8(doc.bounds()), one);
    assert!(doc.undo());
    assert_eq!(doc.render_rgba8(doc.bounds()), blank);
    assert!(!doc.undo());
    assert!(doc.redo());
    assert!(doc.redo());
    assert_eq!(doc.render_rgba8(doc.bounds()), two);
    assert!(!doc.redo());
}

#[test]
fn new_stroke_discards_redo_history() {
    let mut doc = Document::new(100, 100);
    line(&mut doc, &hard_brush(10.0), [0, 0, 0], false, (10.0, 10.0), (90.0, 10.0));
    doc.undo();
    line(&mut doc, &hard_brush(10.0), [0, 0, 0], false, (10.0, 50.0), (90.0, 50.0));
    assert!(!doc.can_redo());
}

#[test]
fn stroke_opacity_caps_self_overlap() {
    let mut doc = Document::new(200, 200);
    let mut brush = hard_brush(30.0);
    brush.opacity = 0.5;
    doc.begin_stroke(&brush, [0, 0, 0], false, StrokePoint::new(20.0, 100.0, 1.0));
    // Scrub back and forth over the same pixels within one stroke.
    for _ in 0..5 {
        doc.stroke_to(StrokePoint::new(180.0, 100.0, 1.0));
        doc.stroke_to(StrokePoint::new(20.0, 100.0, 1.0));
    }
    doc.end_stroke();
    let v = pixel(&doc, 100, 100)[0];
    assert!((126..=129).contains(&v), "expected ~50% grey, got {v}");

    // A second stroke does darken it further.
    line(&mut doc, &brush, [0, 0, 0], false, (20.0, 100.0), (180.0, 100.0));
    let v2 = pixel(&doc, 100, 100)[0];
    assert!((62..=66).contains(&v2), "expected ~25% grey, got {v2}");
}

#[test]
fn eraser_removes_paint_and_is_undoable() {
    let mut doc = Document::new(200, 200);
    line(&mut doc, &hard_brush(40.0), [10, 20, 30], false, (20.0, 100.0), (180.0, 100.0));
    line(&mut doc, &hard_brush(20.0), [0, 0, 0], true, (100.0, 60.0), (100.0, 140.0));
    assert_eq!(pixel(&doc, 100, 100), [255, 255, 255, 255]);
    assert_eq!(pixel(&doc, 40, 100), [10, 20, 30, 255]);
    doc.undo();
    assert_eq!(pixel(&doc, 100, 100), [10, 20, 30, 255]);
}

#[test]
fn pressure_changes_stroke_width() {
    let mut doc = Document::new(300, 200);
    let mut brush = hard_brush(40.0);
    brush.pressure_size = true;
    brush.min_size = 0.0;
    doc.begin_stroke(&brush, [0, 0, 0], false, StrokePoint::new(20.0, 100.0, 0.1));
    doc.stroke_to(StrokePoint::new(280.0, 100.0, 1.0));
    doc.end_stroke();
    let width_at = |x: i32| (0..200).filter(|&y| pixel(&doc, x, y)[0] < 128).count();
    let (thin, thick) = (width_at(40), width_at(270));
    assert!(thin < 12, "thin end too wide: {thin}");
    assert!(thick > 30, "thick end too narrow: {thick}");
}

#[test]
fn strokes_outside_the_canvas_are_safe() {
    let mut doc = Document::new(100, 100);
    let mut brush = BrushPreset::Aerografo.settings();
    brush.smoothing = 0.0; // keep the path exactly on the input points
    doc.begin_stroke(&brush, [0, 0, 0], false, StrokePoint::new(-500.0, -500.0, 1.0));
    doc.stroke_to(StrokePoint::new(50.0, 50.0, 1.0));
    doc.stroke_to(StrokePoint::new(900.0, 400.0, 1.0));
    doc.end_stroke();
    assert!(pixel(&doc, 50, 50)[0] < 255);
    // A stroke entirely off-canvas adds no undo step.
    let mut doc = Document::new(100, 100);
    line(&mut doc, &hard_brush(10.0), [0, 0, 0], false, (-300.0, -300.0), (-200.0, -200.0));
    assert!(!doc.can_undo());
}

#[test]
fn dirty_region_tracks_changes() {
    let mut doc = Document::new(500, 500);
    assert_eq!(doc.take_dirty(), Some(doc.bounds())); // new document needs a full draw
    assert_eq!(doc.take_dirty(), None);
    line(&mut doc, &hard_brush(10.0), [0, 0, 0], false, (100.0, 100.0), (150.0, 100.0));
    let d = doc.take_dirty().unwrap();
    assert!(d.x0 <= 95 && d.x1 >= 155 && d.y0 <= 95 && d.y1 >= 105);
    assert!(d.width() < 100 && d.height() < 40, "dirty rect too large: {d:?}");
}

#[test]
fn clear_layer_is_undoable() {
    let mut doc = Document::new(100, 100);
    line(&mut doc, &hard_brush(10.0), [0, 0, 0], false, (10.0, 50.0), (90.0, 50.0));
    doc.clear_active_layer();
    assert_eq!(pixel(&doc, 50, 50), [255, 255, 255, 255]);
    doc.undo();
    assert_eq!(pixel(&doc, 50, 50), [0, 0, 0, 255]);
}

#[test]
fn project_round_trips_through_the_file_format() {
    let mut doc = Document::new(300, 200);
    doc.paper = [250, 240, 230];
    line(&mut doc, &BrushPreset::Aerografo.settings(), [200, 30, 90], false, (20.0, 40.0), (280.0, 160.0));
    line(&mut doc, &hard_brush(12.0), [10, 80, 160], false, (20.0, 160.0), (280.0, 40.0));
    let mut bytes = Vec::new();
    doc.write_to(&mut bytes).unwrap();
    let loaded = Document::read_from(&mut bytes.as_slice()).unwrap();
    assert_eq!((loaded.width, loaded.height, loaded.paper), (300, 200, [250, 240, 230]));
    assert_eq!(loaded.render_rgba8(loaded.bounds()), doc.render_rgba8(doc.bounds()));
    // Full 15-bit precision survives, not just the 8-bit preview.
    for (x, y) in [(150, 100), (60, 60), (240, 140)] {
        assert_eq!(loaded.layers[0].pixel(x, y), doc.layers[0].pixel(x, y));
    }
    // A loaded project starts with a clean history.
    assert!(!loaded.can_undo());
}

#[test]
fn damaged_project_files_are_rejected() {
    let mut doc = Document::new(100, 100);
    line(&mut doc, &hard_brush(10.0), [0, 0, 0], false, (10.0, 50.0), (90.0, 50.0));
    let mut bytes = Vec::new();
    doc.write_to(&mut bytes).unwrap();

    assert!(Document::read_from(&mut &b"esto no es un proyecto"[..]).is_err());
    assert!(Document::read_from(&mut &bytes[..bytes.len() / 2]).is_err());
    let mut wrong_version = bytes.clone();
    wrong_version[8] = 99;
    assert!(Document::read_from(&mut wrong_version.as_slice()).is_err());
    let mut huge = bytes.clone();
    huge[12..16].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(Document::read_from(&mut huge.as_slice()).is_err());
}

#[test]
fn revision_tracks_pixel_changes() {
    let mut doc = Document::new(100, 100);
    let r0 = doc.revision();
    line(&mut doc, &hard_brush(10.0), [0, 0, 0], false, (10.0, 50.0), (90.0, 50.0));
    let r1 = doc.revision();
    assert_ne!(r0, r1);
    doc.undo();
    assert_ne!(doc.revision(), r1);
}
