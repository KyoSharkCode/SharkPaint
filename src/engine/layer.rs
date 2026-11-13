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

pub struct Layer {
    pub width: usize,
    pub height: usize,
    pub tiles_x: usize,
    pub tiles_y: usize,
    pub tiles: Vec<Option<Arc<Tile>>>,
}

impl Layer {
    pub fn new(width: usize, height: usize) -> Self {
        let tiles_x = width.div_ceil(TILE);
        let tiles_y = height.div_ceil(TILE);
        Layer { width, height, tiles_x, tiles_y, tiles: vec![None; tiles_x * tiles_y] }
    }

    pub fn bounds(&self) -> IRect {
        IRect::new(0, 0, self.width as i32, self.height as i32)
    }

    pub fn tile_index(&self, tx: usize, ty: usize) -> usize {
        ty * self.tiles_x + tx
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
