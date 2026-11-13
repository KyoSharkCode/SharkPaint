use sharkpaint::engine::{
    BlendMode, BrushPreset, BrushSettings, Document, IRect, LayerTransform, Reference, StrokePoint,
};

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
        assert_eq!(loaded.layers()[0].pixel(x, y), doc.layers()[0].pixel(x, y));
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

#[test]
fn layers_stack_in_order_and_can_be_hidden() {
    let mut doc = Document::new(100, 100);
    line(&mut doc, &hard_brush(40.0), [255, 0, 0], false, (10.0, 50.0), (90.0, 50.0));
    doc.add_layer();
    assert_eq!((doc.layers().len(), doc.active_layer()), (2, 1));
    line(&mut doc, &hard_brush(20.0), [0, 0, 255], false, (50.0, 10.0), (50.0, 90.0));
    assert_eq!(pixel(&doc, 50, 50), [0, 0, 255, 255]); // top layer covers
    assert_eq!(pixel(&doc, 20, 50), [255, 0, 0, 255]);

    let mut props = doc.active().props.clone();
    props.visible = false;
    doc.set_layer_props(1, props, false);
    assert_eq!(pixel(&doc, 50, 50), [255, 0, 0, 255]);
    doc.undo();
    assert_eq!(pixel(&doc, 50, 50), [0, 0, 255, 255]);

    // Moving the blue layer under the red one hides it where they overlap.
    assert!(doc.move_layer(false));
    assert_eq!(pixel(&doc, 50, 50), [255, 0, 0, 255]);
    assert_eq!(pixel(&doc, 50, 15), [0, 0, 255, 255]);
    assert!(!doc.move_layer(false));
}

#[test]
fn layer_opacity_and_blend_modes() {
    let mut doc = Document::new(100, 100);
    line(&mut doc, &hard_brush(60.0), [200, 100, 50], false, (10.0, 50.0), (90.0, 50.0));
    doc.add_layer();
    line(&mut doc, &hard_brush(60.0), [100, 200, 250], false, (10.0, 50.0), (90.0, 50.0));

    let set = |doc: &mut Document, opacity: f32, blend: BlendMode| {
        let mut props = doc.active().props.clone();
        props.opacity = opacity;
        props.blend = blend;
        doc.set_layer_props(1, props, false);
    };
    let near = |got: [u8; 4], want: [u8; 3]| {
        (0..3).all(|i| (got[i] as i32 - want[i] as i32).abs() <= 1)
    };
    set(&mut doc, 0.5, BlendMode::Normal);
    assert!(near(pixel(&doc, 50, 50), [150, 150, 150]), "{:?}", pixel(&doc, 50, 50));
    set(&mut doc, 1.0, BlendMode::Multiply);
    assert!(near(pixel(&doc, 50, 50), [78, 78, 49]), "{:?}", pixel(&doc, 50, 50));
    set(&mut doc, 1.0, BlendMode::Screen);
    assert!(near(pixel(&doc, 50, 50), [222, 222, 251]), "{:?}", pixel(&doc, 50, 50));
    set(&mut doc, 1.0, BlendMode::Darken);
    assert!(near(pixel(&doc, 50, 50), [100, 100, 50]));
    set(&mut doc, 1.0, BlendMode::Lighten);
    assert!(near(pixel(&doc, 50, 50), [200, 200, 250]));
    set(&mut doc, 1.0, BlendMode::Add);
    assert!(near(pixel(&doc, 50, 50), [255, 255, 255]));
    // Multiply against bare paper (white) leaves the layer's own colour.
    set(&mut doc, 1.0, BlendMode::Multiply);
    doc.set_active_layer(0);
    doc.clear_active_layer();
    assert!(near(pixel(&doc, 50, 50), [100, 200, 250]));
}

#[test]
fn merge_down_matches_the_stacked_result_and_undoes() {
    let mut doc = Document::new(120, 120);
    line(&mut doc, &hard_brush(50.0), [220, 60, 60], false, (10.0, 60.0), (110.0, 60.0));
    doc.add_layer();
    line(&mut doc, &BrushPreset::Aerografo.settings(), [40, 90, 200], false, (60.0, 10.0), (60.0, 110.0));
    let mut props = doc.active().props.clone();
    props.opacity = 0.7;
    props.blend = BlendMode::Multiply;
    doc.set_layer_props(1, props, false);
    let stacked = doc.render_rgba8(doc.bounds());

    assert!(doc.merge_down());
    assert_eq!((doc.layers().len(), doc.active_layer()), (1, 0));
    let merged = doc.render_rgba8(doc.bounds());
    let worst = stacked.iter().zip(&merged).map(|(a, b)| (*a as i32 - *b as i32).abs()).max().unwrap();
    assert!(worst <= 1, "merge changed the picture by {worst} levels");
    assert!(!doc.merge_down()); // nothing beneath the bottom layer

    doc.undo();
    assert_eq!(doc.layers().len(), 2);
    assert_eq!(doc.render_rgba8(doc.bounds()), stacked);
}

#[test]
fn layer_structure_changes_are_undoable() {
    let mut doc = Document::new(80, 80);
    line(&mut doc, &hard_brush(20.0), [0, 0, 0], false, (10.0, 40.0), (70.0, 40.0));
    let original = doc.render_rgba8(doc.bounds());
    assert!(!doc.delete_layer()); // the only layer stays

    doc.duplicate_layer();
    assert_eq!(doc.layers()[1].props.name, "Capa 1 copia");
    doc.add_layer();
    assert_eq!(doc.layers()[2].props.name, "Capa 2");
    doc.set_active_layer(0);
    assert!(doc.delete_layer());
    assert_eq!(doc.layers().len(), 2);
    assert_eq!(doc.render_rgba8(doc.bounds()), original); // the copy still shows it

    for _ in 0..3 {
        assert!(doc.undo());
    }
    assert_eq!(doc.layers().len(), 1);
    assert_eq!(doc.layers()[0].props.name, "Capa 1");
    for _ in 0..3 {
        assert!(doc.redo());
    }
    assert_eq!(doc.layers().len(), 2);
}

#[test]
fn slider_drags_coalesce_into_one_undo_step() {
    let mut doc = Document::new(50, 50);
    for (i, opacity) in [0.9, 0.7, 0.5, 0.3].into_iter().enumerate() {
        let mut props = doc.active().props.clone();
        props.opacity = opacity;
        doc.set_layer_props(0, props, i > 0);
    }
    assert_eq!(doc.active().props.opacity, 0.3);
    doc.undo();
    assert_eq!(doc.active().props.opacity, 1.0);
    assert!(!doc.can_undo());
}

#[test]
fn alpha_lock_paints_only_existing_pixels() {
    let mut doc = Document::new(100, 100);
    line(&mut doc, &hard_brush(20.0), [255, 0, 0], false, (10.0, 50.0), (90.0, 50.0));
    let mut props = doc.active().props.clone();
    props.alpha_lock = true;
    doc.set_layer_props(0, props, false);
    line(&mut doc, &hard_brush(60.0), [0, 0, 255], false, (50.0, 5.0), (50.0, 95.0));
    assert_eq!(pixel(&doc, 50, 50), [0, 0, 255, 255]); // recoloured inside the old stroke
    assert_eq!(pixel(&doc, 50, 20), [255, 255, 255, 255]); // nothing outside it
    assert_eq!(doc.layers()[0].pixel(50, 20), [0; 4]);
    // The eraser cannot change locked transparency.
    line(&mut doc, &hard_brush(60.0), [0, 0, 0], true, (50.0, 5.0), (50.0, 95.0));
    assert_eq!(pixel(&doc, 50, 50), [0, 0, 255, 255]);
}

#[test]
fn cancelled_stroke_leaves_no_trace() {
    let mut doc = Document::new(100, 100);
    line(&mut doc, &hard_brush(10.0), [0, 0, 0], false, (10.0, 20.0), (90.0, 20.0));
    let before = doc.render_rgba8(doc.bounds());
    let revision = doc.revision();
    doc.begin_stroke(&hard_brush(30.0), [255, 0, 0], false, StrokePoint::new(10.0, 60.0, 1.0));
    doc.stroke_to(StrokePoint::new(90.0, 20.0, 1.0));
    doc.cancel_stroke();
    assert_eq!(doc.render_rgba8(doc.bounds()), before);
    assert_eq!(doc.revision(), revision);
    doc.undo(); // undoes the first stroke, not a phantom step
    assert_eq!(pixel(&doc, 50, 20), [255, 255, 255, 255]);
}

#[test]
fn layers_survive_the_file_format() {
    let mut doc = Document::new(90, 70);
    line(&mut doc, &hard_brush(30.0), [200, 40, 40], false, (5.0, 35.0), (85.0, 35.0));
    doc.add_layer();
    line(&mut doc, &hard_brush(30.0), [40, 40, 200], false, (45.0, 5.0), (45.0, 65.0));
    let mut props = doc.active().props.clone();
    props.name = "Sombras ✦".into();
    props.opacity = 0.6;
    props.blend = BlendMode::Multiply;
    props.alpha_lock = true;
    doc.set_layer_props(1, props.clone(), false);
    let mut hidden = doc.layers()[0].props.clone();
    hidden.visible = false;
    doc.set_layer_props(0, hidden, false);

    let mut bytes = Vec::new();
    doc.write_to(&mut bytes).unwrap();
    let loaded = Document::read_from(&mut bytes.as_slice()).unwrap();
    assert_eq!(loaded.layers().len(), 2);
    assert_eq!(loaded.active_layer(), 1);
    assert!(!loaded.layers()[0].props.visible);
    let got = &loaded.layers()[1].props;
    assert_eq!((got.name.as_str(), got.blend, got.alpha_lock), ("Sombras ✦", BlendMode::Multiply, true));
    assert!((got.opacity - 0.6).abs() < 1e-4);
    assert_eq!(loaded.render_rgba8(loaded.bounds()), doc.render_rgba8(doc.bounds()));
}

#[test]
fn thumbnails_keep_proportions_and_thin_lines() {
    let mut doc = Document::new(3000, 2000);
    line(&mut doc, &hard_brush(3.0), [0, 0, 0], false, (100.0, 1000.0), (2900.0, 1000.0));
    let (w, h, px) = doc.layers()[0].thumbnail(56, 36, [255, 255, 255]);
    assert_eq!((w, h), (54, 36));
    assert_eq!(px.len(), w * h * 4);
    // The 3 px line is far thinner than one thumbnail pixel, yet still shows.
    let middle_row = &px[18 * w * 4..19 * w * 4];
    assert!(middle_row.chunks(4).filter(|p| p[0] < 215).count() > 40);
    assert_eq!(&px[..4], &[255, 255, 255, 255]);
    // A tall canvas is limited by height instead.
    let (w, h, _) = Document::new(500, 2000).layers()[0].thumbnail(56, 36, [0, 0, 0]);
    assert_eq!((w, h), (9, 36));
}

#[test]
fn layer_versions_track_pixel_changes_and_ids_stay_unique() {
    let mut doc = Document::new(100, 100);
    let v0 = doc.active().version;
    line(&mut doc, &hard_brush(10.0), [0, 0, 0], false, (10.0, 50.0), (90.0, 50.0));
    let v1 = doc.active().version;
    assert_ne!(v0, v1);
    doc.undo();
    assert_ne!(doc.active().version, v1);
    doc.add_layer();
    doc.duplicate_layer();
    let mut ids: Vec<u64> = doc.layers().iter().map(|l| l.id).collect();
    ids.dedup();
    assert_eq!(ids.len(), 3);
    // Changing only properties leaves the pixels' version alone.
    let v = doc.active().version;
    let mut props = doc.active().props.clone();
    props.opacity = 0.5;
    doc.set_layer_props(doc.active_layer(), props, false);
    assert_eq!(doc.active().version, v);
}

#[test]
fn flood_fill_stays_inside_closed_shapes() {
    let mut doc = Document::new(200, 200);
    // A closed square outline.
    let brush = hard_brush(6.0);
    for (a, b) in [((50.0, 50.0), (150.0, 50.0)), ((150.0, 50.0), (150.0, 150.0)),
                   ((150.0, 150.0), (50.0, 150.0)), ((50.0, 150.0), (50.0, 50.0))] {
        line(&mut doc, &brush, [0, 0, 0], false, a, b);
    }
    assert!(doc.flood_fill(100, 100, [255, 0, 0], 0.1, false));
    assert_eq!(pixel(&doc, 100, 100), [255, 0, 0, 255]);
    assert_eq!(pixel(&doc, 60, 140), [255, 0, 0, 255]);
    assert_eq!(pixel(&doc, 20, 20), [255, 255, 255, 255]); // outside untouched
    assert_eq!(pixel(&doc, 50, 100), [0, 0, 0, 255]); // the outline survives
    // No light fringe between fill and outline: scanning across the edge never
    // gets brighter than the fill or the line.
    for x in 50..100 {
        let p = pixel(&doc, x, 100);
        assert!(p[1] < 30 && p[2] < 30, "fringe at x={x}: {p:?}");
    }
    doc.undo();
    assert_eq!(pixel(&doc, 100, 100), [255, 255, 255, 255]);

    // Outside the canvas or on a hidden layer nothing happens.
    assert!(!doc.flood_fill(-5, 10, [0, 0, 255], 0.1, false));
}

#[test]
fn flood_fill_can_follow_lines_on_another_layer() {
    let mut doc = Document::new(200, 200);
    let brush = hard_brush(6.0);
    for (a, b) in [((50.0, 50.0), (150.0, 50.0)), ((150.0, 50.0), (150.0, 150.0)),
                   ((150.0, 150.0), (50.0, 150.0)), ((50.0, 150.0), (50.0, 50.0))] {
        line(&mut doc, &brush, [0, 0, 0], false, a, b);
    }
    doc.add_layer();
    doc.move_layer(false); // colour layer underneath the line art
    // Judged on its own, the empty colour layer fills completely.
    assert!(doc.flood_fill(100, 100, [0, 200, 0], 0.1, false));
    assert_eq!(doc.layers()[0].pixel(20, 20)[3], 32768);
    doc.undo();
    // Looking at all layers, the fill stops at the lines.
    assert!(doc.flood_fill(100, 100, [0, 200, 0], 0.1, true));
    assert_eq!(pixel(&doc, 100, 100), [0, 200, 0, 255]);
    assert_eq!(doc.layers()[0].pixel(20, 20), [0; 4]);
    assert_eq!(doc.layers().len(), 2);
}

#[test]
fn moving_a_layer_shifts_its_pixels_and_undoes() {
    let mut doc = Document::new(300, 300);
    line(&mut doc, &hard_brush(20.0), [255, 0, 0], false, (50.0, 50.0), (60.0, 50.0));
    let before = doc.render_rgba8(doc.bounds());
    for (dx, dy) in [(37, 81), (64, 128), (-13, 0)] {
        doc.begin_layer_move();
        doc.move_layer_to(5, 5); // intermediate drag positions do not accumulate
        doc.move_layer_to(dx, dy);
        doc.end_stroke();
        assert_eq!(pixel(&doc, 55 + dx, 50 + dy), [255, 0, 0, 255], "offset {dx},{dy}");
        assert_eq!(pixel(&doc, 55, 50), if (dx, dy) == (-13, 0) { [255, 0, 0, 255] } else { [255, 255, 255, 255] });
        assert!(doc.undo());
        assert_eq!(doc.render_rgba8(doc.bounds()), before);
    }
    // Dragging off the canvas and back loses nothing until the move ends.
    doc.begin_layer_move();
    doc.move_layer_to(-500, 0);
    assert_eq!(doc.layers()[0].tiles.iter().flatten().count(), 0);
    doc.move_layer_to(0, 0);
    doc.end_stroke();
    assert_eq!(doc.render_rgba8(doc.bounds()), before);
    // A move that ends where it began is not a step: undo reaches the stroke itself.
    doc.undo();
    assert_eq!(pixel(&doc, 55, 50), [255, 255, 255, 255]);
    assert!(!doc.can_undo());
}

#[test]
fn smudge_drags_paint_and_is_one_undo_step() {
    let mut doc = Document::new(300, 200);
    line(&mut doc, &hard_brush(40.0), [0, 0, 255], false, (60.0, 100.0), (80.0, 100.0));
    assert_eq!(pixel(&doc, 130, 100), [255, 255, 255, 255]);
    let before = doc.render_rgba8(doc.bounds());
    let mut finger = hard_brush(40.0);
    finger.hardness = 0.5;
    finger.flow = 0.9;
    doc.begin_smudge(&finger, StrokePoint::new(70.0, 100.0, 1.0));
    doc.stroke_to(StrokePoint::new(160.0, 100.0, 1.0));
    doc.end_stroke();
    let dragged = pixel(&doc, 130, 100);
    assert!(dragged[2] > dragged[0] + 40, "no blue pulled to the right: {dragged:?}");
    assert_eq!(pixel(&doc, 150, 30), [255, 255, 255, 255]); // far from the path
    assert!(doc.undo());
    assert_eq!(doc.render_rgba8(doc.bounds()), before);
    assert!(!doc.can_undo() || { doc.undo(); !doc.can_undo() });
}

#[test]
fn inserted_images_become_an_undoable_layer() {
    let mut doc = Document::new(100, 80);
    line(&mut doc, &hard_brush(10.0), [0, 0, 0], false, (10.0, 40.0), (90.0, 40.0));
    let before = doc.render_rgba8(doc.bounds());
    // A 4 × 2 image: opaque red, half-transparent green, opaque blue, fully transparent.
    let mut image = Vec::new();
    for _ in 0..2 {
        image.extend_from_slice(&[255, 0, 0, 255, 0, 255, 0, 128, 0, 0, 255, 255, 9, 9, 9, 0]);
    }
    doc.add_image_layer("foto.png", 4, 2, &image, 20, 10);
    assert_eq!((doc.layers().len(), doc.active_layer()), (2, 1));
    assert_eq!(doc.active().props.name, "foto.png");
    assert_eq!(pixel(&doc, 20, 10), [255, 0, 0, 255]);
    let green = pixel(&doc, 21, 11);
    assert!((126..=129).contains(&green[0]) && green[1] == 255, "{green:?}"); // 50 % over white
    assert_eq!(pixel(&doc, 22, 10), [0, 0, 255, 255]);
    assert_eq!(pixel(&doc, 23, 10), [255, 255, 255, 255]);
    assert_eq!(pixel(&doc, 50, 40), [0, 0, 0, 255]); // the drawing underneath is intact
    doc.undo();
    assert_eq!(doc.layers().len(), 1);
    assert_eq!(doc.render_rgba8(doc.bounds()), before);

    // An image hanging off the canvas is clipped rather than rejected.
    doc.add_image_layer("borde", 4, 2, &image, 98, -1);
    assert_eq!(pixel(&doc, 98, 0), [255, 0, 0, 255]);
}

/// A 40 × 20 opaque red block with its top-left corner at (30, 40).
fn doc_with_block() -> Document {
    let mut doc = Document::new(200, 200);
    let block: Vec<u8> = std::iter::repeat([255u8, 0, 0, 255]).take(40 * 20).flatten().collect();
    doc.add_image_layer("bloque", 40, 20, &block, 30, 40);
    doc
}

fn red(doc: &Document, x: i32, y: i32) -> bool {
    pixel(doc, x, y) == [255, 0, 0, 255]
}

#[test]
fn transform_scales_rotates_and_moves_a_layer() {
    let mut doc = doc_with_block();
    assert_eq!(doc.begin_transform(), Some(IRect::new(30, 40, 70, 60)));

    // Identity leaves every pixel exactly as it was.
    let before = doc.render_rgba8(doc.bounds());
    doc.set_transform(LayerTransform { scale: (1.0, 1.0), rotation: 0.0, offset: (0.0, 0.0) });
    assert_eq!(doc.render_rgba8(doc.bounds()), before);

    // Twice the size about its centre (50, 50): now 80 × 40, from (10, 30).
    doc.set_transform(LayerTransform { scale: (2.0, 2.0), rotation: 0.0, offset: (0.0, 0.0) });
    assert!(red(&doc, 12, 32) && red(&doc, 88, 68));
    assert!(!red(&doc, 8, 50) && !red(&doc, 50, 28) && !red(&doc, 92, 50));

    // A quarter turn makes the wide block tall: 20 × 40 around the same centre.
    doc.set_transform(LayerTransform { scale: (1.0, 1.0), rotation: std::f32::consts::FRAC_PI_2, offset: (0.0, 0.0) });
    assert!(red(&doc, 50, 32) && red(&doc, 50, 68) && red(&doc, 42, 50) && red(&doc, 58, 50));
    assert!(!red(&doc, 35, 50) && !red(&doc, 65, 50));

    // Each setting starts from the original, so nothing accumulates or blurs.
    doc.set_transform(LayerTransform { scale: (1.0, 1.0), rotation: 0.0, offset: (100.0, 80.0) });
    assert!(red(&doc, 131, 121) && red(&doc, 168, 138));
    assert!(!red(&doc, 50, 50));
    doc.end_stroke();
    assert!(!doc.is_transforming());

    // The whole transform is one undo step back to the starting picture.
    assert!(doc.undo());
    assert_eq!(doc.render_rgba8(doc.bounds()), before);
}

#[test]
fn transform_can_be_cancelled_and_needs_content() {
    let mut doc = doc_with_block();
    let before = doc.render_rgba8(doc.bounds());
    let revision = doc.revision();
    doc.begin_transform();
    doc.set_transform(LayerTransform { scale: (3.0, 0.5), rotation: 0.7, offset: (20.0, -10.0) });
    assert_ne!(doc.render_rgba8(doc.bounds()), before);
    doc.cancel_stroke();
    assert_eq!(doc.render_rgba8(doc.bounds()), before);
    assert_eq!(doc.revision(), revision);

    // Ending without changing anything adds no undo step.
    doc.begin_transform();
    doc.end_stroke();
    assert_eq!(doc.revision(), revision);

    // An empty layer has nothing to transform.
    doc.add_layer();
    assert_eq!(doc.begin_transform(), None);
    assert!(!doc.is_transforming());
}

#[test]
fn transform_keeps_soft_edges_when_rotated() {
    let mut doc = doc_with_block();
    doc.begin_transform();
    doc.set_transform(LayerTransform { scale: (1.5, 1.5), rotation: 0.5, offset: (40.0, 40.0) });
    doc.end_stroke();
    // Some edge pixels are partly covered rather than all-or-nothing.
    let layer = &doc.layers()[1];
    let partial = (0..200).flat_map(|y| (0..200).map(move |x| (x, y)))
        .filter(|&(x, y)| { let a = layer.pixel(x, y)[3]; a > 2000 && a < 30000 })
        .count();
    assert!(partial > 40, "expected anti-aliased edges, found {partial} partial pixels");
    // The area is close to 1.5² times the original 800 pixels.
    let area: f64 = (0..200).flat_map(|y| (0..200).map(move |x| (x, y)))
        .map(|(x, y)| layer.pixel(x, y)[3] as f64 / 32768.0).sum();
    assert!((area - 1800.0).abs() < 60.0, "area {area}");
}

fn rect_points(x0: f32, y0: f32, x1: f32, y1: f32) -> Vec<(f32, f32)> {
    vec![(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
}

#[test]
fn selections_cover_the_inside_of_their_shape() {
    let mut doc = Document::new(200, 150);
    doc.select_polygon(&rect_points(20.0, 30.0, 80.5, 90.0));
    let sel = doc.selection().unwrap();
    assert_eq!(sel.bounds, IRect::new(20, 30, 81, 90));
    assert_eq!((sel.coverage(20, 30), sel.coverage(79, 89), sel.coverage(19, 50), sel.coverage(50, 90)), (255, 255, 0, 0));
    assert!((120..=135).contains(&sel.coverage(80, 50)), "half-covered edge: {}", sel.coverage(80, 50));

    // A circle of radius 40: area close to πr², smooth edge, outside empty.
    let circle: Vec<(f32, f32)> = (0..96)
        .map(|i| { let a = i as f32 / 96.0 * std::f32::consts::TAU; (100.0 + 40.0 * a.cos(), 75.0 + 40.0 * a.sin()) })
        .collect();
    doc.select_polygon(&circle);
    let sel = doc.selection().unwrap();
    let area: f64 = sel.mask.iter().map(|&m| m as f64 / 255.0).sum();
    assert!((area - std::f64::consts::PI * 1600.0).abs() < 30.0, "area {area}");
    assert_eq!((sel.coverage(100, 75), sel.coverage(100, 30), sel.coverage(140, 110)), (255, 0, 0));

    // A shape partly off the canvas is clipped; one fully off clears the selection.
    doc.select_polygon(&rect_points(-50.0, -50.0, 10.0, 10.0));
    assert_eq!(doc.selection().unwrap().bounds, IRect::new(0, 0, 10, 10));
    let stamp = doc.selection_stamp();
    doc.select_polygon(&rect_points(500.0, 500.0, 600.0, 600.0));
    assert!(doc.selection().is_none());
    assert_ne!(doc.selection_stamp(), stamp);
    doc.select_all();
    assert_eq!(doc.selection().unwrap().bounds, doc.bounds());
    doc.deselect();
    assert!(doc.selection().is_none());
}

#[test]
fn painting_stays_inside_the_selection() {
    let mut doc = Document::new(200, 200);
    doc.select_polygon(&rect_points(50.0, 50.0, 150.0, 150.0));
    line(&mut doc, &hard_brush(40.0), [255, 0, 0], false, (10.0, 100.0), (190.0, 100.0));
    assert_eq!(pixel(&doc, 100, 100), [255, 0, 0, 255]);
    assert_eq!(pixel(&doc, 30, 100), [255, 255, 255, 255]);
    assert_eq!(pixel(&doc, 170, 100), [255, 255, 255, 255]);
    // The eraser and the fill respect it too.
    doc.deselect();
    line(&mut doc, &hard_brush(40.0), [255, 0, 0], false, (10.0, 100.0), (190.0, 100.0));
    doc.select_polygon(&rect_points(0.0, 0.0, 100.0, 200.0));
    line(&mut doc, &hard_brush(60.0), [0, 0, 0], true, (10.0, 100.0), (190.0, 100.0));
    assert_eq!(pixel(&doc, 50, 100), [255, 255, 255, 255]);
    assert_eq!(pixel(&doc, 150, 100), [255, 0, 0, 255]);
    assert!(doc.flood_fill(50, 20, [0, 0, 255], 0.1, false));
    assert_eq!(pixel(&doc, 50, 20), [0, 0, 255, 255]);
    assert_eq!(pixel(&doc, 150, 20), [255, 255, 255, 255]);
    assert!(!doc.flood_fill(150, 20, [0, 0, 255], 0.1, false)); // clicked outside the selection
}

#[test]
fn transform_moves_only_the_selected_part() {
    let mut doc = doc_with_block(); // red block x 30..70, y 40..60
    let before = doc.render_rgba8(doc.bounds());
    doc.select_polygon(&rect_points(50.0, 0.0, 200.0, 200.0)); // its right half
    assert_eq!(doc.begin_transform(), Some(IRect::new(50, 40, 70, 60)));
    doc.set_transform(LayerTransform { scale: (1.0, 1.0), rotation: 0.0, offset: (60.0, 100.0) });
    assert!(red(&doc, 35, 50) && red(&doc, 49, 50)); // the left half stays
    assert!(!red(&doc, 55, 50) && !red(&doc, 65, 50)); // the right half left
    assert!(red(&doc, 115, 150) && red(&doc, 125, 150)); // and landed here
    doc.end_stroke();
    assert!(doc.selection().is_none()); // the outline no longer matched
    assert!(doc.undo());
    assert_eq!(doc.render_rgba8(doc.bounds()), before);

    // A selection over empty space has nothing to transform.
    doc.select_polygon(&rect_points(120.0, 120.0, 180.0, 180.0));
    assert_eq!(doc.begin_transform(), None);
}

#[test]
fn flipping_a_layer_or_a_selection() {
    let mut doc = doc_with_block(); // red block x 30..70, y 40..60 on a 200 × 200 canvas
    let before = doc.render_rgba8(doc.bounds());
    doc.flip(true);
    assert!(red(&doc, 135, 50) && red(&doc, 165, 50) && !red(&doc, 50, 50)); // now x 130..170
    doc.flip(false);
    assert!(red(&doc, 150, 150) && !red(&doc, 150, 50)); // and y 140..160
    doc.flip(false);
    doc.flip(true);
    assert_eq!(doc.render_rgba8(doc.bounds()), before); // two flips each way cancel out
    for _ in 0..4 {
        assert!(doc.undo());
    }
    assert_eq!(doc.render_rgba8(doc.bounds()), before);

    // With a selection, only its contents flip, within its bounds.
    doc.select_polygon(&rect_points(20.0, 30.0, 120.0, 70.0)); // the block sits in its left part
    doc.flip(true);
    assert!(!red(&doc, 40, 50), "the block left its old place");
    assert!(red(&doc, 75, 50) && red(&doc, 105, 50), "and is mirrored to x 70..110");
    assert_eq!(doc.selection().unwrap().bounds, IRect::new(20, 30, 120, 70));
    doc.undo();
    assert_eq!(doc.render_rgba8(doc.bounds()), before);
}

#[test]
fn deleting_clears_only_the_selection() {
    let mut doc = doc_with_block();
    let revision = doc.revision();
    doc.clear_selection_contents(); // nothing selected: nothing happens
    assert_eq!(doc.revision(), revision);
    doc.select_polygon(&rect_points(0.0, 0.0, 50.0, 200.0));
    doc.clear_selection_contents();
    assert!(!red(&doc, 40, 50) && red(&doc, 60, 50));
    doc.undo();
    assert!(red(&doc, 40, 50));
}

#[test]
fn clipped_layers_show_only_over_their_base() {
    let mut doc = Document::new(200, 200);
    line(&mut doc, &hard_brush(60.0), [255, 0, 0], false, (40.0, 100.0), (160.0, 100.0)); // base: red bar
    doc.add_layer();
    line(&mut doc, &hard_brush(40.0), [0, 0, 255], false, (100.0, 20.0), (100.0, 180.0)); // blue stripe across it
    assert_eq!(pixel(&doc, 100, 30), [0, 0, 255, 255]);

    let mut props = doc.active().props.clone();
    props.clip = true;
    doc.set_layer_props(1, props, false);
    assert_eq!(pixel(&doc, 100, 100), [0, 0, 255, 255]); // over the bar: visible
    assert_eq!(pixel(&doc, 100, 30), [255, 255, 255, 255]); // outside it: clipped away
    assert_eq!(pixel(&doc, 60, 100), [255, 0, 0, 255]);

    // Hiding the base hides what is clipped to it.
    let mut base = doc.layers()[0].props.clone();
    base.visible = false;
    doc.set_layer_props(0, base, false);
    assert_eq!(pixel(&doc, 100, 100), [255, 255, 255, 255]);
    doc.undo();

    // The flag survives saving, and merging down keeps the clipped look.
    let mut bytes = Vec::new();
    doc.write_to(&mut bytes).unwrap();
    let loaded = Document::read_from(&mut bytes.as_slice()).unwrap();
    assert!(loaded.layers()[1].props.clip && !loaded.layers()[0].props.clip);
    assert_eq!(loaded.render_rgba8(loaded.bounds()), doc.render_rgba8(doc.bounds()));
    let stacked = doc.render_rgba8(doc.bounds());
    assert!(doc.merge_down());
    assert_eq!(doc.render_rgba8(doc.bounds()), stacked);
}

#[test]
fn copying_takes_the_selected_pixels() {
    let mut doc = doc_with_block(); // red block x 30..70, y 40..60
    let (rect, rgba) = doc.copy_pixels().unwrap();
    assert_eq!(rect, IRect::new(30, 40, 70, 60));
    assert_eq!(rgba.len(), 40 * 20 * 4);
    assert!(rgba.chunks(4).all(|p| p == [255, 0, 0, 255]));

    doc.select_polygon(&rect_points(60.0, 30.0, 90.0, 70.0)); // overlaps its right end
    let (rect, rgba) = doc.copy_pixels().unwrap();
    assert_eq!(rect, IRect::new(60, 30, 90, 70));
    let at = |x: i32, y: i32| { let i = ((y - 30) * 30 + (x - 60)) as usize * 4; [rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]] };
    assert_eq!(at(65, 50), [255, 0, 0, 255]);
    assert_eq!(at(80, 50), [0, 0, 0, 0]);

    // Pasting it back in place reproduces those pixels on a new layer.
    doc.add_image_layer("copia", rect.width(), rect.height(), &rgba, rect.x0, rect.y0);
    assert_eq!(doc.layers()[2].pixel(65, 50), doc.layers()[1].pixel(65, 50));
    assert_eq!(doc.layers()[2].pixel(40, 50), [0; 4]);

    doc.select_polygon(&rect_points(120.0, 120.0, 180.0, 180.0)); // nothing there
    assert!(doc.copy_pixels().is_none());
}

#[test]
fn references_are_saved_with_the_project_but_not_drawn() {
    let mut doc = Document::new(120, 90);
    line(&mut doc, &hard_brush(10.0), [0, 0, 0], false, (10.0, 45.0), (110.0, 45.0));
    let picture = doc.render_rgba8(doc.bounds());
    let mut plain = Vec::new();
    doc.write_to(&mut plain).unwrap();

    let revision = doc.revision();
    let pixels: Vec<u8> = (0..5 * 3).flat_map(|i| [i as u8 * 10, 200, 30, 255]).collect();
    doc.add_reference(Reference { name: "pose ✦".into(), width: 5, height: 3, rgba: std::sync::Arc::new(pixels.clone()) });
    doc.add_reference(Reference { name: "manos".into(), width: 1, height: 1, rgba: std::sync::Arc::new(vec![1, 2, 3, 4]) });
    assert_ne!(doc.revision(), revision); // counts as an unsaved change
    assert_eq!(doc.render_rgba8(doc.bounds()), picture); // but never shows in the picture

    let mut bytes = Vec::new();
    doc.write_to(&mut bytes).unwrap();
    let loaded = Document::read_from(&mut bytes.as_slice()).unwrap();
    assert_eq!(loaded.references().len(), 2);
    let first = &loaded.references()[0];
    assert_eq!((first.name.as_str(), first.width, first.height), ("pose ✦", 5, 3));
    assert_eq!(*first.rgba, pixels);
    assert_eq!(loaded.render_rgba8(loaded.bounds()), picture);

    // Files without references still load, and a file with references starts
    // with the same bytes as one without, so older versions can read it.
    assert!(Document::read_from(&mut plain.as_slice()).unwrap().references().is_empty());
    assert_eq!(&bytes[..plain.len()], &plain[..]);

    doc.remove_reference(0);
    assert_eq!(doc.references()[0].name, "manos");
    // A damaged trailer is rejected rather than half-read.
    assert!(Document::read_from(&mut &bytes[..bytes.len() - 3]).is_err());
}
