//! The document: layers, the stroke in progress, and undo history.

use std::sync::Arc;

use super::brush::BrushSettings;
use super::layer::{blend_over, IRect, Layer, LayerProps, Tile, ONE, TILE};
use super::stroke::{ActiveStroke, StrokePoint};

const MAX_UNDO_STEPS: usize = 200;
const EMPTY: IRect = IRect { x0: 0, y0: 0, x1: 0, y1: 0 };

/// One undoable step. Applying an entry swaps its contents with the document's,
/// which turns it into the entry that reverses itself.
enum HistoryEntry {
    /// Pixels of one layer: tile indices and the contents to swap them with.
    Tiles { layer: usize, tiles: Vec<(usize, Option<Arc<Tile>>)> },
    /// The whole layer stack (add, delete, reorder, merge). Tiles are shared,
    /// so the snapshot only costs the per-layer tile tables.
    Layers { layers: Vec<Layer>, active: usize },
    /// One layer's name, visibility, opacity, blend mode or alpha lock.
    Props { layer: usize, props: LayerProps },
}

pub struct Document {
    pub width: usize,
    pub height: usize,
    /// Bottom layer first.
    layers: Vec<Layer>,
    active_layer: usize,
    /// Opaque paper colour shown under the layers (sRGB).
    pub paper: [u8; 3],
    stroke: Option<ActiveStroke>,
    undo: Vec<HistoryEntry>,
    redo: Vec<HistoryEntry>,
    dirty: IRect,
    /// Changes every time the document changes; lets the UI detect unsaved work.
    revision: u64,
    /// Counter behind default layer names.
    next_layer_number: usize,
    /// Source of layer ids and versions; never repeats.
    next_stamp: u64,
}

impl Document {
    pub fn new(width: usize, height: usize) -> Self {
        let (width, height) = (width.max(1), height.max(1));
        Document::from_layers(width, height, vec![Layer::new(width, height, "Capa 1")], 0)
    }

    /// Builds a document from an existing stack (bottom layer first).
    pub fn from_layers(width: usize, height: usize, layers: Vec<Layer>, active: usize) -> Self {
        assert!(!layers.is_empty() && active < layers.len());
        let mut layers = layers;
        for (i, layer) in layers.iter_mut().enumerate() {
            layer.id = i as u64 + 1;
            layer.version = layer.id;
        }
        Document {
            next_stamp: layers.len() as u64 + 1,
            width,
            height,
            next_layer_number: layers.len() + 1,
            layers,
            active_layer: active,
            paper: [255, 255, 255],
            stroke: None,
            undo: Vec::new(),
            redo: Vec::new(),
            dirty: IRect::new(0, 0, width as i32, height as i32),
            revision: 0,
        }
    }

    pub fn bounds(&self) -> IRect {
        IRect::new(0, 0, self.width as i32, self.height as i32)
    }

    /// A counter that changes whenever the document changes.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    fn stamp(&mut self) -> u64 {
        self.next_stamp += 1;
        self.next_stamp
    }

    // --- Layers ---------------------------------------------------------------

    /// The layer stack, bottom layer first.
    pub fn layers(&self) -> &[Layer] {
        &self.layers
    }

    pub fn active_layer(&self) -> usize {
        self.active_layer
    }

    pub fn active(&self) -> &Layer {
        &self.layers[self.active_layer]
    }

    pub fn set_active_layer(&mut self, index: usize) {
        if index < self.layers.len() && index != self.active_layer {
            self.end_stroke();
            self.active_layer = index;
        }
    }

    /// Adds an empty layer above the active one and selects it.
    pub fn add_layer(&mut self) {
        self.checkpoint_layers();
        let name = format!("Capa {}", self.next_layer_number);
        self.next_layer_number += 1;
        let mut layer = Layer::new(self.width, self.height, name);
        layer.id = self.stamp();
        layer.version = layer.id;
        self.active_layer += 1;
        self.layers.insert(self.active_layer, layer);
    }

    /// Copies the active layer above itself and selects the copy.
    pub fn duplicate_layer(&mut self) {
        self.checkpoint_layers();
        let mut copy = self.active().clone();
        copy.props.name = format!("{} copia", copy.props.name);
        copy.id = self.stamp();
        self.active_layer += 1;
        self.layers.insert(self.active_layer, copy);
        self.mark_dirty(self.bounds());
    }

    /// Removes the active layer. The last remaining layer cannot be removed.
    pub fn delete_layer(&mut self) -> bool {
        if self.layers.len() < 2 {
            return false;
        }
        self.checkpoint_layers();
        self.layers.remove(self.active_layer);
        self.active_layer = self.active_layer.saturating_sub(1);
        self.mark_dirty(self.bounds());
        true
    }

    /// Moves the active layer one step up (`true`) or down the stack.
    pub fn move_layer(&mut self, up: bool) -> bool {
        let from = self.active_layer;
        let to = if up { from + 1 } else { from.wrapping_sub(1) };
        if to >= self.layers.len() {
            return false;
        }
        self.checkpoint_layers();
        self.layers.swap(from, to);
        self.active_layer = to;
        self.mark_dirty(self.bounds());
        true
    }

    /// Flattens the active layer into the one beneath it, baking in the active
    /// layer's opacity and blend mode.
    pub fn merge_down(&mut self) -> bool {
        if self.active_layer == 0 {
            return false;
        }
        self.checkpoint_layers();
        let top = self.layers.remove(self.active_layer);
        self.active_layer -= 1;
        let version = self.stamp();
        let below = &mut self.layers[self.active_layer];
        below.version = version;
        let (opacity, mode) = (top.props.opacity15(), top.props.blend);
        for (idx, src) in top.tiles.iter().enumerate() {
            let Some(src) = src else { continue };
            let dst = Arc::make_mut(below.tiles[idx].get_or_insert_with(|| Arc::new(Tile::empty())));
            for (d, s) in dst.0.iter_mut().zip(src.0.iter()) {
                let mut px = d.map(u32::from);
                blend_over(&mut px, *s, opacity, mode);
                *d = px.map(|c| c as u16);
            }
        }
        self.mark_dirty(self.bounds());
        true
    }

    /// Replaces a layer's properties as one undoable step. `coalesce` folds the
    /// change into the previous step when that step edited the same layer's
    /// properties, so dragging a slider is undone in one go.
    pub fn set_layer_props(&mut self, index: usize, props: LayerProps, coalesce: bool) {
        if index >= self.layers.len() || self.layers[index].props == props {
            return;
        }
        self.end_stroke();
        let same_edit = coalesce
            && matches!(self.undo.last(), Some(HistoryEntry::Props { layer, .. }) if *layer == index);
        let old = std::mem::replace(&mut self.layers[index].props, props);
        if same_edit {
            self.revision += 1;
        } else {
            self.push_undo(HistoryEntry::Props { layer: index, props: old });
        }
        self.mark_dirty(self.bounds());
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
            self.layers[li].version = self.stamp();
            self.push_undo(HistoryEntry::Tiles { layer: li, tiles });
            self.mark_dirty(self.bounds());
        }
    }

    // --- Strokes --------------------------------------------------------------

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
                self.layers[layer].version = self.stamp();
                self.push_undo(HistoryEntry::Tiles { layer, tiles });
            }
        }
    }

    /// Abandons the stroke in progress, leaving no trace and no undo step.
    pub fn cancel_stroke(&mut self) {
        if let Some(stroke) = self.stroke.take() {
            let layer = stroke.layer;
            let rect = stroke.revert(&mut self.layers[layer]);
            self.mark_dirty(rect);
        }
    }

    // --- History --------------------------------------------------------------

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

    // --- Rendering ------------------------------------------------------------

    /// Returns and resets the area changed since the last call.
    pub fn take_dirty(&mut self) -> Option<IRect> {
        let d = std::mem::replace(&mut self.dirty, EMPTY);
        (!d.is_empty()).then_some(d)
    }

    /// Flattens `rect` (clipped to the canvas) over the paper into opaque 8-bit
    /// RGBA, row-major, `rect.width() * rect.height() * 4` bytes.
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
                let tiles: Vec<(&Tile, &LayerProps)> = self
                    .layers
                    .iter()
                    .filter(|l| l.props.visible && l.props.opacity > 0.0)
                    .filter_map(|l| l.tiles[l.tile_index(tx, ty)].as_deref().map(|t| (t, &l.props)))
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
                        let mut c = [paper[0], paper[1], paper[2], ONE];
                        for (tile, props) in &tiles {
                            blend_over(&mut c, tile.0[i], props.opacity15(), props.blend);
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

    // --- Internals ------------------------------------------------------------

    /// Records the layer stack as it is now, before a structural change.
    fn checkpoint_layers(&mut self) {
        self.end_stroke();
        self.push_undo(HistoryEntry::Layers { layers: self.layers.clone(), active: self.active_layer });
    }

    fn push_undo(&mut self, entry: HistoryEntry) {
        self.revision += 1;
        self.undo.push(entry);
        self.redo.clear();
        if self.undo.len() > MAX_UNDO_STEPS {
            self.undo.remove(0);
        }
    }

    fn swap_entry(&mut self, entry: &mut HistoryEntry) {
        self.revision += 1;
        match entry {
            HistoryEntry::Tiles { layer, tiles } => {
                self.next_stamp += 1;
                let layer = &mut self.layers[*layer];
                layer.version = self.next_stamp;
                let tiles_x = layer.tiles_x;
                let mut rect = EMPTY;
                for (idx, saved) in tiles.iter_mut() {
                    std::mem::swap(&mut layer.tiles[*idx], saved);
                    let (tx, ty) = ((*idx % tiles_x) as i32, (*idx / tiles_x) as i32);
                    let t = TILE as i32;
                    rect = rect.union(&IRect::new(tx * t, ty * t, (tx + 1) * t, (ty + 1) * t));
                }
                self.mark_dirty(rect);
            }
            HistoryEntry::Layers { layers, active } => {
                std::mem::swap(&mut self.layers, layers);
                std::mem::swap(&mut self.active_layer, active);
                self.mark_dirty(self.bounds());
            }
            HistoryEntry::Props { layer, props } => {
                std::mem::swap(&mut self.layers[*layer].props, props);
                self.mark_dirty(self.bounds());
            }
        }
    }

    fn mark_dirty(&mut self, rect: IRect) {
        self.dirty = self.dirty.union(&rect.intersect(&self.bounds()));
    }
}
