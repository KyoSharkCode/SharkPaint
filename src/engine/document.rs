//! The document: layers, the stroke in progress, and undo history.

use std::sync::Arc;

use super::brush::BrushSettings;
use super::layer::{blend_over, IRect, Layer, LayerGroup, LayerProps, Tile, ONE, TILE};
use super::selection::{SelectMode, Selection};
use super::stroke::{ActiveStroke, StrokePoint};
use super::tools::{MoveSession, SmudgeStroke, TransformSession};

const MAX_UNDO_STEPS: usize = 200;
const EMPTY: IRect = IRect { x0: 0, y0: 0, x1: 0, y1: 0 };

/// One undoable step. Applying an entry swaps its contents with the document's,
/// which turns it into the entry that reverses itself.
enum HistoryEntry {
    /// Pixels of one layer: tile indices and the contents to swap them with.
    Tiles { layer: usize, tiles: Vec<(usize, Option<Arc<Tile>>)> },
    /// The whole layer stack (add, delete, reorder, merge). Tiles are shared,
    /// so the snapshot only costs the per-layer tile tables.
    Layers { layers: Vec<Layer>, groups: Vec<LayerGroup>, active: usize },
    /// The groups' names, visibility and opacity.
    Groups(Vec<LayerGroup>),
    /// One layer's name, visibility, opacity, blend mode or alpha lock.
    Props { layer: usize, props: LayerProps },
    /// The selection as it was.
    Selection(Option<Arc<Selection>>),
}

/// A reference image kept alongside the drawing: shown in its own panel,
/// never part of the picture or of exports.
#[derive(Clone)]
pub struct Reference {
    pub name: String,
    pub width: usize,
    pub height: usize,
    /// 8-bit RGBA, not premultiplied, row-major.
    pub rgba: Arc<Vec<u8>>,
}

/// Everything a project file holds, detached from the live document so it
/// can be written on another thread. Layers share their pixels with the
/// document, so taking one is cheap.
pub struct ProjectSnapshot {
    pub width: usize,
    pub height: usize,
    pub paper: [u8; 3],
    pub layers: Vec<Layer>,
    /// Layer groups; each layer names its group by `id`.
    pub groups: Vec<LayerGroup>,
    pub active_layer: usize,
    pub references: Vec<Reference>,
}

pub struct Document {
    pub width: usize,
    pub height: usize,
    /// Bottom layer first.
    pub(super) layers: Vec<Layer>,
    /// Layer groups, in no particular order; layers refer to them by id.
    pub(super) groups: Vec<LayerGroup>,
    pub(super) active_layer: usize,
    /// Opaque paper colour shown under the layers (sRGB).
    pub paper: [u8; 3],
    /// Brush and eraser strokes are mirrored across the canvas's vertical
    /// centre line.
    pub symmetry: bool,
    stroke: Option<ActiveStroke>,
    pub(super) smudge: Option<SmudgeStroke>,
    pub(super) moving: Option<MoveSession>,
    pub(super) transforming: Option<TransformSession>,
    undo: Vec<HistoryEntry>,
    redo: Vec<HistoryEntry>,
    dirty: IRect,
    /// Changes every time the document changes; lets the UI detect unsaved work.
    revision: u64,
    /// Counter behind default layer names.
    next_layer_number: usize,
    /// Source of layer ids and versions; never repeats.
    next_stamp: u64,
    references: Vec<Reference>,
    pub(super) selection: Option<Arc<Selection>>,
    /// Changes whenever the selection does.
    pub(super) selection_stamp: u64,
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
            groups: Vec::new(),
            active_layer: active,
            paper: [255, 255, 255],
            symmetry: false,
            stroke: None,
            smudge: None,
            moving: None,
            transforming: None,
            undo: Vec::new(),
            redo: Vec::new(),
            dirty: IRect::new(0, 0, width as i32, height as i32),
            revision: 0,
            references: Vec::new(),
            selection: None,
            selection_stamp: 0,
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
        // A layer added inside a group joins it.
        layer.group = self.active().group;
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
        self.prune_groups();
        self.mark_dirty(self.bounds());
        true
    }

    /// Moves the active layer one step up (`true`) or down the stack. At the
    /// edge of its group the step takes it out of the group; next to another
    /// group the step takes it in.
    pub fn move_layer(&mut self, up: bool) -> bool {
        let from = self.active_layer;
        let to = if up { from + 1 } else { from.wrapping_sub(1) };
        let own = self.layers[from].group;
        if to >= self.layers.len() {
            // Nothing beyond, but the layer can still step out of its group.
            if own == 0 {
                return false;
            }
            self.checkpoint_layers();
            self.layers[from].group = 0;
            self.prune_groups();
            self.mark_dirty(self.bounds());
            return true;
        }
        self.checkpoint_layers();
        let next = self.layers[to].group;
        if own == next {
            self.layers.swap(from, to);
            self.active_layer = to;
        } else if own != 0 {
            self.layers[from].group = 0;
            self.prune_groups();
        } else {
            self.layers[from].group = next;
        }
        self.mark_dirty(self.bounds());
        true
    }

    // --- Layer groups ---------------------------------------------------------

    pub fn groups(&self) -> &[LayerGroup] {
        &self.groups
    }

    pub fn group(&self, id: u64) -> Option<&LayerGroup> {
        self.groups.iter().find(|g| g.id == id)
    }

    /// Whether a layer is on show, and its opacity (0..=32768), once its
    /// group's visibility and opacity are counted in.
    pub fn layer_display(&self, layer: &Layer) -> (bool, u32) {
        let opacity = layer.props.opacity15();
        match self.group(layer.group) {
            Some(group) => {
                let faded = (opacity as f32 * group.opacity.clamp(0.0, 1.0)).round() as u32;
                (layer.props.visible && group.visible && faded > 0, faded)
            }
            None => (layer.props.visible && opacity > 0, opacity),
        }
    }

    /// Puts the active layer in a new group of its own and returns the
    /// group's id. A layer already in a group is taken out of it first, which
    /// is only possible at that group's edge; elsewhere this does nothing.
    pub fn group_active_layer(&mut self) -> Option<u64> {
        let index = self.active_layer;
        let old = self.layers[index].group;
        if old != 0 {
            let inside = |i: usize| self.layers.get(i).is_some_and(|l| l.group == old);
            if index > 0 && inside(index - 1) && inside(index + 1) {
                return None; // it would split the group in two
            }
        }
        self.checkpoint_layers();
        let id = self.stamp();
        let number = self.groups.len() + 1;
        self.groups.push(LayerGroup { id, name: format!("Grupo {number}"), visible: true, opacity: 1.0, collapsed: false });
        self.layers[index].group = id;
        self.prune_groups();
        self.mark_dirty(self.bounds());
        Some(id)
    }

    /// Dissolves a group; its layers stay where they are.
    pub fn ungroup(&mut self, id: u64) -> bool {
        if self.group(id).is_none() {
            return false;
        }
        self.checkpoint_layers();
        for layer in self.layers.iter_mut().filter(|l| l.group == id) {
            layer.group = 0;
        }
        self.prune_groups();
        self.mark_dirty(self.bounds());
        true
    }

    /// Moves a whole group one step up (`true`) or down the stack, past the
    /// layer or the group next to it.
    pub fn move_group(&mut self, id: u64, up: bool) -> bool {
        let Some(first) = self.layers.iter().position(|l| l.group == id) else { return false };
        let last = self.layers.iter().rposition(|l| l.group == id).unwrap_or(first);
        // The neighbouring block: one loose layer, or all of another group.
        let (from, to) = if up {
            if last + 1 >= self.layers.len() {
                return false;
            }
            let other = self.layers[last + 1].group;
            let end = if other == 0 { last + 1 } else { self.layers.iter().rposition(|l| l.group == other).unwrap_or(last + 1) };
            (first, end)
        } else {
            if first == 0 {
                return false;
            }
            let other = self.layers[first - 1].group;
            let start = if other == 0 { first - 1 } else { self.layers.iter().position(|l| l.group == other).unwrap_or(first - 1) };
            (start, last)
        };
        self.checkpoint_layers();
        let active = self.layers[self.active_layer].id;
        let members = last - first + 1;
        if up {
            self.layers[from..=to].rotate_left(members);
        } else {
            self.layers[from..=to].rotate_right(members);
        }
        self.active_layer = self.layers.iter().position(|l| l.id == active).unwrap_or(0);
        self.mark_dirty(self.bounds());
        true
    }

    /// Replaces a group's name, visibility and opacity as one undoable step.
    /// `coalesce` folds the change into the previous step when that step was
    /// also a group edit, so dragging a slider is undone in one go.
    pub fn set_group(&mut self, group: LayerGroup, coalesce: bool) {
        let Some(index) = self.groups.iter().position(|g| g.id == group.id) else { return };
        if self.groups[index] == group {
            return;
        }
        self.end_stroke();
        let before = self.groups.clone();
        self.groups[index] = group;
        if coalesce && matches!(self.undo.last(), Some(HistoryEntry::Groups(_))) {
            self.revision += 1;
        } else {
            self.push_undo(HistoryEntry::Groups(before));
        }
        self.mark_dirty(self.bounds());
    }

    /// Folds or unfolds a group in the layer list. Not an undo step.
    pub fn set_group_collapsed(&mut self, id: u64, collapsed: bool) {
        if let Some(group) = self.groups.iter_mut().find(|g| g.id == id) {
            group.collapsed = collapsed;
        }
    }

    /// Drops groups that no layer belongs to any more.
    fn prune_groups(&mut self) {
        let layers = &self.layers;
        self.groups.retain(|g| layers.iter().any(|l| l.group == g.id));
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
        self.prune_groups();
        let version = self.stamp();
        let below = &mut self.layers[self.active_layer];
        below.version = version;
        let (opacity, mode, clipped) = (top.props.opacity15(), top.props.blend, top.props.clip);
        for (idx, src) in top.tiles.iter().enumerate() {
            let Some(src) = src else { continue };
            if clipped && below.tiles[idx].is_none() {
                continue; // clipped to nothing here
            }
            let dst = Arc::make_mut(below.tiles[idx].get_or_insert_with(|| Arc::new(Tile::empty())));
            for (d, s) in dst.0.iter_mut().zip(src.0.iter()) {
                let mut px = d.map(u32::from);
                // A clipped layer only counts where the layer beneath has pixels.
                let opacity = if clipped { (opacity * d[3] as u32) >> 15 } else { opacity };
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

    // --- References -----------------------------------------------------------

    pub fn references(&self) -> &[Reference] {
        &self.references
    }

    /// Adds a reference image. Not part of the undo history, but it does count
    /// as an unsaved change.
    pub fn add_reference(&mut self, reference: Reference) {
        self.references.push(reference);
        self.revision += 1;
    }

    pub fn remove_reference(&mut self, index: usize) {
        if index < self.references.len() {
            self.references.remove(index);
            self.revision += 1;
        }
    }

    /// The document's contents as they are now, for saving.
    pub fn snapshot(&self) -> ProjectSnapshot {
        ProjectSnapshot {
            width: self.width,
            height: self.height,
            paper: self.paper,
            layers: self.layers.clone(),
            groups: self.groups.clone(),
            active_layer: self.active_layer,
            references: self.references.clone(),
        }
    }

    /// Builds a document from loaded contents, with a clean history.
    pub fn from_snapshot(snapshot: ProjectSnapshot) -> Self {
        let mut doc = Document::from_layers(snapshot.width, snapshot.height, snapshot.layers, snapshot.active_layer);
        // Groups get fresh ids that cannot clash with anything made later.
        for mut group in snapshot.groups {
            let id = doc.stamp();
            for layer in doc.layers.iter_mut().filter(|l| l.group == group.id) {
                layer.group = id;
            }
            group.id = id;
            doc.groups.push(group);
        }
        // A layer naming a group that is not there, or a group split by other
        // layers, would break the list: such layers go loose.
        let known: Vec<u64> = doc.groups.iter().map(|g| g.id).collect();
        let mut closed: Vec<u64> = Vec::new();
        let mut previous = 0;
        for layer in &mut doc.layers {
            if layer.group != 0 && (!known.contains(&layer.group) || (layer.group != previous && closed.contains(&layer.group))) {
                layer.group = 0;
            }
            if previous != 0 && layer.group != previous {
                closed.push(previous);
            }
            previous = layer.group;
        }
        doc.prune_groups();
        doc.paper = snapshot.paper;
        doc.references = snapshot.references;
        doc
    }

    // --- Selection ------------------------------------------------------------

    pub fn selection(&self) -> Option<&Selection> {
        self.selection.as_deref()
    }

    /// A counter that changes whenever the selection changes.
    pub fn selection_stamp(&self) -> u64 {
        self.selection_stamp
    }

    /// Selects the inside of a closed polygon given in canvas coordinates.
    /// A polygon that covers nothing clears the selection.
    pub fn select_polygon(&mut self, points: &[(f32, f32)]) {
        self.select_polygon_with(points, SelectMode::Replace);
    }

    /// Selects the inside of a closed polygon, combined with the selection
    /// already there as `mode` says.
    pub fn select_polygon_with(&mut self, points: &[(f32, f32)], mode: SelectMode) {
        self.end_stroke();
        let shape = Selection::from_polygon(self.width, self.height, points);
        let next = match (shape, mode) {
            (shape, SelectMode::Replace) => shape,
            // Adding or taking away nothing changes nothing.
            (None, SelectMode::Add | SelectMode::Subtract) => return,
            (None, SelectMode::Intersect) => None,
            (Some(shape), _) => Selection::combined(self.selection.as_deref(), self.width, self.height, &shape.mask, mode),
        };
        self.set_selection(next.map(Arc::new));
    }

    /// Selects what was not selected, and the other way round.
    pub fn invert_selection(&mut self) {
        self.end_stroke();
        match self.selection.clone() {
            Some(selection) => self.set_selection(selection.inverted().map(Arc::new)),
            None => self.select_all(),
        }
    }

    /// Replaces the selection, remembering the old one for undo.
    pub(super) fn set_selection(&mut self, next: Option<Arc<Selection>>) {
        if self.selection.is_none() && next.is_none() {
            return;
        }
        let previous = std::mem::replace(&mut self.selection, next);
        self.selection_stamp += 1;
        // The selection is not part of the drawing: changing it leaves the
        // document as saved as it was.
        self.undo.push(HistoryEntry::Selection(previous));
        self.redo.clear();
        if self.undo.len() > MAX_UNDO_STEPS {
            self.undo.remove(0);
        }
    }

    pub fn select_all(&mut self) {
        let (w, h) = (self.width as f32, self.height as f32);
        self.select_polygon(&[(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)]);
    }

    pub fn deselect(&mut self) {
        self.end_stroke();
        self.set_selection(None);
    }

    // --- Strokes --------------------------------------------------------------

    /// A brush stroke, smudge or layer move is in progress.
    pub fn is_stroking(&self) -> bool {
        self.stroke.is_some() || self.smudge.is_some() || self.moving.is_some() || self.transforming.is_some()
    }

    pub fn begin_stroke(&mut self, brush: &BrushSettings, color: [u8; 3], erase: bool, p: StrokePoint) {
        if self.stroke.is_some() {
            self.end_stroke();
        }
        let li = self.active_layer;
        let selection = self.selection.clone();
        let mirror_x = self.symmetry.then_some(self.width as f32 * 0.5);
        let (stroke, rect) =
            ActiveStroke::begin(li, &mut self.layers[li], brush, color, erase, selection, mirror_x, p);
        self.stroke = Some(stroke);
        self.mark_dirty(rect);
    }

    pub fn stroke_to(&mut self, p: StrokePoint) {
        if self.smudge.is_some() {
            self.smudge_to(p);
        }
        if let Some(stroke) = &mut self.stroke {
            let rect = stroke.line_to(&mut self.layers[stroke.layer], p);
            self.mark_dirty(rect);
        }
    }

    /// Finishes whatever is in progress (stroke, smudge or layer move) as one
    /// undoable step.
    pub fn end_stroke(&mut self) {
        self.end_smudge();
        self.end_layer_move();
        self.end_transform();
        if let Some(mut stroke) = self.stroke.take() {
            let rect = stroke.finish(&mut self.layers[stroke.layer]);
            self.mark_dirty(rect);
            let layer = stroke.layer;
            let tiles = stroke.into_saved();
            self.commit_tiles(layer, tiles);
        }
    }

    /// Records changed tiles of a layer as one undoable step.
    pub(super) fn commit_tiles(&mut self, layer: usize, tiles: Vec<(usize, Option<Arc<Tile>>)>) {
        if !tiles.is_empty() {
            self.layers[layer].version = self.stamp();
            self.push_undo(HistoryEntry::Tiles { layer, tiles });
        }
    }

    /// Abandons the stroke in progress, leaving no trace and no undo step.
    pub fn cancel_stroke(&mut self) {
        self.cancel_smudge();
        self.cancel_layer_move();
        self.cancel_transform();
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

        // Whether each layer shows and how strongly, groups counted in.
        let display: Vec<(bool, u32)> = self.layers.iter().map(|layer| self.layer_display(layer)).collect();
        let (tx0, tx1) = (rect.x0 as usize / TILE, (rect.x1 as usize - 1) / TILE);
        let (ty0, ty1) = (rect.y0 as usize / TILE, (rect.y1 as usize - 1) / TILE);
        for ty in ty0..=ty1 {
            for tx in tx0..=tx1 {
                let (ox, oy) = ((tx * TILE) as i32, (ty * TILE) as i32);
                let x0 = rect.x0.max(ox);
                let x1 = rect.x1.min(ox + TILE as i32);
                let y0 = rect.y0.max(oy);
                let y1 = rect.y1.min(oy + TILE as i32);
                // Each drawable layer's tile, with the tile and opacity of the
                // layer it is clipped to, if any.
                let mut tiles: Vec<(&Tile, &LayerProps, u32, Option<(&Tile, u32)>)> = Vec::new();
                let mut clip_base: Option<(&Tile, u32)> = None;
                for (index, layer) in self.layers.iter().enumerate() {
                    let props = &layer.props;
                    let (shown, opacity) = display[index];
                    let tile = layer.tiles[layer.tile_index(tx, ty)].as_deref();
                    if props.clip && index > 0 {
                        if let (true, Some(tile), Some(base)) = (shown, tile, clip_base) {
                            tiles.push((tile, props, opacity, Some(base)));
                        }
                    } else {
                        // A hidden or empty base hides everything clipped to it.
                        clip_base = tile.filter(|_| shown).map(|t| (t, opacity));
                        if let (true, Some(tile)) = (shown, tile) {
                            tiles.push((tile, props, opacity, None));
                        }
                    }
                }
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
                        for (tile, props, opacity, base) in &tiles {
                            let mut opacity = *opacity;
                            if let Some((base, base_opacity)) = base {
                                opacity = (opacity * ((base.0[i][3] as u32 * base_opacity) >> 15)) >> 15;
                            }
                            blend_over(&mut c, tile.0[i], opacity, props.blend);
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
        self.push_undo(HistoryEntry::Layers {
            layers: self.layers.clone(),
            groups: self.groups.clone(),
            active: self.active_layer,
        });
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
        if let HistoryEntry::Selection(selection) = entry {
            std::mem::swap(&mut self.selection, selection);
            self.selection_stamp += 1;
            return;
        }
        self.revision += 1;
        match entry {
            HistoryEntry::Selection(_) => {}
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
            HistoryEntry::Groups(groups) => {
                std::mem::swap(&mut self.groups, groups);
                self.mark_dirty(self.bounds());
            }
            HistoryEntry::Layers { layers, groups, active } => {
                std::mem::swap(&mut self.groups, groups);
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

    pub(super) fn mark_dirty(&mut self, rect: IRect) {
        self.dirty = self.dirty.union(&rect.intersect(&self.bounds()));
    }
}
