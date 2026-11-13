//! Sparse tiled pixel storage.
//!
//! Pixels are premultiplied RGBA with 15-bit fixed-point channels (`ONE` = 1.0),
//! which keeps low-flow brush buildup smooth where 8 bits would band. Tiles are
//! reference counted so undo snapshots share untouched data with the live layer.

use std::sync::Arc;

/// Fixed-point representation of 1.0.
pub const ONE: u32 = 1 << 15;
/// Tile edge length in pixels.
pub const TILE: usize = 64;

/// One premultiplied RGBA pixel.
pub type Px = [u16; 4];

#[derive(Clone)]
pub struct Tile(pub Box<[Px; TILE * TILE]>);

impl Tile {
    pub fn empty() -> Self {
        Tile(vec![[0u16; 4]; TILE * TILE].into_boxed_slice().try_into().unwrap())
    }
}

/// Integer rectangle, `x1`/`y1` exclusive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IRect {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
}

impl IRect {
    pub fn new(x0: i32, y0: i32, x1: i32, y1: i32) -> Self {
        IRect { x0, y0, x1, y1 }
    }

    pub fn is_empty(&self) -> bool {
        self.x1 <= self.x0 || self.y1 <= self.y0
    }

    pub fn width(&self) -> usize {
        (self.x1 - self.x0).max(0) as usize
    }

    pub fn height(&self) -> usize {
        (self.y1 - self.y0).max(0) as usize
    }

    pub fn union(&self, o: &IRect) -> IRect {
        if self.is_empty() {
            return *o;
        }
        if o.is_empty() {
            return *self;
        }
        IRect::new(self.x0.min(o.x0), self.y0.min(o.y0), self.x1.max(o.x1), self.y1.max(o.y1))
    }

    pub fn intersect(&self, o: &IRect) -> IRect {
        IRect::new(self.x0.max(o.x0), self.y0.max(o.y0), self.x1.min(o.x1), self.y1.min(o.y1))
    }
}

/// How a layer's colours combine with the layers beneath it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlendMode {
    Normal,
    Multiply,
    Screen,
    Overlay,
    Darken,
    Lighten,
    Add,
}

impl BlendMode {
    pub const ALL: [BlendMode; 7] = [
        BlendMode::Normal,
        BlendMode::Multiply,
        BlendMode::Screen,
        BlendMode::Overlay,
        BlendMode::Darken,
        BlendMode::Lighten,
        BlendMode::Add,
    ];

    pub fn name(&self) -> &'static str {
        match self {
            BlendMode::Normal => "Normal",
            BlendMode::Multiply => "Multiplicar",
            BlendMode::Screen => "Trama",
            BlendMode::Overlay => "Superponer",
            BlendMode::Darken => "Oscurecer",
            BlendMode::Lighten => "Aclarar",
            BlendMode::Add => "Sumar",
        }
    }

    /// Stable identifier used in project files.
    pub fn id(&self) -> u8 {
        BlendMode::ALL.iter().position(|m| m == self).unwrap() as u8
    }

    pub fn from_id(id: u8) -> Option<BlendMode> {
        BlendMode::ALL.get(id as usize).copied()
    }

    /// Blends two unpremultiplied channel values in 0..=1.
    fn mix(&self, below: f32, above: f32) -> f32 {
        match self {
            BlendMode::Normal => above,
            BlendMode::Multiply => below * above,
            BlendMode::Screen => below + above - below * above,
            BlendMode::Overlay => {
                if below <= 0.5 {
                    2.0 * below * above
                } else {
                    1.0 - 2.0 * (1.0 - below) * (1.0 - above)
                }
            }
            BlendMode::Darken => below.min(above),
            BlendMode::Lighten => below.max(above),
            BlendMode::Add => (below + above).min(1.0),
        }
    }
}

/// Composites `src` at `opacity` (0..=ONE) over `dst` in place. Both are
/// premultiplied; `dst` may itself be partly transparent.
pub fn blend_over(dst: &mut [u32; 4], src: Px, opacity: u32, mode: BlendMode) {
    let sa = (src[3] as u32 * opacity) >> 15;
    if sa == 0 {
        return;
    }
    if mode == BlendMode::Normal {
        let keep = ONE - sa;
        for ch in 0..4 {
            let s = (src[ch] as u32 * opacity) >> 15;
            dst[ch] = s + ((dst[ch] * keep + ONE / 2) >> 15);
        }
        return;
    }
    let one = ONE as f32;
    let (sa, da) = (sa as f32 / one, dst[3] as f32 / one);
    let src_alpha = src[3] as f32;
    for ch in 0..3 {
        let s = src[ch] as f32 / src_alpha; // unpremultiplied
        let d_pm = dst[ch] as f32 / one;
        let d = if da > 0.0 { d_pm / da } else { 0.0 };
        let out = (1.0 - sa) * d_pm + (1.0 - da) * s * sa + sa * da * mode.mix(d, s);
        dst[ch] = (out * one).round().clamp(0.0, one) as u32;
    }
    dst[3] = ((sa + da - sa * da) * one).round().clamp(0.0, one) as u32;
    for ch in 0..3 {
        dst[ch] = dst[ch].min(dst[3]);
    }
}

/// Everything about a layer except its pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct LayerProps {
    pub name: String,
    pub visible: bool,
    /// 0..=1.
    pub opacity: f32,
    pub blend: BlendMode,
    /// Painting keeps the layer's existing transparency.
    pub alpha_lock: bool,
    /// Clipping mask: the layer shows only where the nearest unclipped layer
    /// beneath it has pixels.
    pub clip: bool,
}

impl LayerProps {
    pub fn named(name: impl Into<String>) -> Self {
        LayerProps {
            name: name.into(),
            visible: true,
            opacity: 1.0,
            blend: BlendMode::Normal,
            alpha_lock: false,
            clip: false,
        }
    }

    pub fn opacity15(&self) -> u32 {
        (self.opacity.clamp(0.0, 1.0) * ONE as f32).round() as u32
    }
}

/// A named set of neighbouring layers that are shown, hidden and faded
/// together. The layers still blend with the rest of the picture one by one.
#[derive(Clone, Debug, PartialEq)]
pub struct LayerGroup {
    /// Unique within a document; never zero.
    pub id: u64,
    pub name: String,
    pub visible: bool,
    /// 0..=1, on top of each layer's own opacity.
    pub opacity: f32,
    /// The group's layers are folded away in the layer list.
    pub collapsed: bool,
}

#[derive(Clone)]
pub struct Layer {
    /// Identifies the layer across reordering and undo; unique within a document.
    pub id: u64,
    /// The group the layer belongs to, or zero for none. A group's layers are
    /// always next to each other in the stack.
    pub group: u64,
    /// Changes whenever the layer's pixels change (used to refresh previews).
    pub version: u64,
    pub props: LayerProps,
    pub width: usize,
    pub height: usize,
    pub tiles_x: usize,
    pub tiles_y: usize,
    pub tiles: Vec<Option<Arc<Tile>>>,
}

impl Layer {
    pub fn new(width: usize, height: usize, name: impl Into<String>) -> Self {
        let tiles_x = width.div_ceil(TILE);
        let tiles_y = height.div_ceil(TILE);
        Layer {
            id: 0,
            group: 0,
            version: 0,
            props: LayerProps::named(name),
            width,
            height,
            tiles_x,
            tiles_y,
            tiles: vec![None; tiles_x * tiles_y],
        }
    }

    pub fn bounds(&self) -> IRect {
        IRect::new(0, 0, self.width as i32, self.height as i32)
    }

    pub fn tile_index(&self, tx: usize, ty: usize) -> usize {
        ty * self.tiles_x + tx
    }

    /// A small preview of the layer flattened over `paper`, at most `max_w` ×
    /// `max_h` and keeping the canvas proportions. Every pixel contributes, so
    /// thin lines stay visible. Returns (width, height, opaque RGBA bytes).
    pub fn thumbnail(&self, max_w: usize, max_h: usize, paper: [u8; 3]) -> (usize, usize, Vec<u8>) {
        let scale = (max_w as f64 / self.width as f64).min(max_h as f64 / self.height as f64);
        let tw = ((self.width as f64 * scale).round() as usize).clamp(1, max_w.max(1));
        let th = ((self.height as f64 * scale).round() as usize).clamp(1, max_h.max(1));
        let bin_x: Vec<usize> = (0..self.width).map(|x| x * tw / self.width).collect();
        let bin_y: Vec<usize> = (0..self.height).map(|y| y * th / self.height).collect();
        let mut count_x = vec![0u64; tw];
        let mut count_y = vec![0u64; th];
        bin_x.iter().for_each(|&b| count_x[b] += 1);
        bin_y.iter().for_each(|&b| count_y[b] += 1);

        let mut sums = vec![[0u64; 4]; tw * th];
        for ty in 0..self.tiles_y {
            for tx in 0..self.tiles_x {
                let Some(tile) = &self.tiles[self.tile_index(tx, ty)] else { continue };
                let (ox, oy) = (tx * TILE, ty * TILE);
                for y in 0..TILE.min(self.height - oy) {
                    let row = bin_y[oy + y] * tw;
                    for x in 0..TILE.min(self.width - ox) {
                        let px = tile.0[y * TILE + x];
                        if px[3] != 0 {
                            let sum = &mut sums[row + bin_x[ox + x]];
                            for ch in 0..4 {
                                sum[ch] += px[ch] as u64;
                            }
                        }
                    }
                }
            }
        }

        let mut out = Vec::with_capacity(tw * th * 4);
        for (i, sum) in sums.iter().enumerate() {
            let n = (count_x[i % tw] * count_y[i / tw]).max(1) as f64;
            let one = ONE as f64;
            let alpha = sum[3] as f64 / n / one;
            // Averaging turns thin lines faint; boosting coverage keeps line art
            // readable at preview size.
            let boost = if alpha > 0.0 { alpha.sqrt() / alpha } else { 0.0 };
            for ch in 0..3 {
                let ink = sum[ch] as f64 / n / one * boost;
                let value = ink + paper[ch] as f64 / 255.0 * (1.0 - alpha * boost);
                out.push((value.clamp(0.0, 1.0) * 255.0).round() as u8);
            }
            out.push(255);
        }
        (tw, th, out)
    }

    /// Reads one pixel; out-of-bounds and unallocated areas are transparent.
    pub fn pixel(&self, x: i32, y: i32) -> Px {
        if x < 0 || y < 0 || x as usize >= self.width || y as usize >= self.height {
            return [0; 4];
        }
        let (x, y) = (x as usize, y as usize);
        match &self.tiles[self.tile_index(x / TILE, y / TILE)] {
            Some(t) => t.0[(y % TILE) * TILE + (x % TILE)],
            None => [0; 4],
        }
    }
}
