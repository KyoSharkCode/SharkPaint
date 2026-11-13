//! A stroke in progress.
//!
//! Dabs accumulate into a per-stroke coverage mask rather than straight onto the
//! layer. The layer is then rebuilt from its pre-stroke pixels and the mask, so
//! a stroke never exceeds the brush opacity however often it crosses itself,
//! while flow still builds up inside the stroke.
//!
//! Brushes that mix colour work differently: each dab blends straight into the
//! layer, carrying colour picked up from the canvas along the way.

use std::sync::Arc;

use super::brush::{BrushSettings, BrushTip, PaperTexture};
use super::layer::{IRect, Layer, Tile, ONE, TILE};
use super::selection::Selection;

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
    /// Keep the layer's existing alpha; paint only where it already has pixels.
    alpha_lock: bool,
    opacity: u32,
    /// Painting is confined to this selection, if there is one.
    selection: Option<Arc<Selection>>,
    /// Every dab is repeated mirrored across the vertical line at this x.
    mirror_x: Option<f32>,
    tiles: Vec<Option<StrokeTile>>,
    /// Last point actually drawn to (after smoothing).
    pen: StrokePoint,
    /// Last raw input point.
    raw: StrokePoint,
    /// Distance left to travel before the next dab.
    gap: f32,
    /// Direction the stroke is travelling in, in radians.
    direction: f32,
    /// State of the generator behind scatter and jitter.
    random: u32,
    /// The brush mixes with the canvas: dabs go straight onto the layer.
    wet: bool,
    /// Colour the brush is carrying (premultiplied RGBA, 0..=1), once it has
    /// touched the canvas.
    carried: Option<[f32; 4]>,
}

/// The outline of one dab, in the frame of the canvas.
struct DabShape {
    radius: f32,
    outer: f32,
    falloff: f32,
    soft: bool,
    /// A disc needs no rotation or squash.
    plain: bool,
    sin: f32,
    cos: f32,
    roundness: f32,
    flip: bool,
    tip: BrushTip,
}

impl DabShape {
    /// Coverage (0..=1) at an offset from the dab's centre.
    fn coverage(&self, mut fx: f32, fy: f32) -> f32 {
        if fx * fx + fy * fy >= self.outer * self.outer {
            return 0.0;
        }
        let disc = |lx: f32, ly: f32| {
            let c = ((self.outer - (lx * lx + ly * ly).sqrt()) / self.falloff).clamp(0.0, 1.0);
            if self.soft { c * c * (3.0 - 2.0 * c) } else { c }
        };
        if self.plain {
            return disc(fx, fy);
        }
        if self.flip {
            fx = -fx;
        }
        // Into the tip's frame: undo the rotation, then the squash.
        let (lx, ly) = (fx * self.cos + fy * self.sin, (-fx * self.sin + fy * self.cos) / self.roundness);
        match &self.tip {
            BrushTip::Round => disc(lx, ly),
            BrushTip::Image(image) => image.sample(
                lx / self.radius * 0.5 + 0.5,
                ly / self.radius * 0.5 + 0.5,
                image.size as f32 / (2.0 * self.radius),
            ),
        }
    }
}

/// How much paint the paper holds at a canvas pixel, for a brush with grain.
struct Grain {
    paper: Arc<PaperTexture>,
    strength: f32,
    inverse_scale: f32,
    reach: f32,
}

impl Grain {
    /// Share (0..=1) of full coverage this spot of paper can hold.
    fn held(&self, x: i32, y: i32) -> f32 {
        let height = self.paper.height(x as f32 * self.inverse_scale, y as f32 * self.inverse_scale);
        let held = ((height - (1.0 - self.reach)) / 0.5 + 0.5).clamp(0.0, 1.0);
        1.0 - self.strength + self.strength * held * held * (3.0 - 2.0 * held)
    }
}

impl ActiveStroke {
    pub fn begin(
        layer_index: usize,
        layer: &mut Layer,
        brush: &BrushSettings,
        color: [u8; 3],
        erase: bool,
        selection: Option<Arc<Selection>>,
        mirror_x: Option<f32>,
        p: StrokePoint,
    ) -> (Self, IRect) {
        let to15 = |v: u8| (v as u32 * ONE + 127) / 255;
        let mut s = ActiveStroke {
            layer: layer_index,
            brush: brush.clone(),
            color: [to15(color[0]), to15(color[1]), to15(color[2])],
            erase,
            alpha_lock: layer.props.alpha_lock,
            opacity: (brush.opacity.clamp(0.0, 1.0) * ONE as f32).round() as u32,
            selection,
            mirror_x,
            tiles: (0..layer.tiles.len()).map(|_| None).collect(),
            pen: p,
            raw: p,
            gap: 0.0,
            direction: 0.0,
            // Seeded from where the stroke starts: varied, yet repeatable.
            random: (p.x.to_bits() ^ p.y.to_bits().rotate_left(13) ^ 0x9E37_79B9) | 1,
            wet: brush.mix > 0.0 && !erase,
            carried: None,
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

    /// Puts back every tile the stroke touched, as if it had never started.
    pub fn revert(self, layer: &mut Layer) -> IRect {
        let tiles_x = layer.tiles_x;
        let mut rect = IRect::new(0, 0, 0, 0);
        for (idx, orig) in self.into_saved() {
            layer.tiles[idx] = orig;
            let (tx, ty) = ((idx % tiles_x) as i32, (idx / tiles_x) as i32);
            let t = TILE as i32;
            rect = rect.union(&IRect::new(tx * t, ty * t, (tx + 1) * t, (ty + 1) * t));
        }
        rect
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
        if len > 0.5 {
            self.direction = dy.atan2(dx);
        }
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

    /// A value in 0..1 from the stroke's own generator.
    fn random(&mut self) -> f32 {
        self.random ^= self.random << 13;
        self.random ^= self.random >> 17;
        self.random ^= self.random << 5;
        (self.random >> 8) as f32 / (1u32 << 24) as f32
    }

    /// Stamps the tip at `p` and, with symmetry on, at its mirror image. Each
    /// dab gets its own scatter, rotation and size variation.
    fn dab(&mut self, layer: &mut Layer, p: StrokePoint) -> IRect {
        let diameter = self.brush.diameter_at(p.pressure);
        let mut at = p;
        if self.brush.scatter > 0.0 {
            let reach = self.brush.scatter * diameter;
            at.x += (self.random() * 2.0 - 1.0) * reach;
            at.y += (self.random() * 2.0 - 1.0) * reach;
        }
        let mut angle = self.brush.angle.to_radians();
        if self.brush.follow_direction {
            angle += self.direction;
        }
        if self.brush.angle_jitter > 0.0 {
            angle += (self.random() * 2.0 - 1.0) * std::f32::consts::PI * self.brush.angle_jitter;
        }
        let scale = if self.brush.size_jitter > 0.0 {
            1.0 - self.random() * self.brush.size_jitter.clamp(0.0, 1.0)
        } else {
            1.0
        };
        if self.wet {
            self.pick_up(layer, at);
        }
        let rect = self.place(layer, at, angle, scale, false);
        match self.mirror_x {
            Some(axis) => {
                let mirrored = StrokePoint { x: 2.0 * axis - at.x, ..at };
                rect.union(&self.place(layer, mirrored, angle, scale, true))
            }
            None => rect,
        }
    }

    fn place(&mut self, layer: &mut Layer, p: StrokePoint, angle: f32, scale: f32, flip: bool) -> IRect {
        if self.wet {
            self.blend(layer, p, angle, scale, flip)
        } else {
            self.stamp(layer, p, angle, scale, flip)
        }
    }

    /// The outline of a dab at `p`, its flow, and the canvas area it can touch.
    fn dab_shape(&self, layer: &Layer, p: StrokePoint, angle: f32, scale: f32, flip: bool) -> Option<(DabShape, f32, IRect)> {
        let mut radius = self.brush.diameter_at(p.pressure) * 0.5 * scale;
        let mut flow = self.brush.flow_at(p.pressure);
        // Sub-pixel tips keep a half-pixel footprint and fade instead of vanishing.
        if radius < 0.5 {
            flow *= (radius / 0.5).powi(2);
            radius = 0.5;
        }
        if flow <= 0.0 {
            return None;
        }
        let falloff = (radius * (1.0 - self.brush.hardness.clamp(0.0, 1.0))).max(1.0);
        let outer = radius + 0.5;
        let rect = IRect::new(
            (p.x - outer).floor() as i32,
            (p.y - outer).floor() as i32,
            (p.x + outer).ceil() as i32 + 1,
            (p.y + outer).ceil() as i32 + 1,
        )
        .intersect(&layer.bounds());
        if rect.is_empty() {
            return None;
        }
        let roundness = self.brush.roundness.clamp(0.02, 1.0);
        let (sin, cos) = angle.sin_cos();
        let shape = DabShape {
            radius,
            outer,
            falloff,
            soft: falloff > 1.0,
            plain: matches!(self.brush.tip, BrushTip::Round) && roundness >= 0.999,
            sin,
            cos,
            roundness,
            flip,
            tip: self.brush.tip.clone(),
        };
        Some((shape, flow, rect))
    }

    /// Paper grain for a dab at this pressure, if the brush has any: light
    /// pressure only reaches the high points of the paper, firm pressure fills
    /// the hollows too. The paper is fixed to the canvas.
    fn grain(&self, pressure: f32) -> Option<Grain> {
        self.brush.paper.clone().filter(|_| self.brush.grain > 0.0).map(|paper| Grain {
            paper,
            strength: self.brush.grain.clamp(0.0, 1.0),
            inverse_scale: 1.0 / self.brush.grain_scale.clamp(0.1, 16.0),
            reach: 0.25 + 0.7 * pressure.clamp(0.0, 1.0),
        })
    }

    /// Lets the brush take up the colour under it. The longer the drag, the
    /// more slowly what it already carries is replaced.
    ///
    /// A blender reads partly the canvas as it is now, so going over a spot
    /// again softens it further. A brush with paint of its own reads mostly the
    /// canvas as it was before the stroke. Reading only the current canvas would
    /// make any brush pick up what it has just put down and stop taking colour.
    fn pick_up(&mut self, layer: &Layer, p: StrokePoint) {
        let radius = (self.brush.diameter_at(p.pressure) * 0.5).max(1.0);
        // Average of a grid of samples across the middle of the tip.
        let reach = radius * 0.6;
        let steps = ((reach * 2.0) as i32).clamp(1, 7);
        let live = self.brush.mix.clamp(0.0, 1.0).powi(2) * 0.6;
        let (mut sum, mut count) = ([0f32; 4], 0f32);
        for iy in 0..steps {
            for ix in 0..steps {
                let offset = |i: i32| if steps == 1 { 0.0 } else { (i as f32 / (steps - 1) as f32 * 2.0 - 1.0) * reach };
                let (dx, dy) = (offset(ix), offset(iy));
                if dx * dx + dy * dy > reach * reach + 0.01 {
                    continue;
                }
                let (x, y) = ((p.x + dx).floor() as i32, (p.y + dy).floor() as i32);
                let now = layer.pixel(x, y);
                let before = self.before(layer, x, y).unwrap_or(now);
                for c in 0..4 {
                    sum[c] += before[c] as f32 + (now[c] as f32 - before[c] as f32) * live;
                }
                count += 1.0;
            }
        }
        let under: [f32; 4] = std::array::from_fn(|c| sum[c] / (count.max(1.0) * ONE as f32));
        let keep = (self.brush.drag.clamp(0.0, 1.0) * 0.97).powf(0.5);
        self.carried = Some(match self.carried {
            Some(carried) => std::array::from_fn(|c| under[c] + (carried[c] - under[c]) * keep),
            None => under,
        });
    }

    /// The pixel as it was before this stroke, if the stroke has touched its tile.
    fn before(&self, layer: &Layer, x: i32, y: i32) -> Option<[u16; 4]> {
        if x < 0 || y < 0 || x >= layer.width as i32 || y >= layer.height as i32 {
            return None;
        }
        let (tx, ty) = (x as usize / TILE, y as usize / TILE);
        let saved = self.tiles[layer.tile_index(tx, ty)].as_ref()?;
        Some(saved.orig.as_ref().map_or([0; 4], |tile| tile.0[(y as usize - ty * TILE) * TILE + (x as usize - tx * TILE)]))
    }

    /// Blends one dab of a mixing brush straight into the layer: each pixel
    /// moves towards the brush's colour, which is part paint and part what it
    /// picked up from the canvas.
    fn blend(&mut self, layer: &mut Layer, p: StrokePoint, angle: f32, scale: f32, flip: bool) -> IRect {
        let Some((shape, flow, rect)) = self.dab_shape(layer, p, angle, scale, flip) else {
            return IRect::new(0, 0, 0, 0);
        };
        let grain = self.grain(p.pressure);
        let mix = self.brush.mix.clamp(0.0, 1.0);
        let carried = self.carried.unwrap_or([0.0; 4]);
        let paint = [self.color[0] as f32 / ONE as f32, self.color[1] as f32 / ONE as f32, self.color[2] as f32 / ONE as f32, 1.0];
        // A painting brush keeps its own colour where the canvas has none to give;
        // a pure blender carries the emptiness too.
        let weight = mix * (carried[3] + (1.0 - carried[3]) * mix.powi(7));
        let loaded: [f32; 4] = std::array::from_fn(|c| (paint[c] + (carried[c] - paint[c]) * weight) * ONE as f32);

        let (tx0, tx1) = (rect.x0 as usize / TILE, (rect.x1 as usize - 1) / TILE);
        let (ty0, ty1) = (rect.y0 as usize / TILE, (rect.y1 as usize - 1) / TILE);
        for ty in ty0..=ty1 {
            for tx in tx0..=tx1 {
                let idx = layer.tile_index(tx, ty);
                if self.alpha_lock && layer.tiles[idx].is_none() {
                    continue; // nothing to recolour
                }
                // Remember the tile as it was, for undo.
                if self.tiles[idx].is_none() {
                    self.tiles[idx] = Some(StrokeTile { mask: Box::default(), orig: layer.tiles[idx].clone() });
                }
                let (ox, oy) = ((tx * TILE) as i32, (ty * TILE) as i32);
                let x0 = rect.x0.max(ox);
                let x1 = rect.x1.min(ox + TILE as i32);
                let y0 = rect.y0.max(oy);
                let y1 = rect.y1.min(oy + TILE as i32);
                for y in y0..y1 {
                    let fy = y as f32 + 0.5 - p.y;
                    for x in x0..x1 {
                        let mut amount = shape.coverage(x as f32 + 0.5 - p.x, fy) * flow;
                        if let Some(grain) = &grain {
                            amount *= grain.held(x, y);
                        }
                        if let Some(selection) = &self.selection {
                            amount *= selection.coverage(x, y) as f32 / 255.0;
                        }
                        if amount <= 0.002 {
                            continue;
                        }
                        let i = (y - oy) as usize * TILE + (x - ox) as usize;
                        let here = layer.tiles[idx].as_ref().map_or([0; 4], |tile| tile.0[i]);
                        let mut out: [u16; 4] = std::array::from_fn(|c| {
                            (here[c] as f32 + (loaded[c] - here[c] as f32) * amount).round().clamp(0.0, ONE as f32) as u16
                        });
                        if self.alpha_lock {
                            out[3] = here[3];
                        }
                        for c in 0..3 {
                            out[c] = out[c].min(out[3]);
                        }
                        if out == here {
                            continue;
                        }
                        // Only now is the tile needed; empty areas stay empty.
                        Arc::make_mut(layer.tiles[idx].get_or_insert_with(|| Arc::new(Tile::empty()))).0[i] = out;
                    }
                }
            }
        }
        rect
    }

    /// Stamps one tip into the coverage mask, rotated by `angle` and scaled by
    /// `scale`; `flip` mirrors it left to right.
    fn stamp(&mut self, layer: &Layer, p: StrokePoint, angle: f32, scale: f32, flip: bool) -> IRect {
        let Some((shape, flow, rect)) = self.dab_shape(layer, p, angle, scale, flip) else {
            return IRect::new(0, 0, 0, 0);
        };
        let grain = self.grain(p.pressure);
        let flow15 = flow * ONE as f32;

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
                        let c = shape.coverage(x as f32 + 0.5 - p.x, fy);
                        if c <= 0.0 {
                            continue;
                        }
                        let add = (c * flow15) as u32;
                        let m = &mut tile.mask[row + (x - ox) as usize];
                        let cur = *m as u32;
                        let mut value = (cur + (((ONE - cur) * add) >> 15)).min(ONE);
                        if let Some(grain) = &grain {
                            // The paper caps the coverage rather than thinning each
                            // dab, so overlapping dabs do not fill the grain in.
                            let limit = (grain.held(x, y) * ONE as f32) as u32;
                            value = value.min(limit.max(cur));
                        }
                        *m = value as u16;
                    }
                }
            }
        }
        rect
    }

    /// Rebuilds the layer inside `rect` from the pre-stroke pixels and the mask.
    fn recomposite(&mut self, layer: &mut Layer, rect: IRect) {
        let rect = rect.intersect(&layer.bounds());
        if rect.is_empty() || self.wet {
            return; // mixing brushes have already painted the layer
        }
        let (tx0, tx1) = (rect.x0 as usize / TILE, (rect.x1 as usize - 1) / TILE);
        let (ty0, ty1) = (rect.y0 as usize / TILE, (rect.y1 as usize - 1) / TILE);
        for ty in ty0..=ty1 {
            for tx in tx0..=tx1 {
                let idx = layer.tile_index(tx, ty);
                let Some(st) = &self.tiles[idx] else { continue };
                if st.orig.is_none() && (self.erase || self.alpha_lock) {
                    continue; // nothing to erase or to paint onto
                }
                if self.erase && self.alpha_lock {
                    continue; // erasing would change the locked transparency
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
                        let mut a = (st.mask[i] as u32 * self.opacity) >> 15;
                        if let Some(selection) = &self.selection {
                            a = a * selection.coverage(x, y) as u32 / 255;
                        }
                        let keep = ONE - a;
                        let mix = |under: u16, paint: u32| -> u16 {
                            ((paint * a + under as u32 * keep + ONE / 2) >> 15) as u16
                        };
                        dst.0[i] = if self.erase {
                            [mix(o[0], 0), mix(o[1], 0), mix(o[2], 0), mix(o[3], 0)]
                        } else if self.alpha_lock {
                            // Paint scaled by the pixel's own alpha, which stays put.
                            let own = o[3] as u32;
                            let paint = |c: u32| (c * own + ONE / 2) >> 15;
                            [
                                mix(o[0], paint(self.color[0])),
                                mix(o[1], paint(self.color[1])),
                                mix(o[2], paint(self.color[2])),
                                o[3],
                            ]
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
