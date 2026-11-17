//! Turning text into pixels, to place on the canvas as a layer.

use ab_glyph::{point, Font, FontVec, Glyph, PxScale, ScaleFont};

/// Largest side of a rendered block of text, in pixels.
const MAX_SIDE: usize = 8192;

/// Name of the typeface that comes with the program.
pub const BUILTIN_FONT: &str = "Ubuntu Light";

/// The names of the typefaces in a font file, as their makers call them
/// ("Arial Bold"), one per face in file order. A face that does not say gets
/// an empty name; data that is not a font gives an empty list.
pub fn font_names(data: &[u8]) -> Vec<String> {
    let faces = ttf_parser::fonts_in_collection(data).unwrap_or(1);
    (0..faces)
        .filter_map(|index| ttf_parser::Face::parse(data, index).ok())
        .map(|face| {
            // The full name if there is one, in English for choice; else the
            // family and style put together.
            let find = |id: u16| {
                let named = || face.names().into_iter().filter(move |name| name.name_id == id && name.is_unicode());
                named()
                    .find(|name| name.language() == ttf_parser::Language::English_UnitedStates)
                    .or_else(|| named().next())
                    .and_then(|name| name.to_string())
                    .map(|name| name.trim().to_owned())
                    .filter(|name| !name.is_empty())
            };
            find(ttf_parser::name_id::FULL_NAME)
                .or_else(|| {
                    let family = find(ttf_parser::name_id::FAMILY)?;
                    Some(match find(ttf_parser::name_id::SUBFAMILY) {
                        Some(style) if style != "Regular" => format!("{family} {style}"),
                        _ => family,
                    })
                })
                .unwrap_or_default()
        })
        .collect()
}

/// A typeface text can be drawn in.
pub struct TextFont {
    pub name: String,
    font: FontVec,
}

impl TextFont {
    /// A typeface from the contents of a font file (TrueType or OpenType);
    /// `index` picks the face in a file that holds several. `None` if the
    /// data is not a usable font.
    pub fn from_bytes(name: impl Into<String>, data: Vec<u8>, index: u32) -> Option<TextFont> {
        let font = FontVec::try_from_vec_and_index(data, index).ok()?;
        Some(TextFont { name: name.into(), font })
    }

    /// The font file of the typeface that comes with the program.
    pub fn builtin_data() -> &'static [u8] {
        epaint_default_fonts::UBUNTU_LIGHT
    }

    /// The typeface that comes with the program.
    pub fn builtin() -> TextFont {
        TextFont::from_bytes(BUILTIN_FONT, epaint_default_fonts::UBUNTU_LIGHT.to_vec(), 0).expect("the bundled font is valid")
    }
}

/// Draws `text` (lines separated by `\n`) at `size` pixels tall in one colour.
/// Returns the width, the height and 8-bit RGBA that is not premultiplied,
/// trimmed to the text with a pixel of margin; `None` if there is nothing to
/// draw or it would be unreasonably large.
pub fn render_text(font: &TextFont, text: &str, size: f32, color: [u8; 3]) -> Option<(usize, usize, Vec<u8>)> {
    let scale = PxScale::from(size.clamp(4.0, 2000.0));
    let scaled = font.font.as_scaled(scale);
    let line_height = scaled.height() + scaled.line_gap();
    // Place every glyph, kerning each against the one before it on its line.
    let mut glyphs: Vec<Glyph> = Vec::new();
    for (row, line) in text.lines().enumerate() {
        let baseline = scaled.ascent() + row as f32 * line_height;
        let mut x = 0.0;
        let mut previous = None;
        for character in line.chars() {
            let id = font.font.glyph_id(character);
            if let Some(previous) = previous {
                x += scaled.kern(previous, id);
            }
            glyphs.push(id.with_scale_and_position(scale, point(x, baseline)));
            x += scaled.h_advance(id);
            previous = Some(id);
        }
    }
    let outlines: Vec<_> = glyphs.into_iter().filter_map(|glyph| font.font.outline_glyph(glyph)).collect();
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for outline in &outlines {
        let bounds = outline.px_bounds();
        x0 = x0.min(bounds.min.x);
        y0 = y0.min(bounds.min.y);
        x1 = x1.max(bounds.max.x);
        y1 = y1.max(bounds.max.y);
    }
    if outlines.is_empty() || x1 <= x0 || y1 <= y0 {
        return None;
    }
    let (left, top) = (x0.floor() as i32 - 1, y0.floor() as i32 - 1);
    let (width, height) = ((x1.ceil() as i32 - left + 1) as usize, (y1.ceil() as i32 - top + 1) as usize);
    if width > MAX_SIDE || height > MAX_SIDE {
        return None;
    }
    let mut coverage = vec![0f32; width * height];
    for outline in &outlines {
        let bounds = outline.px_bounds();
        let (ox, oy) = (bounds.min.x as i32 - left, bounds.min.y as i32 - top);
        outline.draw(|x, y, amount| {
            let (px, py) = (ox + x as i32, oy + y as i32);
            if px >= 0 && py >= 0 && (px as usize) < width && (py as usize) < height {
                // Glyphs that touch build up rather than cut into each other.
                let cell = &mut coverage[py as usize * width + px as usize];
                *cell = (*cell + amount).min(1.0);
            }
        });
    }
    let mut rgba = Vec::with_capacity(width * height * 4);
    for amount in coverage {
        rgba.extend([color[0], color[1], color[2], (amount * 255.0).round() as u8]);
    }
    Some((width, height, rgba))
}
