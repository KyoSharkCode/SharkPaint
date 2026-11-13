//! CPU raster painting engine: tiled layers, a dab-based brush, and undo history.

mod abr;
mod brush;
mod document;
mod export;
mod layer;
mod papers;
mod project;
mod selection;
mod stroke;
mod tips;
mod tools;

pub use abr::{read_abr, AbrBrush};
pub use brush::{BrushPreset, BrushSettings, BrushTip, PaperTexture, TipImage};
pub use papers::builtin_papers;
pub use tips::builtin_tips;
pub use document::{Document, ProjectSnapshot, Reference};
pub use export::write_png;
pub use project::PROJECT_EXTENSION;
pub use layer::{BlendMode, IRect, Layer, LayerProps, Px, Tile, ONE, TILE};
pub use selection::Selection;
pub use stroke::StrokePoint;
pub use tools::{brush_preview, LayerTransform};
