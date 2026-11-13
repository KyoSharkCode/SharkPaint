//! Canvas navigation: maps between canvas pixels and screen points.

use eframe::egui::{Pos2, Rect, Vec2};

pub const MIN_ZOOM: f32 = 0.02;
pub const MAX_ZOOM: f32 = 64.0;

#[derive(Clone, Copy, Debug)]
pub struct View {
    /// Offset of the canvas centre from the viewport centre, in screen points.
    pub pan: Vec2,
    /// Screen points per canvas pixel.
    pub zoom: f32,
    /// Clockwise rotation in radians.
    pub rotation: f32,
    /// Mirror the canvas horizontally (display only).
    pub flip: bool,
}

impl Default for View {
    fn default() -> Self {
        View { pan: Vec2::ZERO, zoom: 1.0, rotation: 0.0, flip: false }
    }
}

impl View {
    /// Screen position of the canvas centre.
    pub fn center(&self, viewport: Rect) -> Pos2 {
        viewport.center() + self.pan
    }

    pub fn to_screen(&self, canvas: Pos2, viewport: Rect, doc_size: Vec2) -> Pos2 {
        let mut v = (canvas.to_vec2() - doc_size * 0.5) * self.zoom;
        if self.flip {
            v.x = -v.x;
        }
        let (s, c) = self.rotation.sin_cos();
        self.center(viewport) + Vec2::new(v.x * c - v.y * s, v.x * s + v.y * c)
    }

    pub fn to_canvas(&self, screen: Pos2, viewport: Rect, doc_size: Vec2) -> Pos2 {
        let d = screen - self.center(viewport);
        let (s, c) = self.rotation.sin_cos();
        let mut v = Vec2::new(d.x * c + d.y * s, -d.x * s + d.y * c);
        if self.flip {
            v.x = -v.x;
        }
        (v / self.zoom + doc_size * 0.5).to_pos2()
    }

    /// Scales the view, keeping the canvas point under `anchor` fixed on screen.
    pub fn zoom_about(&mut self, anchor: Pos2, factor: f32, viewport: Rect, doc_size: Vec2) {
        let under = self.to_canvas(anchor, viewport, doc_size);
        self.zoom = (self.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        self.pan += anchor - self.to_screen(under, viewport, doc_size);
    }

    /// Rotates the view, keeping the canvas point under `anchor` fixed on screen.
    pub fn rotate_about(&mut self, anchor: Pos2, delta: f32, viewport: Rect, doc_size: Vec2) {
        let under = self.to_canvas(anchor, viewport, doc_size);
        self.rotation = (self.rotation + delta).rem_euclid(std::f32::consts::TAU);
        self.pan += anchor - self.to_screen(under, viewport, doc_size);
    }

    /// Centres the whole canvas in the viewport with a small margin.
    pub fn fit(&mut self, viewport: Rect, doc_size: Vec2) {
        let avail = (viewport.size() - Vec2::splat(48.0)).max(Vec2::splat(1.0));
        let (s, c) = self.rotation.sin_cos();
        let bbox = Vec2::new(
            doc_size.x * c.abs() + doc_size.y * s.abs(),
            doc_size.x * s.abs() + doc_size.y * c.abs(),
        );
        self.zoom = (avail.x / bbox.x).min(avail.y / bbox.y).clamp(MIN_ZOOM, MAX_ZOOM);
        self.pan = Vec2::ZERO;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screen_and_canvas_round_trip() {
        let viewport = Rect::from_min_size(Pos2::new(60.0, 30.0), Vec2::new(900.0, 700.0));
        let doc = Vec2::new(3000.0, 2000.0);
        let mut view = View { pan: Vec2::new(37.0, -12.0), zoom: 0.37, rotation: 0.8, flip: true };
        for p in [Pos2::new(0.0, 0.0), Pos2::new(1234.5, 987.25), Pos2::new(3000.0, 2000.0)] {
            let back = view.to_canvas(view.to_screen(p, viewport, doc), viewport, doc);
            assert!((back - p).length() < 0.01, "{p:?} -> {back:?}");
        }
        // Zooming keeps the anchored canvas point under the cursor.
        let anchor = Pos2::new(400.0, 300.0);
        let before = view.to_canvas(anchor, viewport, doc);
        view.zoom_about(anchor, 2.5, viewport, doc);
        let after = view.to_canvas(anchor, viewport, doc);
        assert!((after - before).length() < 0.01);
        // So does rotating about it (two-finger rotation pivots between the fingers).
        view.rotate_about(anchor, 1.1, viewport, doc);
        let after = view.to_canvas(anchor, viewport, doc);
        assert!((after - before).length() < 0.02);
    }
}
