//! Brush parameters, tip shapes and the built-in presets.

use std::sync::Arc;

/// A tip shape: a square greyscale stamp, where 255 deposits full paint.
pub struct TipImage {
    /// Stable identifier used when saving brushes ("carbon", "archivo:hoja"…).
    pub id: String,
    /// Name shown to the user.
    pub name: String,
    /// Side of the square, in pixels.
    pub size: usize,
    /// `size * size` coverage values, row-major.
    pub alpha: Vec<u8>,
}

impl TipImage {
    /// Coverage (0..=1) at a position given in 0..=1 across the tip, with
    /// bilinear filtering; outside the tip there is none.
    pub fn sample(&self, u: f32, v: f32) -> f32 {
        let n = self.size as f32;
        let (x, y) = (u * n - 0.5, v * n - 0.5);
        let (x0, y0) = (x.floor(), y.floor());
        let (tx, ty) = (x - x0, y - y0);
        let at = |ix: f32, iy: f32| -> f32 {
            if ix < 0.0 || iy < 0.0 || ix >= n || iy >= n {
                0.0
            } else {
                self.alpha[iy as usize * self.size + ix as usize] as f32
            }
        };
        let top = at(x0, y0) + (at(x0 + 1.0, y0) - at(x0, y0)) * tx;
        let bottom = at(x0, y0 + 1.0) + (at(x0 + 1.0, y0 + 1.0) - at(x0, y0 + 1.0)) * tx;
        (top + (bottom - top) * ty) / 255.0
    }
}

/// What the brush stamps along the stroke.
#[derive(Clone)]
pub enum BrushTip {
    /// A disc whose edge softness follows the brush hardness.
    Round,
    Image(Arc<TipImage>),
}

impl BrushTip {
    /// Identifier used when saving brushes.
    pub fn id(&self) -> &str {
        match self {
            BrushTip::Round => "redonda",
            BrushTip::Image(tip) => &tip.id,
        }
    }
}

impl PartialEq for BrushTip {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (BrushTip::Round, BrushTip::Round) => true,
            (BrushTip::Image(a), BrushTip::Image(b)) => Arc::ptr_eq(a, b) || a.id == b.id,
            _ => false,
        }
    }
}

impl std::fmt::Debug for BrushTip {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "BrushTip({})", self.id())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct BrushSettings {
    /// Tip diameter in canvas pixels at full pressure.
    pub size: f32,
    /// Maximum opacity a single stroke can reach (0..=1).
    pub opacity: f32,
    /// Paint deposited per dab (0..=1); low values build up gradually.
    pub flow: f32,
    /// Edge hardness of the round tip (0 = fully feathered, 1 = crisp).
    pub hardness: f32,
    /// Distance between dabs as a fraction of the tip diameter.
    pub spacing: f32,
    /// Stroke stabilisation (0 = raw input, 1 = heaviest smoothing).
    pub smoothing: f32,
    /// Pen pressure scales the tip size.
    pub pressure_size: bool,
    /// Pen pressure scales the flow.
    pub pressure_flow: bool,
    /// Tip size at zero pressure, as a fraction of `size`.
    pub min_size: f32,
    pub tip: BrushTip,
    /// Tip rotation in degrees, clockwise.
    pub angle: f32,
    /// Random rotation added to each dab (0 = none, 1 = any angle).
    pub angle_jitter: f32,
    /// The tip turns to follow the direction of the stroke.
    pub follow_direction: bool,
    /// Random offset of each dab, as a fraction of the tip diameter.
    pub scatter: f32,
    /// Random shrinking of each dab (0 = none, 1 = down to nothing).
    pub size_jitter: f32,
    /// Height of the tip relative to its width (1 = as drawn, lower = flatter).
    pub roundness: f32,
}

impl Default for BrushSettings {
    fn default() -> Self {
        BrushPreset::Tinta.settings()
    }
}

impl BrushSettings {
    /// A plain round brush; the starting point for every preset.
    pub fn round(size: f32) -> Self {
        BrushSettings {
            size,
            opacity: 1.0,
            flow: 1.0,
            hardness: 0.9,
            spacing: 0.08,
            smoothing: 0.3,
            pressure_size: false,
            pressure_flow: false,
            min_size: 1.0,
            tip: BrushTip::Round,
            angle: 0.0,
            angle_jitter: 0.0,
            follow_direction: false,
            scatter: 0.0,
            size_jitter: 0.0,
            roundness: 1.0,
        }
    }

    /// Tip diameter for a given pressure.
    pub fn diameter_at(&self, pressure: f32) -> f32 {
        if self.pressure_size {
            let p = pressure.clamp(0.0, 1.0);
            self.size * (self.min_size + (1.0 - self.min_size) * p)
        } else {
            self.size
        }
    }

    /// Per-dab flow for a given pressure.
    pub fn flow_at(&self, pressure: f32) -> f32 {
        if self.pressure_flow {
            self.flow * pressure.clamp(0.0, 1.0)
        } else {
            self.flow
        }
    }
}

/// The four basic round brushes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BrushPreset {
    Lapiz,
    Tinta,
    Marcador,
    Aerografo,
}

impl BrushPreset {
    pub const ALL: [BrushPreset; 4] =
        [BrushPreset::Lapiz, BrushPreset::Tinta, BrushPreset::Marcador, BrushPreset::Aerografo];

    pub fn name(&self) -> &'static str {
        match self {
            BrushPreset::Lapiz => "Lápiz",
            BrushPreset::Tinta => "Tinta",
            BrushPreset::Marcador => "Marcador",
            BrushPreset::Aerografo => "Aerógrafo",
        }
    }

    pub fn settings(&self) -> BrushSettings {
        match self {
            BrushPreset::Lapiz => BrushSettings {
                opacity: 0.9,
                flow: 0.45,
                hardness: 0.75,
                spacing: 0.08,
                smoothing: 0.25,
                pressure_size: true,
                pressure_flow: true,
                min_size: 0.5,
                ..BrushSettings::round(6.0)
            },
            BrushPreset::Tinta => BrushSettings {
                hardness: 0.95,
                spacing: 0.05,
                smoothing: 0.5,
                pressure_size: true,
                min_size: 0.08,
                ..BrushSettings::round(14.0)
            },
            BrushPreset::Marcador => BrushSettings {
                opacity: 0.6,
                hardness: 0.85,
                spacing: 0.06,
                ..BrushSettings::round(40.0)
            },
            BrushPreset::Aerografo => BrushSettings {
                flow: 0.06,
                hardness: 0.0,
                pressure_flow: true,
                ..BrushSettings::round(120.0)
            },
        }
    }
}
