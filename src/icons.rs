//! Interface icons.
//!
//! Stroke icons on a 24 × 24 grid, written as SVG path data in the same style as
//! SharkCut's icon set (1.7 px round strokes), so both applications look alike.
//! Each icon is rasterised from SVG at the size it is shown and tinted to the
//! theme's text colour.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use eframe::egui::{self, Color32, Image, ImageSource, Vec2};

/// Icon name and its path data.
pub const TRAZOS: &[(&str, &str)] = &[
    ("pincel", "M20.6 3.4a2 2 0 0 0-2.9 0l-8.2 8.3 2.8 2.8 8.3-8.2a2 2 0 0 0 0-2.9zM9.5 11.7c-2.3-.2-4 1.4-4.2 3.5-.2 1.6-1 2.6-2.3 3.3 3.6 1.2 8 .3 9.3-4"),
    ("borrador", "M9 20l-5.3-5.3a1.6 1.6 0 0 1 0-2.3l8.7-8.7a1.6 1.6 0 0 1 2.3 0l5.6 5.6a1.6 1.6 0 0 1 0 2.3L12 20zM7.5 8.6l8 8M12 20h9"),
    ("difuminar", "M12 3.5c-3.3 4.2-5.5 7.2-5.5 10.3a5.5 5.5 0 0 0 11 0c0-3.1-2.2-6.1-5.5-10.3zM9.5 14.5a2.6 2.6 0 0 0 2.2 2.6"),
    ("relleno", "M10.5 3.5l8.5 8.5-6.6 6.6a2 2 0 0 1-2.8 0l-5-5a2 2 0 0 1 0-2.8zM8 6l-2.5-2.5M4.2 12.5h14.3M20 15.5c-1 1.6-1.6 2.5-1.6 3.3a1.6 1.6 0 0 0 3.2 0c0-.8-.6-1.7-1.6-3.3z"),
    ("cuentagotas", "M13.5 6.5l4 4M11.5 8.5l4 4-7.5 7.5H5.5V17.5zM15.5 6.5l1.4-1.4a2.1 2.1 0 0 1 3 3L18.5 9.5M4 21l1.5-1"),
    ("mover", "M12 3v18M3 12h18M9 6l3-3 3 3M9 18l3 3 3-3M6 9l-3 3 3 3M18 9l3 3-3 3"),
    ("transformar", "M6.5 6.5h11v11h-11zM4 4h5v5H4zM15 4h5v5h-5zM4 15h5v5H4zM15 15h5v5h-5z"),
    ("forma", "M3 13h9v8H3zM21 8.5a5.5 5.5 0 1 1-11 0a5.5 5.5 0 1 1 11 0"),
    ("texto", "M5 7V5h14v2M12 5v14M9 19h6"),
    ("seleccion", "M4 4h3M10.5 4h3M17 4h3M4 20h3M10.5 20h3M17 20h3M4 7.5v2.5M4 14v2.5M20 7.5v2.5M20 14v2.5"),
    ("imagen", "M4 5h16v14H4zM4 16l4.5-4.5 3.5 3.5 3-3 5 5M9.5 9a1 1 0 1 1-2 0a1 1 0 1 1 2 0"),
    ("mas", "M12 5v14M5 12h14"),
    ("carpeta", "M3 6h6l2 2.5h10V19H3z"),
    ("duplicar", "M9 9h11v11H9zM15 9V4H4v11h5"),
    ("subir", "M12 19V5M6 11l6-6 6 6"),
    ("bajar", "M12 5v14M6 13l6 6 6-6"),
    ("fusionar", "M4 4h16M12 7v9M8 12l4 4 4-4M4 20h16"),
    ("quitar", "M4 7h16M10 11v6M14 11v6M6 7l1 13h10l1-13M9 7V4h6v3"),
    ("ojo", "M2 12c2-4 5.5-7 10-7s8 3 10 7c-2 4-5.5 7-10 7s-8-3-10-7zM15 12a3 3 0 1 1-6 0a3 3 0 1 1 6 0"),
    ("ojoCerrado", "M2 12c2-4 5.5-7 10-7s8 3 10 7c-2 4-5.5 7-10 7s-8-3-10-7zM15 12a3 3 0 1 1-6 0a3 3 0 1 1 6 0M4 4l16 16"),
    ("candado", "M7 11h10a2 2 0 0 1 2 2v6a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2v-6a2 2 0 0 1 2-2zM8 11V7a4 4 0 0 1 8 0v4"),
    ("candadoAbierto", "M7 11h10a2 2 0 0 1 2 2v6a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2v-6a2 2 0 0 1 2-2zM8 11V7a4 4 0 0 1 7.8-1.2"),
    ("ajustes", "M4 7h9M17 7h3M4 17h3M11 17h9M15 4.5v5M9 14.5v5"),
    ("deshacer", "M9 14L4 9l5-5M4 9h11a5 5 0 0 1 0 10h-3"),
    ("rehacer", "M15 14l5-5-5-5M20 9H9a5 5 0 0 0 0 10h3"),
];

/// The icon as a standalone SVG document with white strokes.
pub fn svg(path: &str) -> String {
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"24\" height=\"24\" viewBox=\"0 0 24 24\" \
         fill=\"none\" stroke=\"#ffffff\" stroke-width=\"1.7\" stroke-linecap=\"round\" \
         stroke-linejoin=\"round\"><path d=\"{path}\"/></svg>"
    )
}

fn sources() -> &'static HashMap<&'static str, (String, Arc<[u8]>)> {
    static SOURCES: OnceLock<HashMap<&'static str, (String, Arc<[u8]>)>> = OnceLock::new();
    SOURCES.get_or_init(|| {
        TRAZOS
            .iter()
            .map(|(name, path)| {
                let uri = format!("bytes://icono/{name}.svg");
                (*name, (uri, Arc::from(svg(path).into_bytes())))
            })
            .collect()
    })
}

/// An icon image `size` points square in the given colour.
pub fn icon(name: &str, size: f32, color: Color32) -> Image<'static> {
    let (uri, bytes) = sources().get(name).unwrap_or_else(|| panic!("icono desconocido: {name}"));
    Image::new(ImageSource::Bytes { uri: uri.clone().into(), bytes: egui::load::Bytes::Shared(bytes.clone()) })
        .fit_to_exact_size(Vec2::splat(size))
        .tint(color)
}
