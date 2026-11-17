//! Layer groups: folders of layers that can hold other groups.
//!
//! The layers stay in one flat stack; each names the group it is directly in,
//! and each group names the group it is in, if any. What holds it together is
//! one rule: the layers inside a group, at any depth, are next to each other
//! in the stack. Every operation here keeps it.

use super::document::Document;
use super::layer::{BlendMode, Layer, LayerGroup, ONE};

/// A group whose layers are combined apart and blended in as one.
#[derive(Clone, Copy)]
pub(super) struct Isolated {
    pub id: u64,
    /// 0..=32768, with the opacity of the plain groups around it counted in.
    pub opacity: u32,
    pub blend: BlendMode,
}

/// The id of the group `group` is in; zero for none.
fn parent_of(groups: &[LayerGroup], group: u64) -> u64 {
    groups.iter().find(|g| g.id == group).map_or(0, |g| g.parent)
}

/// The groups `group` is in, itself first and the outermost last. Stops at a
/// group that is not there or at a loop, which never happen in a sound document.
fn chain_of(groups: &[LayerGroup], group: u64) -> Vec<u64> {
    let mut chain = Vec::new();
    let mut at = group;
    while at != 0 && !chain.contains(&at) && groups.iter().any(|g| g.id == at) {
        chain.push(at);
        at = parent_of(groups, at);
    }
    chain
}

/// Whether the layers inside every group are next to each other, and every
/// group's chain of parents ends without going round in a loop.
pub(super) fn nesting_holds(layers: &[Layer], groups: &[LayerGroup]) -> bool {
    for group in groups {
        // Climbing from a sound group reaches the top in fewer steps than there are groups.
        let mut at = group.parent;
        for _ in 0..groups.len() {
            if at == 0 {
                break;
            }
            if !groups.iter().any(|g| g.id == at) {
                return false;
            }
            at = parent_of(groups, at);
        }
        if at != 0 {
            return false;
        }
    }
    let chains: Vec<Vec<u64>> = layers.iter().map(|layer| chain_of(groups, layer.group)).collect();
    if layers.iter().zip(&chains).any(|(layer, chain)| layer.group != 0 && chain.is_empty()) {
        return false; // a layer names a group that is not there
    }
    groups.iter().all(|group| {
        let inside = |chain: &Vec<u64>| chain.contains(&group.id);
        match (chains.iter().position(inside), chains.iter().rposition(inside)) {
            (Some(first), Some(last)) => chains[first..=last].iter().all(inside),
            _ => true,
        }
    })
}

impl Document {
    pub fn groups(&self) -> &[LayerGroup] {
        &self.groups
    }

    pub fn group(&self, id: u64) -> Option<&LayerGroup> {
        self.groups.iter().find(|g| g.id == id)
    }

    /// The groups a group is in, itself first and the outermost last. Pass a
    /// layer's `group` to get everything the layer is inside.
    pub fn group_chain(&self, group: u64) -> Vec<u64> {
        chain_of(&self.groups, group)
    }

    /// How many groups deep a group is: one for a group at the top level.
    pub fn group_depth(&self, group: u64) -> usize {
        self.group_chain(group).len()
    }

    /// Whether `group` is `ancestor` or lies somewhere inside it.
    pub fn group_within(&self, group: u64, ancestor: u64) -> bool {
        self.group_chain(group).contains(&ancestor)
    }

    /// The first and last places in the stack of the layers inside a group,
    /// at any depth.
    pub fn group_range(&self, id: u64) -> Option<(usize, usize)> {
        let inside = |layer: &Layer| self.group_within(layer.group, id);
        Some((self.layers.iter().position(inside)?, self.layers.iter().rposition(inside)?))
    }

    /// Whether a layer is on show, and its opacity (0..=32768), once the
    /// groups it is in are counted in. Groups that blend as one keep their
    /// opacity for the combined result; see [`Document::isolation`].
    pub fn layer_display(&self, layer: &Layer) -> (bool, u32) {
        let mut shown = layer.props.visible;
        let mut opacity = layer.props.opacity.clamp(0.0, 1.0);
        let mut plain = true;
        for id in self.group_chain(layer.group) {
            let Some(group) = self.group(id) else { continue };
            shown &= group.visible && group.opacity > 0.0;
            // Plain groups fade the layer itself, up to the first group that
            // gathers its layers apart; from there on they fade that group.
            plain &= group.blend == BlendMode::Normal;
            if plain {
                opacity *= group.opacity.clamp(0.0, 1.0);
            }
        }
        let opacity = (opacity * ONE as f32).round() as u32;
        (shown && opacity > 0, opacity)
    }

    /// The groups a layer is gathered into before reaching the picture: those
    /// around it that blend as one, the outermost first.
    pub(super) fn isolation(&self, layer: &Layer) -> Vec<Isolated> {
        let mut found: Vec<(u64, BlendMode, f32)> = Vec::new();
        for id in self.group_chain(layer.group) {
            let Some(group) = self.group(id) else { continue };
            let opacity = group.opacity.clamp(0.0, 1.0);
            if group.blend != BlendMode::Normal {
                found.push((group.id, group.blend, opacity));
            } else if let Some(inner) = found.last_mut() {
                // A plain group around one that blends as one fades it.
                inner.2 *= opacity;
            }
        }
        found
            .into_iter()
            .rev()
            .map(|(id, blend, opacity)| Isolated { id, blend, opacity: (opacity * ONE as f32).round() as u32 })
            .collect()
    }

    fn new_group(&mut self, parent: u64) -> u64 {
        let id = self.next_version();
        let number = self.groups.len() + 1;
        self.groups.push(LayerGroup {
            id,
            parent,
            name: format!("Grupo {number}"),
            visible: true,
            opacity: 1.0,
            blend: BlendMode::Normal,
            collapsed: false,
        });
        id
    }

    /// Puts the active layer in a new group of its own, inside the group it
    /// was in, and returns the new group's id.
    pub fn group_active_layer(&mut self) -> Option<u64> {
        self.checkpoint_layers();
        let index = self.active_layer;
        let id = self.new_group(self.layers[index].group);
        self.layers[index].group = id;
        self.mark_dirty(self.bounds());
        Some(id)
    }

    /// Wraps a group in a new group, which takes its place inside whatever
    /// the group was in. Returns the new group's id.
    pub fn group_group(&mut self, id: u64) -> Option<u64> {
        let parent = self.group(id)?.parent;
        self.checkpoint_layers();
        let outer = self.new_group(parent);
        if let Some(group) = self.groups.iter_mut().find(|g| g.id == id) {
            group.parent = outer;
        }
        self.mark_dirty(self.bounds());
        Some(outer)
    }

    /// Dissolves a group; its layers and the groups in it stay where they
    /// are, one level up.
    pub fn ungroup(&mut self, id: u64) -> bool {
        let Some(parent) = self.group(id).map(|g| g.parent) else { return false };
        self.checkpoint_layers();
        for layer in self.layers.iter_mut().filter(|l| l.group == id) {
            layer.group = parent;
        }
        for group in self.groups.iter_mut().filter(|g| g.parent == id) {
            group.parent = parent;
        }
        self.groups.retain(|g| g.id != id);
        self.prune_groups();
        self.mark_dirty(self.bounds());
        true
    }

    /// Moves the active layer one step up (`true`) or down the stack. Next to
    /// a group it steps into it; at the edge of its own group it steps out,
    /// one level at a time.
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
            self.layers[from].group = parent_of(&self.groups, own);
            self.prune_groups();
            self.mark_dirty(self.bounds());
            return true;
        }
        self.checkpoint_layers();
        let next = self.layers[to].group;
        let towards = self.group_chain(next);
        if own == next {
            self.layers.swap(from, to);
            self.active_layer = to;
        } else if own == 0 || towards.contains(&own) {
            // The neighbour is in a group inside this layer's own: step into
            // the one that starts right here.
            let own_at = towards.iter().position(|g| *g == own).unwrap_or(towards.len());
            self.layers[from].group = towards[own_at - 1];
        } else {
            self.layers[from].group = parent_of(&self.groups, own);
            self.prune_groups();
        }
        self.mark_dirty(self.bounds());
        true
    }

    /// Moves a whole group one step up (`true`) or down the stack: past the
    /// layer or the group next to it, or, at the edge of the group it is in,
    /// out of that group.
    pub fn move_group(&mut self, id: u64, up: bool) -> bool {
        let Some((first, last)) = self.group_range(id) else { return false };
        let parent = parent_of(&self.groups, id);
        let beyond = if up { last + 1 } else { first.wrapping_sub(1) };
        let Some(neighbour) = self.layers.get(beyond).map(|layer| layer.group) else {
            // The end of the stack: all that is left is to step out.
            if parent == 0 {
                return false;
            }
            self.checkpoint_layers();
            let outer = parent_of(&self.groups, parent);
            if let Some(group) = self.groups.iter_mut().find(|g| g.id == id) {
                group.parent = outer;
            }
            self.mark_dirty(self.bounds());
            return true;
        };
        let around = self.group_chain(neighbour);
        self.checkpoint_layers();
        if parent != 0 && !around.contains(&parent) {
            // The neighbour is outside the group this one is in: step out.
            let outer = parent_of(&self.groups, parent);
            if let Some(group) = self.groups.iter_mut().find(|g| g.id == id) {
                group.parent = outer;
            }
            self.mark_dirty(self.bounds());
            return true;
        }
        // The neighbour is a layer beside this group, or lies in a group
        // beside it: go past the whole of that.
        let parent_at = around.iter().position(|g| *g == parent).unwrap_or(around.len());
        let (from, to) = match parent_at.checked_sub(1).and_then(|at| self.group_range(around[at])) {
            Some((_, other_last)) if up => (first, other_last.max(beyond)),
            Some((other_first, _)) => (other_first.min(beyond), last),
            None if up => (first, beyond),
            None => (beyond, last),
        };
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

    /// Moves a layer to a gap in the stack and into a group (zero for none), as
    /// when it is dragged in the layer list. Gap `g` is the place just under
    /// the layer now at index `g`; gap `len` is the top. Nothing happens, and
    /// `false` comes back, if the layer would not move or the move would
    /// leave a group in two pieces.
    pub fn place_layer(&mut self, from: usize, gap: usize, group: u64) -> bool {
        if from >= self.layers.len() || gap > self.layers.len() || (group != 0 && self.group(group).is_none()) {
            return false;
        }
        let to = if gap > from { gap - 1 } else { gap };
        if to == from && self.layers[from].group == group {
            return false;
        }
        let mut layers = self.layers.clone();
        let mut layer = layers.remove(from);
        layer.group = group;
        layers.insert(to, layer);
        self.restack(layers, self.groups.clone())
    }

    /// Moves a whole group, with everything in it, to a gap in the stack and
    /// into another group (zero for none); see [`Document::place_layer`]. A
    /// group cannot go inside itself.
    pub fn place_group(&mut self, id: u64, gap: usize, parent: u64) -> bool {
        let Some((first, last)) = self.group_range(id) else { return false };
        if gap > self.layers.len() || (parent != 0 && self.group(parent).is_none()) || self.group_within(parent, id) {
            return false;
        }
        let stays = (first..=last + 1).contains(&gap);
        if stays && parent_of(&self.groups, id) == parent {
            return false; // it is already there
        }
        let mut layers = self.layers.clone();
        if !stays {
            let block: Vec<Layer> = layers.drain(first..=last).collect();
            let at = if gap > last { gap - block.len() } else { gap };
            layers.splice(at..at, block);
        }
        let mut groups = self.groups.clone();
        if let Some(group) = groups.iter_mut().find(|g| g.id == id) {
            group.parent = parent;
        }
        self.restack(layers, groups)
    }

    /// Takes a new order for the layers and a new arrangement of the groups
    /// as one undoable step, unless it would leave some group's layers apart
    /// from each other.
    fn restack(&mut self, layers: Vec<Layer>, groups: Vec<LayerGroup>) -> bool {
        if !nesting_holds(&layers, &groups) {
            return false;
        }
        self.checkpoint_layers();
        let active = self.layers[self.active_layer].id;
        self.layers = layers;
        self.groups = groups;
        self.active_layer = self.layers.iter().position(|l| l.id == active).unwrap_or(0);
        self.prune_groups();
        self.mark_dirty(self.bounds());
        true
    }

    /// Folds or unfolds a group in the layer list. Not an undo step.
    pub fn set_group_collapsed(&mut self, id: u64, collapsed: bool) {
        if let Some(group) = self.groups.iter_mut().find(|g| g.id == id) {
            group.collapsed = collapsed;
        }
    }

    /// Drops groups that hold no layer any more, at any depth.
    pub(super) fn prune_groups(&mut self) {
        let mut used: Vec<u64> = Vec::new();
        for layer in &self.layers {
            for id in chain_of(&self.groups, layer.group) {
                if !used.contains(&id) {
                    used.push(id);
                }
            }
        }
        self.groups.retain(|g| used.contains(&g.id));
    }

    /// Makes loaded groups sound: fresh ids that cannot clash with anything
    /// made later, parents that exist, and layers next to each other. Groups
    /// that cannot be made sense of are dropped, leaving their layers loose.
    pub(super) fn adopt_groups(&mut self, groups: Vec<LayerGroup>) {
        let renamed: Vec<(u64, u64)> = groups.iter().map(|group| (group.id, self.next_version())).collect();
        let rename = |old: u64| renamed.iter().find(|(from, _)| *from == old).map_or(0, |(_, to)| *to);
        for layer in &mut self.layers {
            layer.group = rename(layer.group);
        }
        self.groups = groups
            .into_iter()
            .map(|mut group| {
                group.id = rename(group.id);
                group.parent = rename(group.parent);
                group
            })
            .collect();
        if !nesting_holds(&self.layers, &self.groups) {
            // Try again without the nesting, then without the groups at all.
            for group in &mut self.groups {
                group.parent = 0;
            }
            if !nesting_holds(&self.layers, &self.groups) {
                self.groups.clear();
                for layer in &mut self.layers {
                    layer.group = 0;
                }
            }
        }
        self.prune_groups();
    }
}
