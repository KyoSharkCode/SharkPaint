//! Changes to the canvas as a whole: its size, with or without rescaling the
//! picture, and turning the whole picture over.

use std::sync::Arc;

use super::document::Document;
use super::layer::{IRect, Layer, Px, Tile, TILE};

/// Smallest canvas side.
const MIN_SIDE: usize = 1;

/// A copy of `layer` for a canvas of another size. `pixel` gives the new
/// layer's pixel at a position; `reads` tells which part of the old layer a
/// part of the new one is made from, so empty stretches are skipped.
fn rebuilt(
    layer: &Layer,
    width: usize,
    height: usize,
    version: u64,
    reads: impl Fn(IRect) -> IRect,
    pixel: impl Fn(i32, i32) -> Px,
) -> Layer {
    let mut out = Layer::new(width, height, layer.props.name.as_str());
    out.id = layer.id;
    out.group = layer.group;
    out.props = layer.props.clone();
    out.version = version;
    for ty in 0..out.tiles_y {
        for tx in 0..out.tiles_x {
            let (ox, oy) = ((tx * TILE) as i32, (ty * TILE) as i32);
            let area = IRect::new(ox, oy, ox + TILE as i32, oy + TILE as i32).intersect(&out.bounds());
            let from = reads(area).intersect(&layer.bounds());
            if from.is_empty() || !has_pixels(layer, from) {
                continue;
            }
            let mut tile = Tile::empty();
            let mut any = false;
            for y in area.y0..area.y1 {
                for x in area.x0..area.x1 {
                    let px = pixel(x, y);
                    if px[3] != 0 {
                        tile.0[(y - oy) as usize * TILE + (x - ox) as usize] = px;
                        any = true;
                    }
                }
            }
            if any {
                let index = out.tile_index(tx, ty);
                out.tiles[index] = Some(Arc::new(tile));
            }
        }
    }
    out
}

/// Whether the layer has anything stored where `rect` falls.
fn has_pixels(layer: &Layer, rect: IRect) -> bool {
    let (tx0, tx1) = (rect.x0 as usize / TILE, (rect.x1 as usize - 1) / TILE);
    let (ty0, ty1) = (rect.y0 as usize / TILE, (rect.y1 as usize - 1) / TILE);
    (ty0..=ty1).any(|ty| (tx0..=tx1).any(|tx| layer.tiles[layer.tile_index(tx, ty)].is_some()))
}

impl Document {
    /// Changes the canvas size without rescaling: the picture keeps its size
    /// and is shifted by (`dx`, `dy`); what falls outside is cut off and new
    /// area is empty. One undoable step. Returns whether anything changed.
    pub fn resize_canvas(&mut self, width: usize, height: usize, dx: i32, dy: i32) -> bool {
        let (width, height) = (width.max(MIN_SIDE), height.max(MIN_SIDE));
        if (width, height, dx, dy) == (self.width, self.height, 0, 0) {
            return false;
        }
        self.end_stroke();
        let version = self.next_version();
        let layers = self
            .layers
            .iter()
            .map(|layer| {
                let reads = |area: IRect| IRect::new(area.x0 - dx, area.y0 - dy, area.x1 - dx, area.y1 - dy);
                let shift = |layer: &Layer| rebuilt(layer, width, height, version, reads, |x, y| layer.pixel(x - dx, y - dy));
                let mut moved = shift(layer);
                moved.mask = layer.mask.as_deref().map(|mask| Box::new(shift(mask)));
                // Text keeps being text, in its new place.
                moved.text = layer.text.clone().map(|mut text| {
                    text.x += dx;
                    text.y += dy;
                    text
                });
                moved
            })
            .collect();
        self.replace_canvas(width, height, layers);
        true
    }

    /// Cuts the canvas down to the box around the selection. Returns whether
    /// there was a selection to crop to.
    pub fn crop_to_selection(&mut self) -> bool {
        let Some(bounds) = self.selection.as_ref().map(|s| s.bounds) else { return false };
        self.resize_canvas(bounds.width(), bounds.height(), -bounds.x0, -bounds.y0)
    }

    /// Rescales the whole picture to a new canvas size, smoothly. Text layers
    /// become ordinary ones. One undoable step. Returns whether anything changed.
    pub fn scale_image(&mut self, width: usize, height: usize) -> bool {
        let (width, height) = (width.max(MIN_SIDE), height.max(MIN_SIDE));
        if (width, height) == (self.width, self.height) {
            return false;
        }
        self.end_stroke();
        let version = self.next_version();
        // Old pixels per new pixel, along each axis.
        let (sx, sy) = (self.width as f32 / width as f32, self.height as f32 / height as f32);
        let scale = |layer: &Layer| {
                let reads = |area: IRect| {
                    IRect::new(
                        (area.x0 as f32 * sx).floor() as i32 - 1,
                        (area.y0 as f32 * sy).floor() as i32 - 1,
                        (area.x1 as f32 * sx).ceil() as i32 + 1,
                        (area.y1 as f32 * sy).ceil() as i32 + 1,
                    )
                };
                // Along an axis that shrinks, every old pixel under the new one
                // counts; along one that grows, the two nearest are blended.
                let span = |at: i32, scale: f32, limit: usize| -> (i32, i32, f32) {
                    if scale > 1.0 {
                        let from = (at as f32 * scale).floor() as i32;
                        let to = (((at + 1) as f32 * scale).ceil() as i32).min(limit as i32).max(from + 1);
                        (from, to, 0.0)
                    } else {
                        let centre = (at as f32 + 0.5) * scale - 0.5;
                        let from = centre.floor();
                        (from as i32, from as i32 + 2, centre - from)
                    }
                };
                let pixel = |x: i32, y: i32| -> Px {
                    let (x0, x1, fx) = span(x, sx, layer.width);
                    let (y0, y1, fy) = span(y, sy, layer.height);
                    let mut sum = [0f32; 4];
                    let mut total = 0f32;
                    for yy in y0..y1 {
                        // Blending two neighbours weighs them by closeness; an
                        // average weighs everything alike.
                        let wy = if sy > 1.0 { 1.0 } else if yy == y0 { 1.0 - fy } else { fy };
                        for xx in x0..x1 {
                            let wx = if sx > 1.0 { 1.0 } else if xx == x0 { 1.0 - fx } else { fx };
                            let weight = wx * wy;
                            if weight <= 0.0 {
                                continue;
                            }
                            // Past the edge, the nearest pixel stands in.
                            let px = layer.pixel(xx.clamp(0, layer.width as i32 - 1), yy.clamp(0, layer.height as i32 - 1));
                            for c in 0..4 {
                                sum[c] += px[c] as f32 * weight;
                            }
                            total += weight;
                        }
                    }
                    if total <= 0.0 {
                        return [0; 4];
                    }
                    sum.map(|v| (v / total).round() as u16)
                };
                rebuilt(layer, width, height, version, reads, pixel)
        };
        let layers = self
            .layers
            .iter()
            .map(|layer| {
                let mut scaled = scale(layer);
                scaled.mask = layer.mask.as_deref().map(|mask| Box::new(scale(mask)));
                scaled
            })
            .collect();
        self.replace_canvas(width, height, layers);
        true
    }

    /// Turns the whole picture over, every layer at once: left to right
    /// (`horizontal`) or top to bottom. Text layers become ordinary ones. One
    /// undoable step.
    pub fn flip_canvas(&mut self, horizontal: bool) {
        self.end_stroke();
        let version = self.next_version();
        let (w, h) = (self.width as i32, self.height as i32);
        let (width, height) = (self.width, self.height);
        let turn = |layer: &Layer| {
                let reads = |area: IRect| {
                    if horizontal {
                        IRect::new(w - area.x1, area.y0, w - area.x0, area.y1)
                    } else {
                        IRect::new(area.x0, h - area.y1, area.x1, h - area.y0)
                    }
                };
                let pixel = |x: i32, y: i32| if horizontal { layer.pixel(w - 1 - x, y) } else { layer.pixel(x, h - 1 - y) };
                rebuilt(layer, width, height, version, reads, pixel)
        };
        let layers = self
            .layers
            .iter()
            .map(|layer| {
                let mut turned = turn(layer);
                turned.mask = layer.mask.as_deref().map(|mask| Box::new(turn(mask)));
                turned
            })
            .collect();
        self.replace_canvas(self.width, self.height, layers);
    }
}
