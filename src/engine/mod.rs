//! CPU raster painting engine: tiled layers, a dab-based brush, and undo history.

mod abr;
mod brush;
mod document;
mod export;
mod layer;
mod papers;
mod project;
mod psd;
mod selection;
mod shapes;
mod stroke;
mod text;
mod tips;
mod tools;

pub use abr::{read_abr, AbrBrush};
pub use brush::{BrushPreset, BrushSettings, BrushTip, PaperTexture, TipImage};
pub use papers::builtin_papers;
pub use tips::builtin_tips;
pub use document::{Document, ProjectSnapshot, Reference};
pub use export::write_png;
pub use project::PROJECT_EXTENSION;
pub use psd::read_psd;
pub use layer::{BlendMode, IRect, Layer, LayerGroup, LayerProps, Px, Tile, ONE, TILE};
pub use selection::{SelectMode, Selection};
pub use shapes::Shape;
pub use stroke::StrokePoint;
pub use text::{render_text, TextFont};
pub use tools::{brush_preview, LayerTransform};
