//! CPU raster painting engine: tiled layers, a dab-based brush, and undo history.

mod brush;
mod document;
mod export;
mod layer;
mod stroke;

pub use brush::{BrushPreset, BrushSettings};
pub use document::Document;
pub use export::write_png;
pub use layer::{IRect, Layer, Px, Tile, ONE, TILE};
pub use stroke::StrokePoint;
