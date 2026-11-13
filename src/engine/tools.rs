//! Tools other than the brush: flood fill, smudge and moving a layer.

use std::collections::HashMap;
use std::sync::Arc;

use super::brush::BrushSettings;
use super::document::Document;
use super::layer::{IRect, Layer, Px, Tile, ONE, TILE};
use super::stroke::StrokePoint;

/// Remembers the original contents of every tile an edit touches.
#[derive(Default)]
pub(super) struct TileEdit {
    saved: HashMap<usize, Option<Arc<Tile>>>,
}

impl TileEdit {
    fn tile_mut<'a>(&mut self, layer: &'a mut Layer, idx: usize) -> &'a mut Tile {
        self.saved.entry(idx).or_insert_with(|| layer.tiles[idx].clone());
        Arc::make_mut(layer.tiles[idx].get_or_insert_with(|| Arc::new(Tile::empty())))
    }

    fn into_saved(self) -> Vec<(usize, Option<Arc<Tile>>)> {
        self.saved.into_iter().collect()
    }

    /// Puts every touched tile back.
    fn revert(self, layer: &mut Layer) {
        for (idx, orig) in self.saved {
            layer.tiles[idx] = orig;
        }
    }
}

// --- Flood fill -------------------------------------------------------------

impl Document {
    /// Fills the area around (`x`, `y`) on the active layer with `color`.
    ///
    /// The area is every connected pixel within `tolerance` (0..=1) of the one
    /// clicked, judged on the active layer alone or, with `all_layers`, on the
    /// whole picture. The fill also reaches one pixel under the area's border, so
    /// no light fringe is left along anti-aliased line art. Returns whether
    /// anything was filled.
    pub fn flood_fill(&mut self, x: i32, y: i32, color: [u8; 3], tolerance: f32, all_layers: bool) -> bool {
        self.end_stroke();
        let (w, h) = (self.width, self.height);
        if x < 0 || y < 0 || x as usize >= w || y as usize >= h {
            return false;
        }
        let li = self.active_layer;
        if !self.layers[li].props.visible {
            return false;
        }

        // What the fill looks at: 8-bit premultiplied RGBA per pixel.
        let reference: Vec<[u8; 4]> = if all_layers {
            self.render_rgba8(self.bounds()).chunks_exact(4).map(|p| [p[0], p[1], p[2], p[3]]).collect()
        } else {
            let layer = &self.layers[li];
            let mut pixels = vec![[0u8; 4]; w * h];
            for ty in 0..layer.tiles_y {
                for tx in 0..layer.tiles_x {
                    let Some(tile) = &layer.tiles[layer.tile_index(tx, ty)] else { continue };
                    for py in 0..TILE.min(h - ty * TILE) {
                        let row = (ty * TILE + py) * w + tx * TILE;
                        for px in 0..TILE.min(w - tx * TILE) {
                            pixels[row + px] = tile.0[py * TILE + px].map(|c| (c >> 7).min(255) as u8);
                        }
                    }
                }
            }
            pixels
        };
        let seed = reference[y as usize * w + x as usize];
        let limit = (tolerance.clamp(0.0, 1.0) * 255.0).round() as i32;
        let matches = |p: [u8; 4]| (0..4).all(|c| (p[c] as i32 - seed[c] as i32).abs() <= limit);

        // 0 = untouched, 1 = border (fill goes underneath), 2 = inside the area.
        let mut mask = vec![0u8; w * h];
        let mut stack = vec![(x as usize, y as usize)];
        mask[y as usize * w + x as usize] = 2;
        let (mut x0, mut y0, mut x1, mut y1) = (x as usize, y as usize, x as usize, y as usize);
        while let Some((cx, cy)) = stack.pop() {
            let mut visit = |nx: usize, ny: usize| {
                let i = ny * w + nx;
                if mask[i] == 2 {
                    return;
                }
                if matches(reference[i]) {
                    mask[i] = 2;
                    stack.push((nx, ny));
                } else {
                    mask[i] = 1;
                }
                x0 = x0.min(nx);
                x1 = x1.max(nx);
                y0 = y0.min(ny);
                y1 = y1.max(ny);
            };
            if cx > 0 {
                visit(cx - 1, cy);
            }
            if cx + 1 < w {
                visit(cx + 1, cy);
            }
            if cy > 0 {
                visit(cx, cy - 1);
            }
            if cy + 1 < h {
                visit(cx, cy + 1);
            }
        }

        let paint = color.map(|v| (v as u32 * ONE + 127) / 255);
        let layer = &mut self.layers[li];
        let lock = layer.props.alpha_lock;
        let mut edit = TileEdit::default();
        for ty in y0 / TILE..=y1 / TILE {
            for tx in x0 / TILE..=x1 / TILE {
                let (ox, oy) = (tx * TILE, ty * TILE);
                let rows = oy.max(y0)..(oy + TILE).min(y1 + 1);
                let cols = ox.max(x0)..(ox + TILE).min(x1 + 1);
                if !rows.clone().any(|py| cols.clone().any(|px| mask[py * w + px] != 0)) {
                    continue;
                }
                let idx = layer.tile_index(tx, ty);
                if lock && layer.tiles[idx].is_none() {
                    continue; // nothing to recolour
                }
                let tile = edit.tile_mut(layer, idx);
                for py in rows.clone() {
                    for px in cols.clone() {
                        let kind = mask[py * w + px];
                        if kind == 0 {
                            continue;
                        }
                        let d = &mut tile.0[(py - oy) * TILE + (px - ox)];
                        let own = d[3] as u32;
                        if lock {
                            // Recolour what is there, keeping its transparency.
                            if kind == 2 {
                                for c in 0..3 {
                                    d[c] = ((paint[c] * own + ONE / 2) >> 15) as u16;
                                }
                            }
                        } else if kind == 2 {
                            *d = [paint[0] as u16, paint[1] as u16, paint[2] as u16, ONE as u16];
                        } else {
                            // Border: paint behind the existing pixel.
                            let gap = ONE - own;
                            for c in 0..3 {
                                d[c] = (d[c] as u32 + ((paint[c] * gap + ONE / 2) >> 15)) as u16;
                            }
                            d[3] = ONE as u16;
                        }
                    }
                }
            }
        }
        let saved = edit.into_saved();
        let changed = !saved.is_empty();
        self.commit_tiles(li, saved);
        self.mark_dirty(IRect::new(x0 as i32, y0 as i32, x1 as i32 + 1, y1 as i32 + 1));
        changed
    }
}

// --- Smudge -----------------------------------------------------------------

pub(super) struct SmudgeStroke {
    layer: usize,
    brush: BrushSettings,
    last: StrokePoint,
    edit: TileEdit,
}

impl Document {
    /// Starts dragging paint around on the active layer. The brush's size and
    /// hardness shape the tip; its flow is the smudge strength.
    pub fn begin_smudge(&mut self, brush: &BrushSettings, p: StrokePoint) {
        self.end_stroke();
        self.smudge = Some(SmudgeStroke {
            layer: self.active_layer,
            brush: brush.clone(),
            last: p,
            edit: TileEdit::default(),
        });
    }

    pub(super) fn smudge_to(&mut self, p: StrokePoint) {
        let Some(mut smudge) = self.smudge.take() else { return };
        let layer = &mut self.layers[smudge.layer];
        let mut dirty = IRect::new(0, 0, 0, 0);
        loop {
            let (dx, dy) = (p.x - smudge.last.x, p.y - smudge.last.y);
            let dist = (dx * dx + dy * dy).sqrt();
            let radius = (smudge.brush.diameter_at(p.pressure) * 0.5).max(1.0);
            // Short steps keep the smear continuous.
            let step = (radius * 0.2).clamp(1.0, 6.0);
            if dist < step {
                break;
            }
            let (sx, sy) = (dx / dist * step, dy / dist * step);
            let t = step / dist;
            let to = StrokePoint::new(
                smudge.last.x + sx,
                smudge.last.y + sy,
                smudge.last.pressure + (p.pressure - smudge.last.pressure) * t,
            );
            dirty = dirty.union(&smudge_step(layer, &mut smudge.edit, &smudge.brush, to, sx, sy));
            smudge.last = to;
        }
        self.smudge = Some(smudge);
        self.mark_dirty(dirty);
    }

    pub(super) fn end_smudge(&mut self) {
        if let Some(smudge) = self.smudge.take() {
            self.commit_tiles(smudge.layer, smudge.edit.into_saved());
        }
    }

    pub(super) fn cancel_smudge(&mut self) {
        if let Some(smudge) = self.smudge.take() {
            smudge.edit.revert(&mut self.layers[smudge.layer]);
            self.mark_dirty(self.bounds());
        }
    }
}

/// Pulls the pixels around `at` one step along (`sx`, `sy`): each pixel under
/// the tip moves towards the colour that was one step behind it.
fn smudge_step(layer: &mut Layer, edit: &mut TileEdit, brush: &BrushSettings, at: StrokePoint, sx: f32, sy: f32) -> IRect {
    let radius = (brush.diameter_at(at.pressure) * 0.5).max(1.0);
    let strength = brush.flow_at(at.pressure).clamp(0.0, 1.0);
    let falloff = (radius * (1.0 - brush.hardness.clamp(0.0, 1.0))).max(1.0);
    let outer = radius + 0.5;
    let rect = IRect::new(
        (at.x - outer).floor() as i32,
        (at.y - outer).floor() as i32,
        (at.x + outer).ceil() as i32 + 1,
        (at.y + outer).ceil() as i32 + 1,
    )
    .intersect(&layer.bounds());
    if rect.is_empty() || strength <= 0.0 {
        return IRect::new(0, 0, 0, 0);
    }
    // Snapshot of the area plus a margin for sampling one step back.
    let margin = sx.abs().max(sy.abs()).ceil() as i32 + 2;
    let src = IRect::new(rect.x0 - margin, rect.y0 - margin, rect.x1 + margin, rect.y1 + margin);
    let sw = src.width();
    let mut buffer = vec![[0u16; 4]; sw * src.height()];
    for y in src.y0..src.y1 {
        for x in src.x0..src.x1 {
            buffer[(y - src.y0) as usize * sw + (x - src.x0) as usize] = layer.pixel(x, y);
        }
    }
    let at_buffer = |x: i32, y: i32| buffer[(y - src.y0) as usize * sw + (x - src.x0) as usize];
    let sample = |fx: f32, fy: f32| -> [f32; 4] {
        let (x0, y0) = (fx.floor(), fy.floor());
        let (tx, ty) = (fx - x0, fy - y0);
        let (x0, y0) = (x0 as i32, y0 as i32);
        let (a, b, c, d) = (at_buffer(x0, y0), at_buffer(x0 + 1, y0), at_buffer(x0, y0 + 1), at_buffer(x0 + 1, y0 + 1));
        std::array::from_fn(|i| {
            let top = a[i] as f32 + (b[i] as f32 - a[i] as f32) * tx;
            let bottom = c[i] as f32 + (d[i] as f32 - c[i] as f32) * tx;
            top + (bottom - top) * ty
        })
    };

    let lock = layer.props.alpha_lock;
    for y in rect.y0..rect.y1 {
        for x in rect.x0..rect.x1 {
            let (fx, fy) = (x as f32 + 0.5 - at.x, y as f32 + 0.5 - at.y);
            let d = (fx * fx + fy * fy).sqrt();
            if d >= outer {
                continue;
            }
            let mut weight = ((outer - d) / falloff).min(1.0);
            weight = weight * weight * (3.0 - 2.0 * weight) * strength;
            let here = at_buffer(x, y);
            let behind = sample(x as f32 - sx, y as f32 - sy);
            if here == [0; 4] && behind[3] < 0.5 {
                continue; // transparent stays transparent; do not allocate tiles
            }
            let mut out: Px = std::array::from_fn(|i| {
                (here[i] as f32 + (behind[i] - here[i] as f32) * weight).round().clamp(0.0, ONE as f32) as u16
            });
            if lock {
                out[3] = here[3];
            }
            for c in 0..3 {
                out[c] = out[c].min(out[3]);
            }
            if out != here {
                let (ux, uy) = (x as usize, y as usize);
                let idx = layer.tile_index(ux / TILE, uy / TILE);
                edit.tile_mut(layer, idx).0[(uy % TILE) * TILE + ux % TILE] = out;
            }
        }
    }
    rect
}

// --- Moving a layer ---------------------------------------------------------

pub(super) struct MoveSession {
    layer: usize,
    original: Vec<Option<Arc<Tile>>>,
    offset: (i32, i32),
}

impl Document {
    /// Starts moving the active layer's contents.
    pub fn begin_layer_move(&mut self) {
        self.end_stroke();
        let layer = self.active_layer;
        self.moving = Some(MoveSession { layer, original: self.layers[layer].tiles.clone(), offset: (0, 0) });
    }

    /// Places the layer `dx`, `dy` pixels from where the move began. Pixels
    /// pushed off the canvas come back if the layer is dragged back before the
    /// move ends.
    pub fn move_layer_to(&mut self, dx: i32, dy: i32) {
        let Some(session) = &mut self.moving else { return };
        if session.offset == (dx, dy) {
            return;
        }
        session.offset = (dx, dy);
        let layer = &mut self.layers[session.layer];
        layer.tiles = shifted(layer, &session.original, dx, dy);
        self.mark_dirty(self.bounds());
    }

    pub(super) fn end_layer_move(&mut self) {
        if let Some(session) = self.moving.take() {
            if session.offset == (0, 0) {
                return;
            }
            let now = &self.layers[session.layer].tiles;
            let saved: Vec<_> = session
                .original
                .into_iter()
                .enumerate()
                .filter(|(i, orig)| orig.is_some() || now[*i].is_some())
                .collect();
            self.commit_tiles(session.layer, saved);
        }
    }

    pub(super) fn cancel_layer_move(&mut self) {
        if let Some(session) = self.moving.take() {
            self.layers[session.layer].tiles = session.original;
            self.mark_dirty(self.bounds());
        }
    }
}

/// The tile table of `layer` with `original` contents moved by (`dx`, `dy`).
fn shifted(layer: &Layer, original: &[Option<Arc<Tile>>], dx: i32, dy: i32) -> Vec<Option<Arc<Tile>>> {
    let t = TILE as i32;
    let mut out: Vec<Option<Arc<Tile>>> = vec![None; original.len()];
    if dx % t == 0 && dy % t == 0 {
        // Whole-tile moves just rearrange the table.
        let (sx, sy) = (dx / t, dy / t);
        for ty in 0..layer.tiles_y as i32 {
            for tx in 0..layer.tiles_x as i32 {
                let (nx, ny) = (tx + sx, ty + sy);
                if nx >= 0 && ny >= 0 && (nx as usize) < layer.tiles_x && (ny as usize) < layer.tiles_y {
                    out[layer.tile_index(nx as usize, ny as usize)] =
                        original[layer.tile_index(tx as usize, ty as usize)].clone();
                }
            }
        }
        trim_edges(layer, &mut out);
        return out;
    }
    let bounds = layer.bounds();
    for ty in 0..layer.tiles_y {
        for tx in 0..layer.tiles_x {
            let Some(src) = &original[layer.tile_index(tx, ty)] else { continue };
            let (ox, oy) = ((tx * TILE) as i32, (ty * TILE) as i32);
            // Where this tile's pixels land, clipped to the canvas.
            let dest = IRect::new(ox + dx, oy + dy, ox + dx + t, oy + dy + t).intersect(&bounds);
            if dest.is_empty() {
                continue;
            }
            for dty in dest.y0 / t..=(dest.y1 - 1) / t {
                for dtx in dest.x0 / t..=(dest.x1 - 1) / t {
                    let part = dest.intersect(&IRect::new(dtx * t, dty * t, (dtx + 1) * t, (dty + 1) * t));
                    let idx = layer.tile_index(dtx as usize, dty as usize);
                    let tile = Arc::make_mut(out[idx].get_or_insert_with(|| Arc::new(Tile::empty())));
                    let width = part.width();
                    for y in part.y0..part.y1 {
                        let from = ((y - dy - oy) * t + (part.x0 - dx - ox)) as usize;
                        let to = ((y - dty * t) * t + (part.x0 - dtx * t)) as usize;
                        tile.0[to..to + width].copy_from_slice(&src.0[from..from + width]);
                    }
                }
            }
        }
    }
    out
}

/// Clears pixels that whole-tile moves carried past the right or bottom edge of
/// the canvas (tiles on those edges extend beyond it).
fn trim_edges(layer: &Layer, tiles: &mut [Option<Arc<Tile>>]) {
    let (w, h) = (layer.width, layer.height);
    for ty in 0..layer.tiles_y {
        for tx in 0..layer.tiles_x {
            let (ox, oy) = (tx * TILE, ty * TILE);
            if ox + TILE <= w && oy + TILE <= h {
                continue;
            }
            let Some(tile) = &mut tiles[ty * layer.tiles_x + tx] else { continue };
            let tile = Arc::make_mut(tile);
            for py in 0..TILE {
                for px in 0..TILE {
                    if ox + px >= w || oy + py >= h {
                        tile.0[py * TILE + px] = [0; 4];
                    }
                }
            }
        }
    }
}

// --- Inserting images -------------------------------------------------------

impl Document {
    /// Adds a layer above the active one holding an image, with its top-left
    /// corner at (`x`, `y`) on the canvas; parts outside the canvas are dropped.
    /// `rgba` is 8-bit, not premultiplied, row-major, `width * height * 4` bytes.
    pub fn add_image_layer(&mut self, name: &str, width: usize, height: usize, rgba: &[u8], x: i32, y: i32) {
        assert_eq!(rgba.len(), width * height * 4, "image buffer does not match its size");
        self.add_layer();
        let li = self.active_layer;
        let layer = &mut self.layers[li];
        layer.props.name = name.to_owned();
        let area = IRect::new(x, y, x + width as i32, y + height as i32).intersect(&layer.bounds());
        for cy in area.y0..area.y1 {
            for cx in area.x0..area.x1 {
                let i = ((cy - y) as usize * width + (cx - x) as usize) * 4;
                let alpha = rgba[i + 3] as u32;
                if alpha == 0 {
                    continue;
                }
                let a15 = (alpha * ONE + 127) / 255;
                let px: Px = [
                    (((rgba[i] as u32 * ONE + 127) / 255 * a15 + ONE / 2) >> 15) as u16,
                    (((rgba[i + 1] as u32 * ONE + 127) / 255 * a15 + ONE / 2) >> 15) as u16,
                    (((rgba[i + 2] as u32 * ONE + 127) / 255 * a15 + ONE / 2) >> 15) as u16,
                    a15 as u16,
                ];
                let (ux, uy) = (cx as usize, cy as usize);
                let idx = layer.tile_index(ux / TILE, uy / TILE);
                let tile = Arc::make_mut(layer.tiles[idx].get_or_insert_with(|| Arc::new(Tile::empty())));
                tile.0[(uy % TILE) * TILE + ux % TILE] = px;
            }
        }
        self.mark_dirty(self.bounds());
    }
}

// --- Scaling and rotating a layer -------------------------------------------

/// Placement of a layer's contents relative to where they were: scaled and
/// rotated about the centre of their bounding box, then shifted.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayerTransform {
    pub scale: (f32, f32),
    /// Clockwise, in radians.
    pub rotation: f32,
    pub offset: (f32, f32),
}

impl LayerTransform {
    pub const IDENTITY: LayerTransform = LayerTransform { scale: (1.0, 1.0), rotation: 0.0, offset: (0.0, 0.0) };
}

pub(super) struct TransformSession {
    layer: usize,
    original: Vec<Option<Arc<Tile>>>,
    bounds: IRect,
    current: LayerTransform,
}

impl Document {
    /// The transform in progress is still open.
    pub fn is_transforming(&self) -> bool {
        self.transforming.is_some()
    }

    /// Starts transforming the active layer. Returns the bounding box of its
    /// contents, or `None` (and starts nothing) if the layer is empty.
    pub fn begin_transform(&mut self) -> Option<IRect> {
        self.end_stroke();
        let li = self.active_layer;
        let bounds = content_bounds(&self.layers[li])?;
        self.transforming = Some(TransformSession {
            layer: li,
            original: self.layers[li].tiles.clone(),
            bounds,
            current: LayerTransform::IDENTITY,
        });
        Some(bounds)
    }

    /// Redraws the layer from its original contents with `transform` applied.
    pub fn set_transform(&mut self, transform: LayerTransform) {
        let Some(session) = &mut self.transforming else { return };
        if session.current == transform {
            return;
        }
        session.current = transform;
        let layer = &mut self.layers[session.layer];
        layer.tiles = if transform == LayerTransform::IDENTITY {
            session.original.clone()
        } else {
            transformed(layer, &session.original, session.bounds, transform)
        };
        self.mark_dirty(self.bounds());
    }

    pub(super) fn end_transform(&mut self) {
        if let Some(session) = self.transforming.take() {
            if session.current == LayerTransform::IDENTITY {
                return;
            }
            let now = &self.layers[session.layer].tiles;
            let saved: Vec<_> = session
                .original
                .into_iter()
                .enumerate()
                .filter(|(i, orig)| orig.is_some() || now[*i].is_some())
                .collect();
            self.commit_tiles(session.layer, saved);
        }
    }

    pub(super) fn cancel_transform(&mut self) {
        if let Some(session) = self.transforming.take() {
            self.layers[session.layer].tiles = session.original;
            self.mark_dirty(self.bounds());
        }
    }
}

/// Smallest rectangle containing every non-transparent pixel of the layer.
fn content_bounds(layer: &Layer) -> Option<IRect> {
    let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0usize, 0usize);
    for ty in 0..layer.tiles_y {
        for tx in 0..layer.tiles_x {
            let Some(tile) = &layer.tiles[layer.tile_index(tx, ty)] else { continue };
            for py in 0..TILE {
                for px in 0..TILE {
                    if tile.0[py * TILE + px][3] != 0 {
                        let (x, y) = (tx * TILE + px, ty * TILE + py);
                        x0 = x0.min(x);
                        x1 = x1.max(x);
                        y0 = y0.min(y);
                        y1 = y1.max(y);
                    }
                }
            }
        }
    }
    (x0 != usize::MAX).then(|| IRect::new(x0 as i32, y0 as i32, x1 as i32 + 1, y1 as i32 + 1))
}

/// The tile table of `layer` with `original` contents scaled, rotated and
/// shifted. Each destination pixel samples the original with bilinear
/// filtering; tiles are computed on several threads.
fn transformed(layer: &Layer, original: &[Option<Arc<Tile>>], bounds: IRect, t: LayerTransform) -> Vec<Option<Arc<Tile>>> {
    let (sx, sy) = (t.scale.0.abs().max(1e-4), t.scale.1.abs().max(1e-4));
    let (sin, cos) = t.rotation.sin_cos();
    let centre = ((bounds.x0 + bounds.x1) as f32 * 0.5, (bounds.y0 + bounds.y1) as f32 * 0.5);
    let moved = (centre.0 + t.offset.0, centre.1 + t.offset.1);
    let half = (bounds.width() as f32 * 0.5, bounds.height() as f32 * 0.5);

    // Where the box ends up, to know which tiles can receive pixels.
    let (mut dx0, mut dy0, mut dx1, mut dy1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for (cx, cy) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
        let (lx, ly) = (cx * half.0 * sx, cy * half.1 * sy);
        let (x, y) = (moved.0 + lx * cos - ly * sin, moved.1 + lx * sin + ly * cos);
        dx0 = dx0.min(x);
        dx1 = dx1.max(x);
        dy0 = dy0.min(y);
        dy1 = dy1.max(y);
    }
    let dest = IRect::new(dx0.floor() as i32 - 1, dy0.floor() as i32 - 1, dx1.ceil() as i32 + 1, dy1.ceil() as i32 + 1)
        .intersect(&layer.bounds());
    let mut out: Vec<Option<Arc<Tile>>> = vec![None; original.len()];
    if dest.is_empty() {
        return out;
    }

    let (width, height, tiles_x) = (layer.width as i32, layer.height as i32, layer.tiles_x);
    let fetch = |x: i32, y: i32| -> Px {
        if x < 0 || y < 0 || x >= width || y >= height {
            return [0; 4];
        }
        let (x, y) = (x as usize, y as usize);
        match &original[(y / TILE) * tiles_x + x / TILE] {
            Some(tile) => tile.0[(y % TILE) * TILE + x % TILE],
            None => [0; 4],
        }
    };
    let render_tile = |tx: usize, ty: usize| -> Option<Tile> {
        let mut tile = Tile::empty();
        let mut any = false;
        let (ox, oy) = ((tx * TILE) as i32, (ty * TILE) as i32);
        for py in (dest.y0.max(oy) - oy)..(dest.y1.min(oy + TILE as i32) - oy) {
            for px in (dest.x0.max(ox) - ox)..(dest.x1.min(ox + TILE as i32) - ox) {
                // Destination pixel centre, back through the transform.
                let (rx, ry) = ((ox + px) as f32 + 0.5 - moved.0, (oy + py) as f32 + 0.5 - moved.1);
                let (lx, ly) = ((rx * cos + ry * sin) / sx, (-rx * sin + ry * cos) / sy);
                if lx.abs() > half.0 + 1.0 || ly.abs() > half.1 + 1.0 {
                    continue;
                }
                let (fx, fy) = (centre.0 + lx - 0.5, centre.1 + ly - 0.5);
                let (x0, y0) = (fx.floor(), fy.floor());
                let (wx, wy) = (fx - x0, fy - y0);
                let (x0, y0) = (x0 as i32, y0 as i32);
                let (a, b, c, d) = (fetch(x0, y0), fetch(x0 + 1, y0), fetch(x0, y0 + 1), fetch(x0 + 1, y0 + 1));
                if a[3] | b[3] | c[3] | d[3] == 0 {
                    continue;
                }
                let px_out: Px = std::array::from_fn(|i| {
                    let top = a[i] as f32 + (b[i] as f32 - a[i] as f32) * wx;
                    let bottom = c[i] as f32 + (d[i] as f32 - c[i] as f32) * wx;
                    (top + (bottom - top) * wy).round().clamp(0.0, ONE as f32) as u16
                });
                if px_out[3] != 0 {
                    tile.0[py as usize * TILE + px as usize] = px_out;
                    any = true;
                }
            }
        }
        any.then_some(tile)
    };

    let targets: Vec<(usize, usize)> = (dest.y0 as usize / TILE..=(dest.y1 as usize - 1) / TILE)
        .flat_map(|ty| (dest.x0 as usize / TILE..=(dest.x1 as usize - 1) / TILE).map(move |tx| (tx, ty)))
        .collect();
    let workers = std::thread::available_parallelism().map_or(1, |n| n.get()).min(8);
    let chunk = targets.len().div_ceil(workers).max(1);
    let rendered: Vec<Vec<(usize, Tile)>> = std::thread::scope(|scope| {
        let handles: Vec<_> = targets
            .chunks(chunk)
            .map(|part| {
                let render_tile = &render_tile;
                scope.spawn(move || {
                    part.iter()
                        .filter_map(|&(tx, ty)| render_tile(tx, ty).map(|tile| (ty * tiles_x + tx, tile)))
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().expect("transform worker panicked")).collect()
    });
    for (idx, tile) in rendered.into_iter().flatten() {
        out[idx] = Some(Arc::new(tile));
    }
    out
}
