//! A stroke in progress.
//!
//! Dabs accumulate into a per-stroke coverage mask rather than straight onto the
//! layer. The layer is then rebuilt from its pre-stroke pixels and the mask, so
//! a stroke never exceeds the brush opacity however often it crosses itself,
//! while flow still builds up inside the stroke.

use std::sync::Arc;

use super::brush::BrushSettings;
use super::layer::{IRect, Layer, Tile, ONE, TILE};

/// One input sample in canvas coordinates. `pressure` is 0..=1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StrokePoint {
    pub x: f32,
    pub y: f32,
    pub pressure: f32,
}

impl StrokePoint {
    pub fn new(x: f32, y: f32, pressure: f32) -> Self {
        StrokePoint { x, y, pressure }
    }
}

struct StrokeTile {
    mask: Box<[u16]>,
    /// The layer tile as it was before the stroke touched it.
    orig: Option<Arc<Tile>>,
}

pub(crate) struct ActiveStroke {
    pub layer: usize,
    brush: BrushSettings,
    /// Paint colour, 15-bit per channel, not premultiplied.
    color: [u32; 3],
    erase: bool,
    opacity: u32,
    tiles: Vec<Option<StrokeTile>>,
    /// Last point actually drawn to (after smoothing).
    pen: StrokePoint,
    /// Last raw input point.
    raw: StrokePoint,
    /// Distance left to travel before the next dab.
    gap: f32,
}

impl ActiveStroke {
    pub fn begin(
        layer_index: usize,
        layer: &mut Layer,
        brush: &BrushSettings,
        color: [u8; 3],
        erase: bool,
        p: StrokePoint,
    ) -> (Self, IRect) {
        let to15 = |v: u8| (v as u32 * ONE + 127) / 255;
        let mut s = ActiveStroke {
            layer: layer_index,
            brush: brush.clone(),
            color: [to15(color[0]), to15(color[1]), to15(color[2])],
            erase,
            opacity: (brush.opacity.clamp(0.0, 1.0) * ONE as f32).round() as u32,
            tiles: (0..layer.tiles.len()).map(|_| None).collect(),
            pen: p,
            raw: p,
            gap: 0.0,
        };
        let rect = s.dab(layer, p);
        s.gap = s.spacing_at(p.pressure);
        s.recomposite(layer, rect);
        (s, rect)
    }

    /// Extends the stroke towards a new input sample. Returns the changed area.
    pub fn line_to(&mut self, layer: &mut Layer, p: StrokePoint) -> IRect {
        self.raw = p;
        // The pen chases the raw input; heavier smoothing means a lazier chase.
        let k = (1.0 - self.brush.smoothing.clamp(0.0, 1.0)).powi(2) * 0.97 + 0.03;
        let kp = k.max(0.5);
        let target = StrokePoint {
            x: self.pen.x + (p.x - self.pen.x) * k,
            y: self.pen.y + (p.y - self.pen.y) * k,
            pressure: self.pen.pressure + (p.pressure - self.pen.pressure) * kp,
        };
        self.segment_to(layer, target)
    }

    /// Draws the remaining distance to where the pen was lifted.
    pub fn finish(&mut self, layer: &mut Layer) -> IRect {
        let raw = self.raw;
        self.segment_to(layer, raw)
    }

    /// Pre-stroke contents of every tile the stroke modified.
    pub fn into_saved(self) -> Vec<(usize, Option<Arc<Tile>>)> {
        self.tiles
            .into_iter()
            .enumerate()
            .filter_map(|(i, t)| t.map(|t| (i, t.orig)))
            .collect()
    }

    fn spacing_at(&self, pressure: f32) -> f32 {
        (self.brush.spacing * self.brush.diameter_at(pressure)).max(0.3)
    }

    fn segment_to(&mut self, layer: &mut Layer, to: StrokePoint) -> IRect {
        let from = self.pen;
        let (dx, dy) = (to.x - from.x, to.y - from.y);
        let len = (dx * dx + dy * dy).sqrt();
        let mut rect = IRect::new(0, 0, 0, 0);
        let mut pos = 0.0;
        while len > 0.0 {
            if pos + self.gap > len {
                self.gap -= len - pos;
                break;
            }
            pos += self.gap;
            let t = pos / len;
            let p = StrokePoint {
                x: from.x + dx * t,
                y: from.y + dy * t,
                pressure: from.pressure + (to.pressure - from.pressure) * t,
            };
            rect = rect.union(&self.dab(layer, p));
            self.gap = self.spacing_at(p.pressure);
        }
        self.pen = to;
        self.recomposite(layer, rect);
        rect
    }

    /// Stamps one round tip into the coverage mask.
    fn dab(&mut self, layer: &Layer, p: StrokePoint) -> IRect {
        let mut radius = self.brush.diameter_at(p.pressure) * 0.5;
        let mut flow = self.brush.flow_at(p.pressure);
        // Sub-pixel tips keep a half-pixel footprint and fade instead of vanishing.
        if radius < 0.5 {
            flow *= (radius / 0.5).powi(2);
            radius = 0.5;
        }
        if flow <= 0.0 {
            return IRect::new(0, 0, 0, 0);
        }
        let falloff = (radius * (1.0 - self.brush.hardness.clamp(0.0, 1.0))).max(1.0);
        let soft = falloff > 1.0;
        let outer = radius + 0.5;
        let rect = IRect::new(
            (p.x - outer).floor() as i32,
            (p.y - outer).floor() as i32,
            (p.x + outer).ceil() as i32 + 1,
            (p.y + outer).ceil() as i32 + 1,
        )
        .intersect(&layer.bounds());
        if rect.is_empty() {
            return rect;
        }
        let flow15 = flow * ONE as f32;
        let outer_sq = outer * outer;

        let (tx0, tx1) = (rect.x0 as usize / TILE, (rect.x1 as usize - 1) / TILE);
        let (ty0, ty1) = (rect.y0 as usize / TILE, (rect.y1 as usize - 1) / TILE);
        for ty in ty0..=ty1 {
            for tx in tx0..=tx1 {
                let idx = layer.tile_index(tx, ty);
                let tile = self.tiles[idx].get_or_insert_with(|| StrokeTile {
                    mask: vec![0u16; TILE * TILE].into_boxed_slice(),
                    orig: layer.tiles[idx].clone(),
                });
                let (ox, oy) = ((tx * TILE) as i32, (ty * TILE) as i32);
                let x0 = rect.x0.max(ox);
                let x1 = rect.x1.min(ox + TILE as i32);
                let y0 = rect.y0.max(oy);
                let y1 = rect.y1.min(oy + TILE as i32);
                for y in y0..y1 {
                    let fy = y as f32 + 0.5 - p.y;
                    let row = (y - oy) as usize * TILE;
                    for x in x0..x1 {
                        let fx = x as f32 + 0.5 - p.x;
                        let d2 = fx * fx + fy * fy;
                        if d2 >= outer_sq {
                            continue;
                        }
                        let mut c = ((outer - d2.sqrt()) / falloff).min(1.0);
                        if soft {
                            c = c * c * (3.0 - 2.0 * c);
                        }
                        let add = (c * flow15) as u32;
                        let m = &mut tile.mask[row + (x - ox) as usize];
                        let cur = *m as u32;
                        *m = (cur + (((ONE - cur) * add) >> 15)).min(ONE) as u16;
                    }
                }
            }
        }
        rect
    }

    /// Rebuilds the layer inside `rect` from the pre-stroke pixels and the mask.
    fn recomposite(&mut self, layer: &mut Layer, rect: IRect) {
        let rect = rect.intersect(&layer.bounds());
        if rect.is_empty() {
            return;
        }
        let (tx0, tx1) = (rect.x0 as usize / TILE, (rect.x1 as usize - 1) / TILE);
        let (ty0, ty1) = (rect.y0 as usize / TILE, (rect.y1 as usize - 1) / TILE);
        for ty in ty0..=ty1 {
            for tx in tx0..=tx1 {
                let idx = layer.tile_index(tx, ty);
                let Some(st) = &self.tiles[idx] else { continue };
                if self.erase && st.orig.is_none() {
                    continue; // nothing to erase
                }
                let dst = Arc::make_mut(
                    layer.tiles[idx].get_or_insert_with(|| Arc::new(Tile::empty())),
                );
                let (ox, oy) = ((tx * TILE) as i32, (ty * TILE) as i32);
                let x0 = rect.x0.max(ox);
                let x1 = rect.x1.min(ox + TILE as i32);
                let y0 = rect.y0.max(oy);
                let y1 = rect.y1.min(oy + TILE as i32);
                for y in y0..y1 {
                    let row = (y - oy) as usize * TILE;
                    for x in x0..x1 {
                        let i = row + (x - ox) as usize;
                        let o = match &st.orig {
                            Some(t) => t.0[i],
                            None => [0; 4],
                        };
                        let a = (st.mask[i] as u32 * self.opacity) >> 15;
                        let keep = ONE - a;
                        let mix = |under: u16, paint: u32| -> u16 {
                            ((paint * a + under as u32 * keep + ONE / 2) >> 15) as u16
                        };
                        dst.0[i] = if self.erase {
                            [mix(o[0], 0), mix(o[1], 0), mix(o[2], 0), mix(o[3], 0)]
                        } else {
                            [
                                mix(o[0], self.color[0]),
                                mix(o[1], self.color[1]),
                                mix(o[2], self.color[2]),
                                mix(o[3], ONE),
                            ]
                        };
                    }
                }
            }
        }
    }
}
