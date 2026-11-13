//! The selection: the part of the canvas that tools are allowed to change.

use super::layer::IRect;

pub struct Selection {
    pub width: usize,
    pub height: usize,
    /// Coverage per canvas pixel, row-major: 0 = outside, 255 = fully selected.
    pub mask: Vec<u8>,
    /// Smallest rectangle containing every selected pixel.
    pub bounds: IRect,
    /// The border between selected and unselected pixels, as straight
    /// segments in canvas coordinates, for drawing it.
    pub edges: Vec<[(f32, f32); 2]>,
}

/// How a new selection combines with the one already there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectMode {
    Replace,
    Add,
    Subtract,
    Intersect,
}

impl Selection {
    /// Selects the inside of a closed polygon (even-odd rule), with smooth
    /// edges. Returns `None` if nothing on the canvas ends up selected.
    pub fn from_polygon(width: usize, height: usize, points: &[(f32, f32)]) -> Option<Selection> {
        if points.len() < 3 {
            return None;
        }
        const SUBROWS: usize = 4;
        let mut mask = vec![0u8; width * height];
        let y_min = points.iter().map(|p| p.1).fold(f32::MAX, f32::min).floor().max(0.0) as usize;
        let y_max = (points.iter().map(|p| p.1).fold(f32::MIN, f32::max).ceil().max(0.0) as usize).min(height);
        let mut row = vec![0f32; width];
        let mut crossings: Vec<f32> = Vec::new();
        let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0usize, 0usize);
        for y in y_min..y_max {
            row.fill(0.0);
            for sub in 0..SUBROWS {
                let sy = y as f32 + (sub as f32 + 0.5) / SUBROWS as f32;
                crossings.clear();
                for i in 0..points.len() {
                    let (a, b) = (points[i], points[(i + 1) % points.len()]);
                    if (a.1 <= sy) != (b.1 <= sy) {
                        crossings.push(a.0 + (sy - a.1) / (b.1 - a.1) * (b.0 - a.0));
                    }
                }
                crossings.sort_by(|a, b| a.total_cmp(b));
                for span in crossings.chunks_exact(2) {
                    let (from, to) = (span[0].clamp(0.0, width as f32), span[1].clamp(0.0, width as f32));
                    if to <= from {
                        continue;
                    }
                    let (first, last) = (from.floor() as usize, (to.ceil() as usize).min(width));
                    for (x, cell) in row.iter_mut().enumerate().take(last).skip(first) {
                        // Share of this pixel's width that the span covers.
                        *cell += (to.min(x as f32 + 1.0) - from.max(x as f32)).max(0.0);
                    }
                }
            }
            for (x, cell) in row.iter().enumerate() {
                let value = (cell / SUBROWS as f32 * 255.0).round().min(255.0) as u8;
                if value != 0 {
                    mask[y * width + x] = value;
                    x0 = x0.min(x);
                    x1 = x1.max(x);
                    y0 = y0.min(y);
                    y1 = y1.max(y);
                }
            }
        }
        let _ = (x0, y0, x1, y1);
        Selection::from_mask(width, height, mask)
    }

    /// A selection from per-pixel coverage. `None` if nothing is selected.
    pub fn from_mask(width: usize, height: usize, mask: Vec<u8>) -> Option<Selection> {
        let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0usize, 0usize);
        for (y, row) in mask.chunks_exact(width).enumerate() {
            let Some(first) = row.iter().position(|&v| v != 0) else { continue };
            let last = row.iter().rposition(|&v| v != 0).unwrap_or(first);
            x0 = x0.min(first);
            x1 = x1.max(last);
            y0 = y0.min(y);
            y1 = y;
        }
        if x0 == usize::MAX {
            return None;
        }
        let bounds = IRect::new(x0 as i32, y0 as i32, x1 as i32 + 1, y1 as i32 + 1);
        let edges = trace_edges(width, &mask, bounds);
        Some(Selection { width, height, mask, bounds, edges })
    }

    /// This selection combined with new coverage. `None` if nothing is left.
    pub fn combined(current: Option<&Selection>, width: usize, height: usize, new: &[u8], mode: SelectMode) -> Option<Selection> {
        let mask: Vec<u8> = match (current, mode) {
            (_, SelectMode::Replace) | (None, SelectMode::Add) => new.to_vec(),
            (None, _) => return None,
            (Some(old), SelectMode::Add) => old.mask.iter().zip(new).map(|(&a, &b)| a.max(b)).collect(),
            (Some(old), SelectMode::Subtract) => {
                old.mask.iter().zip(new).map(|(&a, &b)| ((a as u32 * (255 - b as u32) + 127) / 255) as u8).collect()
            }
            (Some(old), SelectMode::Intersect) => old.mask.iter().zip(new).map(|(&a, &b)| a.min(b)).collect(),
        };
        Selection::from_mask(width, height, mask)
    }

    /// Everything that is not selected. `None` if the whole canvas was.
    pub fn inverted(&self) -> Option<Selection> {
        Selection::from_mask(self.width, self.height, self.mask.iter().map(|&v| 255 - v).collect())
    }

    /// Coverage (0..=255) at a canvas pixel; outside the canvas is unselected.
    pub fn coverage(&self, x: i32, y: i32) -> u8 {
        if x < 0 || y < 0 || x as usize >= self.width || y as usize >= self.height {
            return 0;
        }
        self.mask[y as usize * self.width + x as usize]
    }

    /// The selection mirrored within its own bounding box.
    pub(super) fn mirrored(&self, horizontal: bool) -> Selection {
        let b = self.bounds;
        let mut mask = vec![0u8; self.mask.len()];
        for y in b.y0..b.y1 {
            for x in b.x0..b.x1 {
                let (mx, my) = if horizontal { (b.x0 + b.x1 - 1 - x, y) } else { (x, b.y0 + b.y1 - 1 - y) };
                mask[my as usize * self.width + mx as usize] = self.mask[y as usize * self.width + x as usize];
            }
        }
        let edges = trace_edges(self.width, &mask, b);
        Selection { width: self.width, height: self.height, mask, bounds: b, edges }
    }
}

/// The border of the selected area inside `bounds`, as runs along pixel
/// edges. A pixel counts as selected from half coverage up.
fn trace_edges(width: usize, mask: &[u8], bounds: IRect) -> Vec<[(f32, f32); 2]> {
    let inside = |x: i32, y: i32| {
        x >= bounds.x0 && x < bounds.x1 && y >= bounds.y0 && y < bounds.y1 && mask[y as usize * width + x as usize] >= 128
    };
    let mut edges = Vec::new();
    // Horizontal runs: between each row and the one above it.
    for y in bounds.y0..=bounds.y1 {
        let mut start: Option<i32> = None;
        for x in bounds.x0..=bounds.x1 {
            let border = x < bounds.x1 && inside(x, y - 1) != inside(x, y);
            match (border, start) {
                (true, None) => start = Some(x),
                (false, Some(from)) => {
                    edges.push([(from as f32, y as f32), (x as f32, y as f32)]);
                    start = None;
                }
                _ => {}
            }
        }
    }
    // Vertical runs: between each column and the one to its left.
    for x in bounds.x0..=bounds.x1 {
        let mut start: Option<i32> = None;
        for y in bounds.y0..=bounds.y1 {
            let border = y < bounds.y1 && inside(x - 1, y) != inside(x, y);
            match (border, start) {
                (true, None) => start = Some(y),
                (false, Some(from)) => {
                    edges.push([(x as f32, from as f32), (x as f32, y as f32)]);
                    start = None;
                }
                _ => {}
            }
        }
    }
    edges
}
