//! The document: layers, the stroke in progress, and undo history.

use std::sync::Arc;

use super::brush::BrushSettings;
use super::layer::{IRect, Layer, Tile, ONE, TILE};
use super::stroke::{ActiveStroke, StrokePoint};

const MAX_UNDO_STEPS: usize = 200;
const EMPTY: IRect = IRect { x0: 0, y0: 0, x1: 0, y1: 0 };

/// One undoable step: tiles of a layer and the contents to swap them with.
struct HistoryEntry {
    layer: usize,
    tiles: Vec<(usize, Option<Arc<Tile>>)>,
}

pub struct Document {
    pub width: usize,
    pub height: usize,
    pub layers: Vec<Layer>,
    pub active_layer: usize,
    /// Opaque paper colour shown under the layers (sRGB).
    pub paper: [u8; 3],
    stroke: Option<ActiveStroke>,
    undo: Vec<HistoryEntry>,
    redo: Vec<HistoryEntry>,
    dirty: IRect,
}

impl Document {
    pub fn new(width: usize, height: usize) -> Self {
        let (width, height) = (width.max(1), height.max(1));
        Document {
            width,
            height,
            layers: vec![Layer::new(width, height)],
            active_layer: 0,
            paper: [255, 255, 255],
            stroke: None,
            undo: Vec::new(),
            redo: Vec::new(),
            dirty: IRect::new(0, 0, width as i32, height as i32),
        }
    }

    pub fn bounds(&self) -> IRect {
        IRect::new(0, 0, self.width as i32, self.height as i32)
    }

    pub fn is_stroking(&self) -> bool {
        self.stroke.is_some()
    }

    pub fn begin_stroke(&mut self, brush: &BrushSettings, color: [u8; 3], erase: bool, p: StrokePoint) {
        if self.stroke.is_some() {
            self.end_stroke();
        }
        let li = self.active_layer;
        let (stroke, rect) = ActiveStroke::begin(li, &mut self.layers[li], brush, color, erase, p);
        self.stroke = Some(stroke);
        self.mark_dirty(rect);
    }

    pub fn stroke_to(&mut self, p: StrokePoint) {
        if let Some(stroke) = &mut self.stroke {
            let rect = stroke.line_to(&mut self.layers[stroke.layer], p);
            self.mark_dirty(rect);
        }
    }

    pub fn end_stroke(&mut self) {
        if let Some(mut stroke) = self.stroke.take() {
            let rect = stroke.finish(&mut self.layers[stroke.layer]);
            self.mark_dirty(rect);
            let layer = stroke.layer;
            let tiles = stroke.into_saved();
            if !tiles.is_empty() {
                self.push_undo(HistoryEntry { layer, tiles });
            }
        }
    }

    /// Empties the active layer as an undoable step.
    pub fn clear_active_layer(&mut self) {
        self.end_stroke();
        let li = self.active_layer;
        let tiles: Vec<_> = self.layers[li]
            .tiles
            .iter_mut()
            .enumerate()
            .filter_map(|(i, t)| t.take().map(|old| (i, Some(old))))
            .collect();
        if !tiles.is_empty() {
            self.push_undo(HistoryEntry { layer: li, tiles });
            self.mark_dirty(self.bounds());
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo(&mut self) -> bool {
        self.end_stroke();
        match self.undo.pop() {
            Some(mut entry) => {
                self.swap_entry(&mut entry);
                self.redo.push(entry);
                true
            }
            None => false,
        }
    }

    pub fn redo(&mut self) -> bool {
        self.end_stroke();
        match self.redo.pop() {
            Some(mut entry) => {
                self.swap_entry(&mut entry);
                self.undo.push(entry);
                true
            }
            None => false,
        }
    }

    /// Returns and resets the area changed since the last call.
    pub fn take_dirty(&mut self) -> Option<IRect> {
        let d = std::mem::replace(&mut self.dirty, EMPTY);
        (!d.is_empty()).then_some(d)
    }

    /// Flattens `rect` (clipped to the canvas by the caller) over the paper into
    /// opaque 8-bit RGBA, row-major, `rect.width() * rect.height() * 4` bytes.
    pub fn render_rgba8(&self, rect: IRect) -> Vec<u8> {
        let rect = rect.intersect(&self.bounds());
        let (w, h) = (rect.width(), rect.height());
        let mut out = vec![255u8; w * h * 4];
        if rect.is_empty() {
            return out;
        }
        let paper = self.paper.map(|v| (v as u32 * ONE + 127) / 255);
        let to8 = |v: u32| ((v.min(ONE) * 255 + ONE / 2) >> 15) as u8;
        let paper8 = [to8(paper[0]), to8(paper[1]), to8(paper[2])];

        let (tx0, tx1) = (rect.x0 as usize / TILE, (rect.x1 as usize - 1) / TILE);
        let (ty0, ty1) = (rect.y0 as usize / TILE, (rect.y1 as usize - 1) / TILE);
        for ty in ty0..=ty1 {
            for tx in tx0..=tx1 {
                let (ox, oy) = ((tx * TILE) as i32, (ty * TILE) as i32);
                let x0 = rect.x0.max(ox);
                let x1 = rect.x1.min(ox + TILE as i32);
                let y0 = rect.y0.max(oy);
                let y1 = rect.y1.min(oy + TILE as i32);
                let tiles: Vec<&Tile> = self
                    .layers
                    .iter()
                    .filter_map(|l| l.tiles[l.tile_index(tx, ty)].as_deref())
                    .collect();
                for y in y0..y1 {
                    let row = (y - oy) as usize * TILE;
                    let out_row = (y - rect.y0) as usize * w;
                    for x in x0..x1 {
                        let o = (out_row + (x - rect.x0) as usize) * 4;
                        if tiles.is_empty() {
                            out[o..o + 3].copy_from_slice(&paper8);
                            continue;
                        }
                        let i = row + (x - ox) as usize;
                        let mut c = paper;
                        for t in &tiles {
                            let p = t.0[i];
                            let keep = ONE - p[3] as u32;
                            for ch in 0..3 {
                                c[ch] = p[ch] as u32 + ((c[ch] * keep + ONE / 2) >> 15);
                            }
                        }
                        out[o] = to8(c[0]);
                        out[o + 1] = to8(c[1]);
                        out[o + 2] = to8(c[2]);
                    }
                }
            }
        }
        out
    }

    fn push_undo(&mut self, entry: HistoryEntry) {
        self.undo.push(entry);
        self.redo.clear();
        if self.undo.len() > MAX_UNDO_STEPS {
            self.undo.remove(0);
        }
    }

    fn swap_entry(&mut self, entry: &mut HistoryEntry) {
        let layer = &mut self.layers[entry.layer];
        let tiles_x = layer.tiles_x;
        let mut rect = EMPTY;
        for (idx, saved) in entry.tiles.iter_mut() {
            std::mem::swap(&mut layer.tiles[*idx], saved);
            let (tx, ty) = ((*idx % tiles_x) as i32, (*idx / tiles_x) as i32);
            let t = TILE as i32;
            rect = rect.union(&IRect::new(tx * t, ty * t, (tx + 1) * t, (ty + 1) * t));
        }
        self.mark_dirty(rect);
    }

    fn mark_dirty(&mut self, rect: IRect) {
        self.dirty = self.dirty.union(&rect.intersect(&self.bounds()));
    }
}
