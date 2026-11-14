//! Straight lines, rectangles and ellipses drawn onto the active layer.

use super::document::Document;
use super::layer::{blend_over, BlendMode, IRect, ONE, TILE};
use super::tools::TileEdit;

/// A shape stretched between two corners of a drag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    /// From the first point to the second.
    Line,
    /// The box with the two points as opposite corners.
    Rectangle,
    /// The ellipse that fits that box.
    Ellipse,
}

impl Shape {
    /// How far (x, y) is from the shape's edge, in pixels: negative inside,
    /// positive outside. `width` is the thickness of a line or of an outline;
    /// a filled shape has none.
    fn distance(self, x: f32, y: f32, a: (f32, f32), b: (f32, f32), width: f32, filled: bool) -> f32 {
        let (cx, cy) = ((a.0 + b.0) * 0.5, (a.1 + b.1) * 0.5);
        let (hx, hy) = ((b.0 - a.0).abs() * 0.5, (b.1 - a.1).abs() * 0.5);
        let half = width * 0.5;
        let edge = match self {
            Shape::Line => {
                let (dx, dy) = (b.0 - a.0, b.1 - a.1);
                let length = dx * dx + dy * dy;
                let t = if length > 0.0 { (((x - a.0) * dx + (y - a.1) * dy) / length).clamp(0.0, 1.0) } else { 0.0 };
                let (px, py) = (a.0 + dx * t - x, a.1 + dy * t - y);
                return (px * px + py * py).sqrt() - half;
            }
            Shape::Rectangle => {
                let (qx, qy) = ((x - cx).abs() - hx, (y - cy).abs() - hy);
                let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
                outside + qx.max(qy).min(0.0)
            }
            Shape::Ellipse => {
                if hx <= 0.0 || hy <= 0.0 {
                    return f32::MAX;
                }
                // First-order distance: exact on the edge, close near it.
                let (ux, uy) = ((x - cx) / hx, (y - cy) / hy);
                let k = (ux * ux + uy * uy).sqrt();
                let slope = (ux * ux / (hx * hx) + uy * uy / (hy * hy)).sqrt();
                if slope > 0.0 { k * (k - 1.0) / slope } else { -hx.min(hy) }
            }
        };
        if filled { edge } else { edge.abs() - half }
    }
}

impl Document {
    /// Draws a shape on the active layer as one undoable step, with smooth
    /// edges. It keeps to the selection and to a locked layer's transparency.
    /// `width` is the thickness of a line or outline; `filled` fills
    /// rectangles and ellipses instead of outlining them. Returns whether
    /// anything was drawn.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_shape(
        &mut self,
        shape: Shape,
        from: (f32, f32),
        to: (f32, f32),
        color: [u8; 3],
        width: f32,
        filled: bool,
        opacity: f32,
    ) -> bool {
        self.end_stroke();
        let li = self.active_layer;
        let filled = filled && shape != Shape::Line;
        let width = width.max(1.0);
        let reach = if filled { 1.0 } else { width * 0.5 + 1.0 };
        let rect = IRect::new(
            (from.0.min(to.0) - reach).floor() as i32,
            (from.1.min(to.1) - reach).floor() as i32,
            (from.0.max(to.0) + reach).ceil() as i32 + 1,
            (from.1.max(to.1) + reach).ceil() as i32 + 1,
        )
        .intersect(&self.bounds());
        if rect.is_empty() {
            return false;
        }
        let opacity = (opacity.clamp(0.0, 1.0) * ONE as f32).round();
        let paint = color.map(|v| ((v as u32 * ONE + 127) / 255) as u16);
        let selection = self.selection.clone();
        let layer = &mut self.layers[li];
        let lock = layer.props.alpha_lock;
        let mut edit = TileEdit::default();
        for ty in rect.y0 as usize / TILE..=(rect.y1 as usize - 1) / TILE {
            for tx in rect.x0 as usize / TILE..=(rect.x1 as usize - 1) / TILE {
                let idx = layer.tile_index(tx, ty);
                if lock && layer.tiles[idx].is_none() {
                    continue; // nothing to recolour
                }
                let (ox, oy) = ((tx * TILE) as i32, (ty * TILE) as i32);
                // How much each pixel of the tile takes; most tiles of a big
                // outline take nothing and are left as they were.
                let mut amounts = [0u32; TILE * TILE];
                let mut any = false;
                for y in rect.y0.max(oy)..rect.y1.min(oy + TILE as i32) {
                    for x in rect.x0.max(ox)..rect.x1.min(ox + TILE as i32) {
                        let distance = shape.distance(x as f32 + 0.5, y as f32 + 0.5, from, to, width, filled);
                        let mut amount = (0.5 - distance).clamp(0.0, 1.0) * opacity;
                        if let Some(selection) = &selection {
                            amount *= selection.coverage(x, y) as f32 / 255.0;
                        }
                        amounts[(y - oy) as usize * TILE + (x - ox) as usize] = amount as u32;
                        any |= amount as u32 != 0;
                    }
                }
                if !any {
                    continue;
                }
                let tile = edit.tile_mut(layer, idx);
                for (d, amount) in tile.0.iter_mut().zip(amounts) {
                    let own = d[3];
                    if amount == 0 || (lock && own == 0) {
                        continue;
                    }
                    let mut px = d.map(u32::from);
                    blend_over(&mut px, [paint[0], paint[1], paint[2], ONE as u16], amount, BlendMode::Normal);
                    if lock {
                        // Colour changes; the transparency stays as it was.
                        for c in 0..3 {
                            px[c] = (px[c] * own as u32 / px[3].max(1)).min(own as u32);
                        }
                        px[3] = own as u32;
                    }
                    *d = px.map(|c| c as u16);
                }
            }
        }
        let saved = edit.into_saved();
        let changed = !saved.is_empty();
        self.commit_tiles(li, saved);
        self.mark_dirty(rect);
        changed
    }
}
