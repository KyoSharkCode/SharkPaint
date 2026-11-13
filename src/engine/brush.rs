//! Brush parameters and the built-in presets.

#[derive(Clone, Debug, PartialEq)]
pub struct BrushSettings {
    /// Tip diameter in canvas pixels at full pressure.
    pub size: f32,
    /// Maximum opacity a single stroke can reach (0..=1).
    pub opacity: f32,
    /// Paint deposited per dab (0..=1); low values build up gradually.
    pub flow: f32,
    /// Edge hardness (0 = fully feathered, 1 = crisp).
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
}

impl Default for BrushSettings {
    fn default() -> Self {
        BrushPreset::Tinta.settings()
    }
}

impl BrushSettings {
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
                size: 6.0,
                opacity: 0.9,
                flow: 0.45,
                hardness: 0.75,
                spacing: 0.08,
                smoothing: 0.25,
                pressure_size: true,
                pressure_flow: true,
                min_size: 0.5,
            },
            BrushPreset::Tinta => BrushSettings {
                size: 14.0,
                opacity: 1.0,
                flow: 1.0,
                hardness: 0.95,
                spacing: 0.05,
                smoothing: 0.5,
                pressure_size: true,
                pressure_flow: false,
                min_size: 0.08,
            },
            BrushPreset::Marcador => BrushSettings {
                size: 40.0,
                opacity: 0.6,
                flow: 1.0,
                hardness: 0.85,
                spacing: 0.06,
                smoothing: 0.3,
                pressure_size: false,
                pressure_flow: false,
                min_size: 1.0,
            },
            BrushPreset::Aerografo => BrushSettings {
                size: 120.0,
                opacity: 1.0,
                flow: 0.06,
                hardness: 0.0,
                spacing: 0.08,
                smoothing: 0.3,
                pressure_size: false,
                pressure_flow: true,
                min_size: 1.0,
            },
        }
    }
}
