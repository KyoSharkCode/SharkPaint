//! CPU raster painting engine: tiled layers, a dab-based brush, and undo history.

mod brush;
mod document;
mod export;
mod layer;
mod project;
mod selection;
mod stroke;
mod tools;

pub use brush::{BrushPreset, BrushSettings};
pub use document::Document;
pub use export::write_png;
pub use project::PROJECT_EXTENSION;
pub use layer::{BlendMode, IRect, Layer, LayerProps, Px, Tile, ONE, TILE};
pub use selection::Selection;
pub use stroke::StrokePoint;
pub use tools::LayerTransform;
