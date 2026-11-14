//! Reader for Photoshop brush files (`.abr`).
//!
//! A brush is either sampled (it stamps a greyscale picture) or computed (a
//! plain round tip with a hardness). Both are read, with their name, size,
//! spacing, angle and roundness, and whether pen pressure drives the size or
//! the flow. The rest of Photoshop's dynamics have no equivalent here.
//!
//! Two layouts exist. Versions 1 and 2 are a flat list of brushes. Versions 6
//! to 10 keep the pictures in a `samp` block and the settings in a `desc`
//! block, linked by an identifier.

/// One brush tip read from a file.
#[derive(Clone, Debug, PartialEq)]
pub struct AbrBrush {
    pub name: Option<String>,
    /// Size of the picture; zero for a computed (round) brush.
    pub width: usize,
    pub height: usize,
    /// Coverage, row by row: 0 leaves the canvas alone, 255 paints fully.
    /// Empty for a computed (round) brush.
    pub coverage: Vec<u8>,
    /// Edge of a computed brush: 1 = crisp, 0 = fully soft.
    pub hardness: Option<f32>,
    /// Pen pressure changes the size, down to `min_size` (a share of it).
    pub pressure_size: bool,
    pub min_size: Option<f32>,
    /// Pen pressure changes how much paint is laid down.
    pub pressure_flow: bool,
    /// Distance between dabs as a share of the size (0.25 = a quarter).
    pub spacing: Option<f32>,
    /// Size in pixels the brush was saved at.
    pub diameter: Option<f32>,
    /// Rotation in degrees.
    pub angle: Option<f32>,
    /// 1 = as drawn, lower = squashed.
    pub roundness: Option<f32>,
}

impl AbrBrush {
    /// A computed brush: a round tip with no picture.
    pub fn is_round(&self) -> bool {
        self.coverage.is_empty()
    }

    fn blank() -> Self {
        AbrBrush {
            name: None,
            width: 0,
            height: 0,
            coverage: Vec::new(),
            hardness: None,
            pressure_size: false,
            min_size: None,
            pressure_flow: false,
            spacing: None,
            diameter: None,
            angle: None,
            roundness: None,
        }
    }
}

/// Largest tip side accepted; anything bigger is a damaged file.
const MAX_SIDE: i64 = 20_000;

struct Reader<'a> {
    data: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self.at.checked_add(n).filter(|&end| end <= self.data.len()).ok_or("el archivo está incompleto")?;
        let bytes = &self.data[self.at..end];
        self.at = end;
        Ok(bytes)
    }
    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, String> {
        Ok(u16::from_be_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn i32(&mut self) -> Result<i32, String> {
        Ok(self.u32()? as i32)
    }
    fn seek(&mut self, at: usize) {
        self.at = at.min(self.data.len());
    }
}

/// Reads every brush in a `.abr` file, in the order the file lists them.
pub fn read_abr(data: &[u8]) -> Result<Vec<AbrBrush>, String> {
    let mut r = Reader { data, at: 0 };
    let version = r.u16().map_err(|_| "no es un archivo de pinceles de Photoshop")?;
    let count = r.u16().map_err(|_| "no es un archivo de pinceles de Photoshop")?;
    let brushes = match version {
        1 | 2 => read_flat(&mut r, version, count)?,
        6..=10 if count == 1 || count == 2 => read_blocks(&mut r, count)?,
        6..=10 => return Err(format!("versión {version}.{count} del formato no reconocida")),
        _ => return Err("no es un archivo de pinceles de Photoshop".into()),
    };
    if brushes.is_empty() {
        return Err("el archivo no contiene pinceles".into());
    }
    Ok(brushes)
}

/// Versions 1 and 2: one brush after another.
fn read_flat(r: &mut Reader, version: u16, count: u16) -> Result<Vec<AbrBrush>, String> {
    let mut brushes = Vec::new();
    for _ in 0..count {
        let Ok(kind) = r.u16() else { break };
        let Ok(size) = r.u32() else { break };
        let next = r.at.saturating_add(size as usize);
        // Read within the entry only, so a short one cannot run into the next.
        let mut entry = Reader { data: &r.data[..next.min(r.data.len())], at: r.at };
        let brush = match kind {
            1 => read_flat_round(&mut entry),
            2 => read_flat_brush(&mut entry, version),
            _ => Err(String::new()),
        };
        brushes.extend(brush.ok());
        r.seek(next);
    }
    Ok(brushes)
}

/// A computed brush in the old layout: only numbers.
fn read_flat_round(r: &mut Reader) -> Result<AbrBrush, String> {
    r.u32()?; // unused
    let spacing = r.u16()?;
    let diameter = r.u16()?;
    let roundness = r.u16()?;
    let angle = r.u16()? as i16;
    let hardness = r.u16()?;
    Ok(AbrBrush {
        spacing: Some(spacing as f32 / 100.0).filter(|s| *s > 0.0),
        diameter: Some(diameter as f32).filter(|d| *d > 0.0),
        roundness: Some(roundness as f32 / 100.0).filter(|v| *v > 0.0),
        angle: Some(angle as f32),
        hardness: Some(hardness as f32 / 100.0),
        ..AbrBrush::blank()
    })
}

fn read_flat_brush(r: &mut Reader, version: u16) -> Result<AbrBrush, String> {
    r.u32()?; // unused
    let spacing = r.u16()?;
    let name = if version == 2 {
        let chars = r.u32()? as usize;
        Some(utf16(r.take(chars.checked_mul(2).ok_or("nombre dañado")?)?)).filter(|n| !n.is_empty())
    } else {
        None
    };
    r.u8()?; // antialiasing
    r.take(8)?; // bounds, short form
    let (width, height, depth, compressed) = picture_header(r)?;
    if depth != 8 {
        return Err("profundidad no admitida".into());
    }
    let coverage = picture(r, width, height, depth, compressed)?;
    Ok(AbrBrush {
        name,
        width,
        height,
        coverage,
        spacing: Some(spacing as f32 / 100.0).filter(|s| *s > 0.0),
        ..AbrBrush::blank()
    })
}

/// Versions 6 to 10: `8BIM` blocks, of which `samp` holds the pictures and
/// `desc` the settings.
fn read_blocks(r: &mut Reader, subversion: u16) -> Result<Vec<AbrBrush>, String> {
    let (mut samples, mut settings) = (None, None);
    while let Ok(tag) = r.take(4) {
        if tag != b"8BIM" {
            break;
        }
        let name = r.take(4)?;
        let size = r.u32()? as usize;
        let start = r.at;
        let end = start.saturating_add(size).min(r.data.len());
        match name {
            b"samp" => samples = Some((start, end)),
            b"desc" => settings = Some(&r.data[start..end]),
            _ => {}
        }
        r.seek(end);
    }
    // Pictures first, each under its identifier.
    let mut pictures: Vec<(&[u8], AbrBrush)> = Vec::new();
    if let Some((start, end)) = samples {
        r.seek(start);
        while r.at + 4 <= end {
            let size = r.u32()? as usize;
            let begin = r.at;
            // Each entry is padded to a multiple of four bytes.
            let next = begin.saturating_add(size.saturating_add(3) & !3);
            // The entry opens with its identifier, then bytes of no use here.
            let skip = if subversion == 1 { 47 } else { 301 };
            let key = &r.data[begin.min(r.data.len())..(begin + skip).min(r.data.len())];
            r.seek(begin + skip);
            let brush = picture_header(r).and_then(|(width, height, depth, compressed)| {
                let coverage = picture(r, width, height, depth, compressed)?;
                Ok(AbrBrush { width, height, coverage, ..AbrBrush::blank() })
            });
            pictures.extend(brush.ok().map(|brush| (key, brush)));
            if next <= begin {
                break;
            }
            r.seek(next);
        }
    }

    // The settings list the brushes the user sees, in order; several may share
    // a picture, and round ones have none.
    if let Some(listed) = settings.and_then(listed_brushes) {
        let mut used = vec![false; pictures.len()];
        let mut brushes = Vec::new();
        for (key, mut brush) in listed {
            match key {
                None => brushes.push(brush),
                Some(key) => {
                    let Some(i) = pictures.iter().position(|(id, _)| contains(id, key.as_bytes())) else { continue };
                    used[i] = true;
                    let picture = &pictures[i].1;
                    (brush.width, brush.height, brush.coverage) = (picture.width, picture.height, picture.coverage.clone());
                    brushes.push(brush);
                }
            }
        }
        // Pictures no brush refers to are still worth having.
        brushes.extend(pictures.into_iter().zip(used).filter(|(_, used)| !used).map(|((_, brush), _)| brush));
        return Ok(brushes);
    }

    // Settings that cannot be read in full: look for the values by name.
    let presets = settings.map(presets).unwrap_or_default();
    let mut brushes = Vec::new();
    for (index, (key, mut brush)) in pictures.into_iter().enumerate() {
        let preset = presets
            .iter()
            .find(|p| !p.key.is_empty() && contains(key, p.key.as_bytes()))
            .or_else(|| presets.get(index).filter(|p| p.key.is_empty()));
        if let Some(preset) = preset {
            brush.name = preset.name.clone();
            brush.spacing = preset.spacing;
            brush.diameter = preset.diameter;
            brush.angle = preset.angle;
            brush.roundness = preset.roundness;
        }
        brushes.push(brush);
    }
    Ok(brushes)
}

/// A value in the settings block, which is a tree of named values.
enum Value {
    Object(Vec<(String, Value)>),
    List(Vec<Value>),
    Number(f64),
    Text(String),
    Flag(bool),
    Other,
}

impl Value {
    fn get(&self, name: &str) -> Option<&Value> {
        match self {
            Value::Object(items) => items.iter().find(|(key, _)| key == name).map(|(_, value)| value),
            _ => None,
        }
    }
    fn number(&self, name: &str) -> Option<f32> {
        match self.get(name)? {
            Value::Number(v) if v.is_finite() => Some(*v as f32),
            _ => None,
        }
    }
    fn text(&self, name: &str) -> Option<String> {
        match self.get(name)? {
            Value::Text(text) if !text.is_empty() => Some(text.clone()),
            _ => None,
        }
    }
    fn flag(&self, name: &str) -> bool {
        matches!(self.get(name), Some(Value::Flag(true)))
    }
    /// Whether a dynamics entry is driven by pen pressure.
    fn by_pressure(&self, name: &str) -> bool {
        self.get(name).and_then(|entry| entry.number("bVTy")) == Some(2.0)
    }
}

/// A name in the tree: four letters, or a longer one with its length in front.
fn tree_name(r: &mut Reader) -> Result<String, String> {
    let length = r.u32()? as usize;
    let bytes = r.take(if length == 0 { 4 } else { length })?;
    Ok(String::from_utf8_lossy(bytes).into_owned())
}

fn tree_text(r: &mut Reader) -> Result<String, String> {
    let chars = r.u32()? as usize;
    Ok(utf16(r.take(chars.checked_mul(2).ok_or("texto dañado")?)?))
}

fn tree_object(r: &mut Reader, depth: u32) -> Result<Value, String> {
    tree_text(r)?; // display name
    tree_name(r)?; // class
    let count = r.u32()?;
    let mut items = Vec::new();
    for _ in 0..count {
        let name = tree_name(r)?;
        items.push((name, tree_value(r, depth)?));
    }
    Ok(Value::Object(items))
}

fn tree_value(r: &mut Reader, depth: u32) -> Result<Value, String> {
    if depth > 40 {
        return Err("ajustes demasiado anidados".into());
    }
    Ok(match r.take(4)? {
        b"Objc" | b"GlbO" => tree_object(r, depth + 1)?,
        b"VlLs" => {
            let count = r.u32()?;
            let mut items = Vec::new();
            for _ in 0..count {
                items.push(tree_value(r, depth + 1)?);
            }
            Value::List(items)
        }
        b"doub" => Value::Number(f64::from_be_bytes(r.take(8)?.try_into().unwrap())),
        b"UntF" => {
            r.take(4)?; // unit
            Value::Number(f64::from_be_bytes(r.take(8)?.try_into().unwrap()))
        }
        b"TEXT" => Value::Text(tree_text(r)?),
        b"long" => Value::Number(r.i32()? as f64),
        b"bool" => Value::Flag(r.u8()? != 0),
        b"enum" => {
            tree_name(r)?;
            tree_name(r)?;
            Value::Other
        }
        b"comp" => {
            r.take(8)?;
            Value::Other
        }
        b"tdta" | b"alis" => {
            let length = r.u32()? as usize;
            r.take(length)?;
            Value::Other
        }
        b"type" | b"GlbC" => {
            tree_text(r)?;
            tree_name(r)?;
            Value::Other
        }
        _ => return Err("ajuste de tipo desconocido".into()),
    })
}

/// The brushes the settings block lists, each with the identifier of its
/// picture if it has one. `None` if the block cannot be read in full.
fn listed_brushes(block: &[u8]) -> Option<Vec<(Option<String>, AbrBrush)>> {
    let mut r = Reader { data: block, at: 0 };
    r.u32().ok()?; // version of the tree
    let root = tree_object(&mut r, 0).ok()?;
    let Value::List(presets) = root.get("Brsh")? else { return None };
    let percent = |value: Option<f32>| value.map(|v| v / 100.0);
    let mut brushes = Vec::new();
    for preset in presets {
        let Some(tip) = preset.get("Brsh") else { continue };
        let key = tip.text("sampledData");
        let pressure_size = preset.flag("useTipDynamics") && preset.by_pressure("szVr");
        let paint = preset.flag("usePaintDynamics");
        brushes.push((
            key,
            AbrBrush {
                name: preset.text("Nm  ").or_else(|| tip.text("Nm  ")),
                diameter: tip.number("Dmtr").filter(|v| *v > 0.0),
                spacing: percent(tip.number("Spcn")).filter(|v| *v > 0.0),
                angle: tip.number("Angl"),
                roundness: percent(tip.number("Rndn")).filter(|v| *v > 0.0),
                hardness: percent(tip.number("Hrdn")),
                pressure_size,
                min_size: percent(preset.number("minimumDiameter")).filter(|_| pressure_size),
                pressure_flow: paint && (preset.by_pressure("prVr") || preset.by_pressure("opVr")),
                ..AbrBrush::blank()
            },
        ));
    }
    Some(brushes).filter(|b| !b.is_empty())
}

/// Bounds, depth and compression flag in front of a picture.
fn picture_header(r: &mut Reader) -> Result<(usize, usize, u16, bool), String> {
    let (top, left, bottom, right) = (r.i32()? as i64, r.i32()? as i64, r.i32()? as i64, r.i32()? as i64);
    let depth = r.u16()?;
    let compressed = match r.u8()? {
        0 => false,
        1 => true,
        _ => return Err("compresión desconocida".into()),
    };
    let (width, height) = (right - left, bottom - top);
    if width < 1 || height < 1 || width > MAX_SIDE || height > MAX_SIDE || (depth != 8 && depth != 16) {
        return Err("medidas de la punta fuera de rango".into());
    }
    Ok((width as usize, height as usize, depth, compressed))
}

/// The picture itself, as 8-bit coverage.
fn picture(r: &mut Reader, width: usize, height: usize, depth: u16, compressed: bool) -> Result<Vec<u8>, String> {
    let bytes = (depth / 8) as usize;
    let row = width * bytes;
    let raw: Vec<u8> = if compressed {
        // Each row is packed on its own; their packed lengths come first.
        let lengths: Vec<usize> = (0..height).map(|_| r.u16().map(|n| n as usize)).collect::<Result<_, _>>()?;
        let mut out = Vec::with_capacity(row * height);
        for length in lengths {
            let packed = r.take(length)?;
            let start = out.len();
            unpack(packed, &mut out, row);
            out.resize(start + row, 0);
        }
        out
    } else {
        r.take(row * height)?.to_vec()
    };
    if bytes == 1 {
        return Ok(raw);
    }
    // Sixteen-bit pictures: files disagree on which byte comes first, so take
    // the reading that gives the smoother picture; the other one is noise.
    let read = |first: usize| -> Vec<u8> { raw.chunks_exact(2).map(|pair| pair[first]).collect() };
    let roughness = |picture: &[u8]| -> u64 {
        picture.chunks(width).flat_map(|line| line.windows(2)).map(|w| w[0].abs_diff(w[1]) as u64).sum()
    };
    let (high_first, low_first) = (read(0), read(1));
    Ok(if roughness(&high_first) <= roughness(&low_first) { high_first } else { low_first })
}

/// PackBits: a count byte, then either that many literal bytes or one byte to
/// repeat. Stops at `limit` bytes of output.
fn unpack(packed: &[u8], out: &mut Vec<u8>, limit: usize) {
    let end = out.len() + limit;
    let mut i = 0;
    while i < packed.len() && out.len() < end {
        let n = packed[i] as i8;
        i += 1;
        if n >= 0 {
            let count = (n as usize + 1).min(packed.len() - i).min(end - out.len());
            out.extend_from_slice(&packed[i..i + count]);
            i += n as usize + 1;
        } else if n != -128 {
            let count = (1 - n as isize) as usize;
            if let Some(&value) = packed.get(i) {
                out.extend(std::iter::repeat_n(value, count.min(end - out.len())));
            }
            i += 1;
        }
    }
}

#[derive(Default)]
struct Preset {
    key: String,
    name: Option<String>,
    spacing: Option<f32>,
    diameter: Option<f32>,
    angle: Option<f32>,
    roundness: Option<f32>,
}

/// Pulls the few settings of use out of the `desc` block. The block is a tree
/// of named values; rather than walk all of it, this looks for the values by
/// name. Each brush ends with the identifier of its picture, so the values
/// found since the previous identifier belong to it.
fn presets(block: &[u8]) -> Vec<Preset> {
    let mut found = Vec::new();
    let mut from = 0;
    const KEY: &[u8] = b"sampledDataTEXT";
    while let Some(offset) = find(&block[from..], KEY) {
        let at = from + offset;
        let own = &block[from..at];
        let number = |label: &[u8]| -> Option<f32> {
            let i = rfind(own, label)? + label.len();
            let bytes: [u8; 8] = own.get(i..i + 8)?.try_into().ok()?;
            Some(f64::from_be_bytes(bytes) as f32).filter(|v| v.is_finite())
        };
        found.push(Preset {
            key: text(block, at + KEY.len()).unwrap_or_default(),
            name: rfind(own, b"Nm  TEXT").and_then(|i| text(own, i + 8)).filter(|n| !n.is_empty()),
            spacing: number(b"SpcnUntF#Prc").map(|v| v / 100.0).filter(|v| *v > 0.0),
            diameter: number(b"DmtrUntF#Pxl").filter(|v| *v > 0.0),
            angle: number(b"AnglUntF#Ang"),
            roundness: number(b"RndnUntF#Prc").map(|v| v / 100.0).filter(|v| *v > 0.0),
        });
        from = at + KEY.len();
    }
    found
}

/// A length-prefixed UTF-16 string at `at`.
fn text(data: &[u8], at: usize) -> Option<String> {
    let chars = u32::from_be_bytes(data.get(at..at + 4)?.try_into().ok()?) as usize;
    Some(utf16(data.get(at + 4..(at + 4).checked_add(chars.checked_mul(2)?)?)?))
}

fn utf16(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes.chunks_exact(2).map(|pair| u16::from_be_bytes([pair[0], pair[1]])).collect();
    String::from_utf16_lossy(&units).trim_matches(|c: char| c == '\0' || c.is_whitespace()).to_owned()
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn rfind(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).rposition(|w| w == needle)
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    find(haystack, needle).is_some()
}
