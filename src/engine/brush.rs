//! Brush parameters, tip shapes and the built-in presets.

use std::sync::Arc;

use super::layer::BlendMode;

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
    /// Successively halved copies of the shape, so that small dabs are
    /// averaged from it rather than picked from scattered pixels.
    smaller: Vec<(usize, Vec<u8>)>,
}

impl TipImage {
    pub fn new(id: impl Into<String>, name: impl Into<String>, size: usize, alpha: Vec<u8>) -> Self {
        assert_eq!(alpha.len(), size * size, "tip image does not match its size");
        let mut smaller: Vec<(usize, Vec<u8>)> = Vec::new();
        let (mut side, mut level) = (size, alpha.clone());
        while side > 2 {
            let half = side.div_ceil(2);
            let mut next = Vec::with_capacity(half * half);
            for y in 0..half {
                for x in 0..half {
                    let at = |dx: usize, dy: usize| {
                        level[(y * 2 + dy).min(side - 1) * side + (x * 2 + dx).min(side - 1)] as u32
                    };
                    next.push(((at(0, 0) + at(1, 0) + at(0, 1) + at(1, 1) + 2) / 4) as u8);
                }
            }
            smaller.push((half, next.clone()));
            (side, level) = (half, next);
        }
        TipImage { id: id.into(), name: name.into(), size, alpha, smaller }
    }

    /// Bilinear coverage (0..=255) of one level at a position in 0..=1.
    fn sample_level(side: usize, data: &[u8], u: f32, v: f32) -> f32 {
        let n = side as f32;
        let (x, y) = (u * n - 0.5, v * n - 0.5);
        let (x0, y0) = (x.floor(), y.floor());
        let (tx, ty) = (x - x0, y - y0);
        let at = |ix: f32, iy: f32| -> f32 {
            if ix < 0.0 || iy < 0.0 || ix >= n || iy >= n {
                0.0
            } else {
                data[iy as usize * side + ix as usize] as f32
            }
        };
        let top = at(x0, y0) + (at(x0 + 1.0, y0) - at(x0, y0)) * tx;
        let bottom = at(x0, y0 + 1.0) + (at(x0 + 1.0, y0 + 1.0) - at(x0, y0 + 1.0)) * tx;
        top + (bottom - top) * ty
    }

    /// Coverage (0..=1) at a position given in 0..=1 across the tip; outside
    /// the tip there is none. `texels` is how many pixels of the tip one
    /// canvas pixel spans: above 1 the shape is being shrunk, and the answer
    /// blends the two nearest reduced copies.
    pub fn sample(&self, u: f32, v: f32, texels: f32) -> f32 {
        let detail = texels.max(1.0).log2().min(self.smaller.len() as f32);
        let level = |index: usize| -> f32 {
            match index.checked_sub(1) {
                None => Self::sample_level(self.size, &self.alpha, u, v),
                Some(i) => Self::sample_level(self.smaller[i].0, &self.smaller[i].1, u, v),
            }
        };
        let lower = detail.floor() as usize;
        let blend = detail - lower as f32;
        let value = if blend < 0.01 || lower >= self.smaller.len() {
            level(lower)
        } else {
            level(lower) + (level(lower + 1) - level(lower)) * blend
        };
        value / 255.0
    }
}

/// The surface a brush drags over: a tiling height map. High points take
/// paint first; hollows fill only under firm pressure.
pub struct PaperTexture {
    /// Stable identifier used when saving brushes ("fino", "lienzo"…).
    pub id: String,
    /// Name shown to the user.
    pub name: String,
    /// Side of the square, in pixels.
    pub size: usize,
    /// `size * size` heights, row-major: 0 = deepest hollow, 255 = highest point.
    pub data: Vec<u8>,
}

impl PaperTexture {
    /// Height (0..=1) at a position in texture pixels, repeating in both
    /// directions, with bilinear filtering.
    pub fn height(&self, x: f32, y: f32) -> f32 {
        let (x0, y0) = (x.floor(), y.floor());
        let (tx, ty) = (x - x0, y - y0);
        let n = self.size as i64;
        let at = |dx: i64, dy: i64| -> f32 {
            let (ix, iy) = ((x0 as i64 + dx).rem_euclid(n), (y0 as i64 + dy).rem_euclid(n));
            self.data[iy as usize * self.size + ix as usize] as f32
        };
        let top = at(0, 0) + (at(1, 0) - at(0, 0)) * tx;
        let bottom = at(0, 1) + (at(1, 1) - at(0, 1)) * tx;
        (top + (bottom - top) * ty) / 255.0
    }
}

impl PartialEq for PaperTexture {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl std::fmt::Debug for PaperTexture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PaperTexture({})", self.id)
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
    /// Paper whose grain shows through the stroke, if any.
    pub paper: Option<Arc<PaperTexture>>,
    /// How strongly the grain shows (0 = not at all, 1 = hollows stay bare).
    pub grain: f32,
    /// Size of the grain (1 = as drawn, 2 = twice as coarse).
    pub grain_scale: f32,
    /// How much of the colour already on the canvas the brush carries
    /// (0 = paints its own colour only, 1 = only moves what is there).
    pub mix: f32,
    /// How long picked-up colour stays on the brush (0 = replaced at once,
    /// 1 = dragged a long way).
    pub drag: f32,
    /// How the stroke combines with what is already on its layer. Brushes
    /// that mix colour always paint normally.
    pub blend: BlendMode,
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
            paper: None,
            grain: 0.7,
            grain_scale: 1.0,
            mix: 0.0,
            drag: 0.5,
            blend: BlendMode::Normal,
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
