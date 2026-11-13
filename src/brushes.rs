//! The brush library: the brushes the user can pick, the tip shapes they are
//! built from, and their storage between sessions.
//!
//! Brushes live in `pinceles.txt` in the configuration folder; imported tip
//! shapes are greyscale PNG files in its `puntas` folder.

use std::path::Path;
use std::sync::Arc;

use sharkpaint::engine::{
    builtin_papers, builtin_tips, read_abr, BrushPreset, BrushSettings, BrushTip, PaperTexture, TipImage,
};

use crate::settings::config_dir;

/// Raised whenever the set of default brushes grows.
const LIBRARY_VERSION: u32 = 4;

/// Side of the square an imported tip is stored at, at most.
const MAX_TIP_SIDE: u32 = 256;

#[derive(Clone, Debug, PartialEq)]
pub struct Brush {
    pub name: String,
    pub settings: BrushSettings,
}

pub struct BrushLibrary {
    pub brushes: Vec<Brush>,
    /// Built-in tips first, then the imported ones.
    pub tips: Vec<Arc<TipImage>>,
    /// Paper textures whose grain a brush can show.
    pub papers: Vec<Arc<PaperTexture>>,
}

impl BrushLibrary {
    /// The saved library, or the default one on first run.
    pub fn load() -> Self {
        let mut tips = builtin_tips();
        if let Some(dir) = config_dir().map(|d| d.join("puntas")) {
            let mut files: Vec<_> = std::fs::read_dir(dir)
                .into_iter()
                .flatten()
                .filter_map(|entry| entry.ok().map(|e| e.path()))
                .filter(|path| path.extension().is_some_and(|e| e.eq_ignore_ascii_case("png")))
                .collect();
            files.sort();
            tips.extend(files.iter().filter_map(|path| read_tip(path)));
        }
        let mut library = BrushLibrary { brushes: Vec::new(), tips, papers: builtin_papers() };
        let saved = config_dir().and_then(|dir| std::fs::read_to_string(dir.join("pinceles.txt")).ok());
        let version = saved
            .as_deref()
            .and_then(|text| text.lines().find_map(|line| line.strip_prefix("version=")))
            .and_then(|v| v.trim().parse::<u32>().ok())
            .unwrap_or(1);
        library.brushes = match saved.map(|text| library.brushes_from_text(&text)) {
            Some(mut brushes) if !brushes.is_empty() => {
                // A library saved by an earlier version gains the brushes added
                // since, without touching the ones already there.
                if version < LIBRARY_VERSION {
                    for brush in library.default_brushes() {
                        if !brushes.iter().any(|b| b.name == brush.name) {
                            brushes.push(brush);
                        }
                    }
                }
                brushes
            }
            _ => library.default_brushes(),
        };
        library
    }

    pub fn save(&self) {
        if let Some(dir) = config_dir() {
            let _ = std::fs::write(dir.join("pinceles.txt"), self.to_text());
        }
    }

    /// The tip with this identifier; unknown ones fall back to the round tip.
    pub fn tip(&self, id: &str) -> BrushTip {
        self.tips.iter().find(|t| t.id == id).map_or(BrushTip::Round, |t| BrushTip::Image(t.clone()))
    }

    /// The paper with this identifier, if there is one.
    pub fn paper(&self, id: &str) -> Option<Arc<PaperTexture>> {
        self.papers.iter().find(|p| p.id == id).cloned()
    }

    /// The brushes a new installation starts with.
    pub fn default_brushes(&self) -> Vec<Brush> {
        let mut brushes: Vec<Brush> = BrushPreset::ALL
            .iter()
            .map(|preset| Brush { name: preset.name().to_owned(), settings: preset.settings() })
            .collect();
        // The pencil drags over fine paper.
        brushes[0].settings.paper = self.paper("fino");
        brushes[0].settings.grain = 0.75;
        let shaped = |name: &str, tip: &str, settings: BrushSettings| Brush {
            name: name.to_owned(),
            settings: BrushSettings { tip: self.tip(tip), ..settings },
        };
        brushes.extend([
            // Crisp, strongly tapered line for clean line art.
            shaped("Pluma", "redonda", BrushSettings {
                hardness: 1.0,
                spacing: 0.03,
                smoothing: 0.6,
                pressure_size: true,
                min_size: 0.02,
                ..BrushSettings::round(12.0)
            }),
            shaped("Caligrafía", "redonda", BrushSettings {
                hardness: 1.0,
                spacing: 0.04,
                roundness: 0.2,
                angle: -40.0,
                pressure_size: true,
                min_size: 0.5,
                ..BrushSettings::round(28.0)
            }),
            shaped("Carboncillo", "carbon", BrushSettings {
                paper: self.paper("rugoso"),
                grain: 0.8,
                flow: 0.5,
                spacing: 0.18,
                angle_jitter: 1.0,
                pressure_flow: true,
                pressure_size: true,
                min_size: 0.6,
                ..BrushSettings::round(36.0)
            }),
            // Separate hairs that follow the stroke and gather as pressure drops.
            shaped("Cabello", "cerdas", BrushSettings {
                spacing: 0.02,
                smoothing: 0.5,
                follow_direction: true,
                pressure_size: true,
                min_size: 0.08,
                ..BrushSettings::round(44.0)
            }),
            shaped("Pincel seco", "cerdas", BrushSettings {
                paper: self.paper("lienzo"),
                grain: 0.5,
                flow: 0.6,
                spacing: 0.03,
                follow_direction: true,
                angle_jitter: 0.01,
                pressure_size: true,
                pressure_flow: true,
                min_size: 0.4,
                ..BrushSettings::round(60.0)
            }),
            // Soft stick dragged over rough paper: the paper does the texturing.
            shaped("Pastel", "redonda", BrushSettings {
                paper: self.paper("rugoso"),
                grain: 1.0,
                grain_scale: 1.5,
                flow: 0.7,
                hardness: 0.6,
                spacing: 0.06,
                pressure_flow: true,
                ..BrushSettings::round(46.0)
            }),
            // Wax crayon: a hard edge with the tooth of the paper inside it.
            shaped("Crayón", "redonda", BrushSettings {
                paper: self.paper("fino"),
                grain: 1.0,
                grain_scale: 2.0,
                hardness: 0.9,
                spacing: 0.05,
                pressure_size: true,
                min_size: 0.6,
                ..BrushSettings::round(22.0)
            }),
            shaped("Tiza", "tiza", BrushSettings {
                paper: self.paper("rugoso"),
                grain: 0.6,
                flow: 0.75,
                spacing: 0.1,
                follow_direction: true,
                angle: 90.0,
                pressure_flow: true,
                ..BrushSettings::round(34.0)
            }),
            // Thick paint that picks up what is under it and drags it along.
            shaped("Óleo", "cerdas", BrushSettings {
                mix: 0.5,
                drag: 0.75,
                flow: 0.7,
                spacing: 0.03,
                follow_direction: true,
                pressure_size: true,
                min_size: 0.5,
                ..BrushSettings::round(48.0)
            }),
            // Thin washes that melt into the colours next to them.
            shaped("Acuarela", "redonda", BrushSettings {
                mix: 0.35,
                drag: 0.4,
                flow: 0.25,
                hardness: 0.35,
                spacing: 0.06,
                paper: self.paper("acuarela"),
                grain: 0.35,
                pressure_flow: true,
                ..BrushSettings::round(60.0)
            }),
            // Adds no colour: softens the boundary between the ones already there.
            shaped("Difuminador", "redonda", BrushSettings {
                mix: 1.0,
                drag: 0.5,
                flow: 0.6,
                hardness: 0.2,
                spacing: 0.06,
                pressure_flow: true,
                ..BrushSettings::round(50.0)
            }),
            // Adds no colour either, but pulls paint a long way, like a fingertip.
            shaped("Dedo", "redonda", BrushSettings {
                mix: 1.0,
                drag: 0.85,
                flow: 0.75,
                hardness: 0.6,
                spacing: 0.04,
                pressure_size: true,
                min_size: 0.5,
                ..BrushSettings::round(34.0)
            }),
            shaped("Salpicadura", "salpicadura", BrushSettings {
                spacing: 0.5,
                scatter: 0.8,
                angle_jitter: 1.0,
                size_jitter: 0.6,
                smoothing: 0.0,
                ..BrushSettings::round(70.0)
            }),
            shaped("Hojas", "hoja", BrushSettings {
                spacing: 0.7,
                scatter: 0.9,
                angle_jitter: 1.0,
                size_jitter: 0.5,
                smoothing: 0.0,
                ..BrushSettings::round(54.0)
            }),
            shaped("Estrellas", "estrella", BrushSettings {
                spacing: 1.0,
                scatter: 1.4,
                angle_jitter: 1.0,
                size_jitter: 0.7,
                smoothing: 0.0,
                ..BrushSettings::round(40.0)
            }),
        ]);
        brushes
    }

    pub fn to_text(&self) -> String {
        let mut text = format!("version={LIBRARY_VERSION}\n");
        for brush in &self.brushes {
            let s = &brush.settings;
            text.push_str("[pincel]\n");
            let mut put = |key: &str, value: String| text.push_str(&format!("{key}={value}\n"));
            put("nombre", brush.name.replace('\n', " "));
            put("tamano", s.size.to_string());
            put("opacidad", s.opacity.to_string());
            put("flujo", s.flow.to_string());
            put("dureza", s.hardness.to_string());
            put("espaciado", s.spacing.to_string());
            put("estabilizador", s.smoothing.to_string());
            put("presion_tamano", (s.pressure_size as u8).to_string());
            put("presion_flujo", (s.pressure_flow as u8).to_string());
            put("tamano_minimo", s.min_size.to_string());
            put("punta", s.tip.id().to_owned());
            put("angulo", s.angle.to_string());
            put("variacion_angulo", s.angle_jitter.to_string());
            put("seguir_direccion", (s.follow_direction as u8).to_string());
            put("dispersion", s.scatter.to_string());
            put("variacion_tamano", s.size_jitter.to_string());
            put("redondez", s.roundness.to_string());
            put("papel", s.paper.as_ref().map_or("ninguno", |p| p.id.as_str()).to_owned());
            put("grano", s.grain.to_string());
            put("escala_grano", s.grain_scale.to_string());
            put("mezcla", s.mix.to_string());
            put("arrastre", s.drag.to_string());
        }
        text
    }

    /// Reads brushes, skipping values it does not understand.
    pub fn brushes_from_text(&self, text: &str) -> Vec<Brush> {
        let mut brushes: Vec<Brush> = Vec::new();
        for line in text.lines() {
            if line.trim() == "[pincel]" {
                brushes.push(Brush { name: "Pincel".into(), settings: BrushSettings::round(20.0) });
                continue;
            }
            let (Some(brush), Some((key, value))) = (brushes.last_mut(), line.split_once('=')) else { continue };
            let s = &mut brush.settings;
            let number = value.trim().parse::<f32>().ok().filter(|v| v.is_finite());
            let flag = value.trim() == "1";
            match (key.trim(), number) {
                ("nombre", _) => brush.name = value.trim().to_owned(),
                ("punta", _) => s.tip = self.tip(value.trim()),
                ("papel", _) => s.paper = self.paper(value.trim()),
                ("grano", Some(v)) => s.grain = v.clamp(0.0, 1.0),
                ("escala_grano", Some(v)) => s.grain_scale = v.clamp(0.1, 16.0),
                ("mezcla", Some(v)) => s.mix = v.clamp(0.0, 1.0),
                ("arrastre", Some(v)) => s.drag = v.clamp(0.0, 1.0),
                ("presion_tamano", _) => s.pressure_size = flag,
                ("presion_flujo", _) => s.pressure_flow = flag,
                ("seguir_direccion", _) => s.follow_direction = flag,
                ("tamano", Some(v)) => s.size = v.clamp(1.0, 2000.0),
                ("opacidad", Some(v)) => s.opacity = v.clamp(0.0, 1.0),
                ("flujo", Some(v)) => s.flow = v.clamp(0.0, 1.0),
                ("dureza", Some(v)) => s.hardness = v.clamp(0.0, 1.0),
                ("espaciado", Some(v)) => s.spacing = v.clamp(0.01, 5.0),
                ("estabilizador", Some(v)) => s.smoothing = v.clamp(0.0, 1.0),
                ("tamano_minimo", Some(v)) => s.min_size = v.clamp(0.0, 1.0),
                ("angulo", Some(v)) => s.angle = v.clamp(-360.0, 360.0),
                ("variacion_angulo", Some(v)) => s.angle_jitter = v.clamp(0.0, 1.0),
                ("dispersion", Some(v)) => s.scatter = v.clamp(0.0, 10.0),
                ("variacion_tamano", Some(v)) => s.size_jitter = v.clamp(0.0, 1.0),
                ("redondez", Some(v)) => s.roundness = v.clamp(0.02, 1.0),
                _ => {}
            }
        }
        brushes
    }

    /// Turns an image file into a tip shape and adds it to the library. Dark
    /// areas paint; if the image has transparency, its opaque areas do.
    pub fn import_tip(&mut self, path: &Path) -> Result<Arc<TipImage>, String> {
        let image = image::open(path).map_err(|e| e.to_string())?.into_rgba8();
        let (w, h) = (image.width() as usize, image.height() as usize);
        let has_alpha = image.pixels().any(|p| p[3] < 250);
        let coverage: Vec<u8> = image
            .pixels()
            .map(|p| {
                if has_alpha {
                    p[3]
                } else {
                    let luma = (p[0] as u32 * 299 + p[1] as u32 * 587 + p[2] as u32 * 114) / 1000;
                    255 - luma as u8
                }
            })
            .collect();
        let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "punta".into());
        self.store_tip(&stem, w, h, &coverage).map(|(tip, _)| tip)
    }

    /// Reads a Photoshop brush file and adds each of its brushes. Returns how
    /// many were added. The shape, size, hardness, spacing, angle, roundness
    /// and the pressure switches carry over; the rest of Photoshop's dynamics
    /// have no equivalent here.
    pub fn import_abr(&mut self, path: &Path) -> Result<usize, String> {
        let data = std::fs::read(path).map_err(|e| e.to_string())?;
        let file = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "abr".into());
        let mut added = 0;
        for (i, brush) in read_abr(&data)?.into_iter().enumerate() {
            let name = match &brush.name {
                Some(name) => name.clone(),
                None => format!("{file} {}", i + 1),
            };
            let size = brush.diameter.filter(|d| *d > 0.0);
            let (tip, size, hardness) = if brush.is_round() {
                (BrushTip::Round, size.unwrap_or(20.0), brush.hardness.unwrap_or(1.0))
            } else {
                // The number keeps tips with the same name in one file apart.
                let stem = format!("{file} {:03} {name}", i + 1);
                let Ok((tip, side)) = self.store_tip(&stem, brush.width, brush.height, &brush.coverage) else {
                    continue; // an empty picture
                };
                (BrushTip::Image(tip), size.unwrap_or(side as f32), 1.0)
            };
            let settings = BrushSettings {
                tip,
                hardness: hardness.clamp(0.0, 1.0),
                spacing: brush.spacing.unwrap_or(0.25).clamp(0.02, 5.0),
                angle: brush.angle.unwrap_or(0.0).clamp(-360.0, 360.0),
                roundness: brush.roundness.unwrap_or(1.0).clamp(0.02, 1.0),
                pressure_size: brush.pressure_size,
                min_size: brush.min_size.unwrap_or(0.0).clamp(0.0, 1.0),
                pressure_flow: brush.pressure_flow,
                smoothing: 0.2,
                ..BrushSettings::round(size.clamp(1.0, 500.0))
            };
            // Importing a file again replaces its brushes rather than doubling them.
            match self.brushes.iter_mut().find(|b| b.name == name && b.settings.tip == settings.tip) {
                Some(existing) => existing.settings = settings,
                None => self.brushes.push(Brush { name, settings }),
            }
            added += 1;
        }
        if added == 0 {
            return Err("el archivo no contiene pinceles que se puedan usar".into());
        }
        Ok(added)
    }

    /// Trims a coverage picture, centres it in a square, saves it with the
    /// other imported tips and adds it to the library. Also returns the side
    /// of the trimmed shape in pixels, before any reduction.
    fn store_tip(&mut self, name: &str, w: usize, h: usize, coverage: &[u8]) -> Result<(Arc<TipImage>, usize), String> {
        let used: Vec<(usize, usize)> =
            (0..h).flat_map(|y| (0..w).map(move |x| (x, y))).filter(|&(x, y)| coverage[y * w + x] > 8).collect();
        if used.is_empty() {
            return Err("la imagen está vacía: no hay nada que usar como punta".into());
        }
        let (x0, x1) = (used.iter().map(|p| p.0).min().unwrap(), used.iter().map(|p| p.0).max().unwrap());
        let (y0, y1) = (used.iter().map(|p| p.1).min().unwrap(), used.iter().map(|p| p.1).max().unwrap());
        let side = (x1 - x0 + 1).max(y1 - y0 + 1);
        let (pad_x, pad_y) = ((side - (x1 - x0 + 1)) / 2, (side - (y1 - y0 + 1)) / 2);
        let mut square = image::GrayImage::new(side as u32, side as u32);
        for y in y0..=y1 {
            for x in x0..=x1 {
                square.put_pixel((x - x0 + pad_x) as u32, (y - y0 + pad_y) as u32, image::Luma([coverage[y * w + x]]));
            }
        }
        if side as u32 > MAX_TIP_SIDE {
            square = image::imageops::resize(&square, MAX_TIP_SIDE, MAX_TIP_SIDE, image::imageops::FilterType::Triangle);
        }

        let stem: String = name
            .trim()
            .chars()
            .take(80)
            .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' || c == ' ' { c } else { '_' })
            .collect();
        // File names cannot end in a space on Windows.
        let stem = stem.trim().to_owned();
        if let Some(dir) = config_dir().map(|d| d.join("puntas")) {
            let _ = std::fs::create_dir_all(&dir);
            square.save(dir.join(format!("{stem}.png"))).map_err(|e| e.to_string())?;
        }
        let stored = square.width() as usize;
        let tip = Arc::new(TipImage::new(format!("archivo:{stem}"), stem, stored, square.into_raw()));
        // Importing the same name again replaces the earlier shape.
        self.tips.retain(|t| t.id != tip.id);
        self.tips.push(tip.clone());
        Ok((tip, side))
    }
}

/// Loads an imported tip saved earlier.
fn read_tip(path: &Path) -> Option<Arc<TipImage>> {
    let mut image = image::open(path).ok()?.into_luma8();
    if image.width() != image.height() || image.width() > MAX_TIP_SIDE {
        let side = image.width().max(image.height()).min(MAX_TIP_SIDE);
        image = image::imageops::resize(&image, side, side, image::imageops::FilterType::Triangle);
    }
    let stem = path.file_stem()?.to_string_lossy().into_owned();
    let side = image.width() as usize;
    Some(Arc::new(TipImage::new(format!("archivo:{stem}"), stem, side, image.into_raw())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brushes_round_trip_through_text() {
        let library = BrushLibrary { brushes: Vec::new(), tips: builtin_tips(), papers: builtin_papers() };
        let brushes = library.default_brushes();
        assert!(brushes.len() >= 10);
        let saved = BrushLibrary { brushes: brushes.clone(), tips: builtin_tips(), papers: builtin_papers() }.to_text();
        assert_eq!(library.brushes_from_text(&saved), brushes);

        // Unknown tips and junk values fall back instead of failing.
        let odd = "[pincel]\nnombre=Raro\npunta=archivo:no-existe\ntamano=abc\nflujo=7\nalgo=1\n";
        let read = library.brushes_from_text(odd);
        assert_eq!(read.len(), 1);
        assert_eq!((read[0].name.as_str(), read[0].settings.tip.id()), ("Raro", "redonda"));
        assert_eq!((read[0].settings.size, read[0].settings.flow), (20.0, 1.0));
        // Papers are saved by name; an unknown one means no paper.
        assert!(brushes.iter().any(|b| b.settings.paper.as_ref().is_some_and(|p| p.id == "rugoso")));
        let lost = library.brushes_from_text("[pincel]\npapel=desconocido\ngrano=0.4\n");
        assert!(lost[0].settings.paper.is_none() && lost[0].settings.grain == 0.4);
        assert!(library.brushes_from_text("sin secciones\n").is_empty());
    }

    #[cfg(not(windows))]
    #[test]
    fn imported_tips_are_trimmed_stored_and_found_again() {
        let dir = std::env::temp_dir().join(format!("sharkpaint-prueba-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // SAFETY: no other test in this binary reads the configuration folder.
        unsafe { std::env::set_var("XDG_CONFIG_HOME", &dir) };

        // A dark diagonal bar on white, off-centre, with a wide empty margin.
        let mut picture = image::RgbImage::from_pixel(300, 200, image::Rgb([255, 255, 255]));
        for i in 0..80 {
            for t in 0..12 {
                picture.put_pixel(150 + i, 60 + i / 2 + t, image::Rgb([0, 0, 0]));
            }
        }
        let file = dir.join("mi punta!.png");
        picture.save(&file).unwrap();

        let mut library = BrushLibrary { brushes: Vec::new(), tips: builtin_tips(), papers: builtin_papers() };
        let tip = library.import_tip(&file).unwrap();
        assert_eq!((tip.id.as_str(), tip.name.as_str()), ("archivo:mi punta_", "mi punta_"));
        assert_eq!(tip.size, 80); // trimmed to the bar and squared
        assert_eq!(tip.alpha.len(), 80 * 80);
        assert!(tip.sample(0.5, 0.5, 1.0) > 0.9, "the bar crosses the centre");
        assert!(tip.sample(0.9, 0.1, 1.0) < 0.1 && tip.sample(0.1, 0.9, 1.0) < 0.1, "the corners off the bar are empty");

        // A brush using it survives saving and a fresh start.
        library.brushes = vec![Brush {
            name: "Mío".into(),
            settings: BrushSettings { tip: BrushTip::Image(tip.clone()), ..BrushSettings::round(30.0) },
        }];
        library.save();
        let reloaded = BrushLibrary::load();
        assert!(reloaded.tips.iter().any(|t| t.id == tip.id && t.alpha == tip.alpha));
        assert_eq!(reloaded.brushes, library.brushes);

        // An empty image is refused with an explanation.
        let blank = dir.join("vacia.png");
        image::RgbImage::from_pixel(20, 20, image::Rgb([255, 255, 255])).save(&blank).unwrap();
        assert!(library.import_tip(&blank).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
