//! Reading and writing Photoshop documents (`.psd`).
//!
//! What carries over in both directions: the canvas size, each layer's
//! pixels, name, visibility, opacity, blend mode, clipping and transparency
//! lock, and layer groups with their name, visibility, opacity and blend
//! mode. Layer masks, adjustment layers, effects, text and vector data have no
//! counterpart here and are left out when reading; text and shapes come in as
//! the pixels Photoshop stored for them.
//!
//! Only 8- and 16-bit RGB and greyscale documents are read. Files are always
//! written as 8-bit RGB.
//!
//! A `.psd` lists its layers bottom first. A group is a run of layers between
//! two marker layers: a hidden divider below and, above, a heading that holds
//! the group's own settings.

use std::io::{self, Read, Write};
use std::sync::Arc;

use flate2::read::ZlibDecoder;

use super::document::{Document, ProjectSnapshot};
use super::layer::{BlendMode, IRect, Layer, LayerGroup, Px, Tile, ONE, TILE};

/// Name of the filled layer the paper is written as.
const PAPER_LAYER: &str = "Papel";

/// Largest canvas side accepted when reading a file.
const MAX_SIDE: u32 = 32_768;

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.to_owned())
}

/// Blend modes shared with Photoshop, by the four letters it names them with.
const BLEND_KEYS: [(BlendMode, &[u8; 4]); 7] = [
    (BlendMode::Normal, b"norm"),
    (BlendMode::Multiply, b"mul "),
    (BlendMode::Screen, b"scrn"),
    (BlendMode::Overlay, b"over"),
    (BlendMode::Darken, b"dark"),
    (BlendMode::Lighten, b"lite"),
    (BlendMode::Add, b"lddg"),
];

fn blend_from_key(key: &[u8]) -> BlendMode {
    BLEND_KEYS.iter().find(|(_, k)| k.as_slice() == key).map_or(BlendMode::Normal, |(mode, _)| *mode)
}

fn key_from_blend(mode: BlendMode) -> &'static [u8; 4] {
    BLEND_KEYS.iter().find(|(m, _)| *m == mode).map_or(b"norm", |(_, key)| key)
}

// --- Reading -----------------------------------------------------------------

struct Reader<'a> {
    data: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> io::Result<&'a [u8]> {
        let end = self.at.checked_add(n).filter(|&end| end <= self.data.len()).ok_or_else(|| invalid("el archivo está incompleto"))?;
        let bytes = &self.data[self.at..end];
        self.at = end;
        Ok(bytes)
    }
    fn u8(&mut self) -> io::Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> io::Result<u16> {
        Ok(u16::from_be_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn i16(&mut self) -> io::Result<i16> {
        Ok(self.u16()? as i16)
    }
    fn u32(&mut self) -> io::Result<u32> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn i32(&mut self) -> io::Result<i32> {
        Ok(self.u32()? as i32)
    }
    /// A length-prefixed part of the file, as a reader of its own.
    fn section(&mut self) -> io::Result<Reader<'a>> {
        let length = self.u32()? as usize;
        Ok(Reader { data: self.take(length)?, at: 0 })
    }
    fn left(&self) -> usize {
        self.data.len() - self.at
    }
}

/// One entry of the layer list, before it is known whether it is a layer, a
/// group's heading or a group's lower divider.
struct Record {
    rect: IRect,
    /// Channel id and stored length, in file order.
    channels: Vec<(i16, usize)>,
    blend: BlendMode,
    /// A group heading that lets its layers blend one by one.
    pass_through: bool,
    opacity: f32,
    clip: bool,
    visible: bool,
    alpha_lock: bool,
    name: String,
    /// 0 = a layer, 1 or 2 = heading of an open or folded group, 3 = divider.
    section: u32,
}

/// Reads a Photoshop document.
pub fn read_psd(data: &[u8]) -> io::Result<Document> {
    let mut r = Reader { data, at: 0 };
    if r.take(4).ok() != Some(b"8BPS") {
        return Err(invalid("no es un documento de Photoshop"));
    }
    match r.u16()? {
        1 => {}
        2 => return Err(invalid("los documentos grandes de Photoshop (.psb) no se pueden abrir")),
        _ => return Err(invalid("versión de documento desconocida")),
    }
    r.take(6)?;
    let channels = r.u16()? as usize;
    let (height, width) = (r.u32()?, r.u32()?);
    let depth = r.u16()?;
    let mode = r.u16()?;
    if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE {
        return Err(invalid("tamaño de lienzo no válido"));
    }
    if depth != 8 && depth != 16 {
        return Err(invalid("solo se abren documentos de 8 o 16 bits por canal"));
    }
    let grey = match mode {
        3 => false,
        1 => true,
        4 => return Err(invalid("los documentos CMYK no se pueden abrir; conviértelo a RGB")),
        _ => return Err(invalid("solo se abren documentos RGB o en escala de grises")),
    };
    let (width, height) = (width as usize, height as usize);
    let bytes = (depth / 8) as usize;
    r.section()?; // colour table
    r.section()?; // image resources

    let mut layers: Vec<Layer> = Vec::new();
    let mut groups: Vec<LayerGroup> = Vec::new();
    let mut section = r.section()?;
    if section.left() >= 4 {
        let mut info = section.section()?;
        if info.left() >= 2 {
            read_layers(&mut info, width, height, bytes, grey, &mut layers, &mut groups)?;
        }
        // Past the (unused) global mask, 16-bit documents keep their layers in
        // a tagged block instead.
        if layers.is_empty() && section.left() >= 4 && section.section().is_ok() {
            while section.left() >= 12 {
                let signature = section.take(4)?;
                if signature != b"8BIM" && signature != b"8B64" {
                    break;
                }
                let key = section.take(4)?;
                let length = section.u32()? as usize;
                let Ok(body) = section.take(length.min(section.left())) else { break };
                if key == b"Lr16" || key == b"Lr32" {
                    let mut info = Reader { data: body, at: 0 };
                    read_layers(&mut info, width, height, bytes, grey, &mut layers, &mut groups)?;
                    break;
                }
                section.at = (section.at + 3) & !3;
            }
        }
    }

    if layers.is_empty() {
        // A flattened document: the picture itself is the only layer.
        let mut layer = Layer::new(width, height, "Fondo");
        let compression = r.u16()?;
        let rows = channels * height;
        let lengths: Vec<usize> = match compression {
            0 => Vec::new(),
            1 => (0..rows).map(|_| r.u16().map(|n| n as usize)).collect::<io::Result<_>>()?,
            _ => return Err(invalid("compresión de imagen no admitida")),
        };
        let mut planes: Vec<Vec<u8>> = Vec::new();
        for channel in 0..channels.min(if grey { 1 } else { 3 }) {
            let mut plane = Vec::with_capacity(width * height);
            for row in 0..height {
                let line = match compression {
                    0 => r.take(width * bytes)?.to_vec(),
                    _ => unpack(r.take(lengths[channel * height + row])?, width * bytes),
                };
                plane.extend(line.chunks_exact(bytes).map(|sample| sample[0]));
            }
            planes.push(plane);
        }
        if planes.is_empty() {
            return Err(invalid("el documento no tiene imagen"));
        }
        let pick = |c: usize| planes.get(c).unwrap_or(&planes[0]);
        put_pixels(&mut layer, IRect::new(0, 0, width as i32, height as i32), pick(0), pick(1), pick(2), None);
        layers.push(layer);
    }
    // A file written here starts with the paper as a filled layer: take it back.
    let mut paper = [255, 255, 255];
    if layers.len() > 1 {
        if let Some(colour) = paper_colour(&layers[0]) {
            paper = colour;
            layers.remove(0);
        }
    }
    let active_layer = layers.len() - 1;
    Ok(Document::from_snapshot(ProjectSnapshot { width, height, paper, layers, groups, active_layer, references: Vec::new() }))
}

/// The colour of a layer that stands for the paper: named as this program
/// names it, plain, and one opaque colour over the whole canvas.
fn paper_colour(layer: &Layer) -> Option<[u8; 3]> {
    let props = &layer.props;
    if props.name != PAPER_LAYER || layer.group != 0 || !props.visible || props.opacity < 1.0 || props.blend != BlendMode::Normal {
        return None;
    }
    let first = layer.pixel(0, 0);
    if first[3] as u32 != ONE {
        return None;
    }
    for y in 0..layer.height as i32 {
        for x in 0..layer.width as i32 {
            if layer.pixel(x, y) != first {
                return None;
            }
        }
    }
    Some([0, 1, 2].map(|c| ((first[c] as u32 * 255 + ONE / 2) >> 15) as u8))
}

/// Reads the layer list and each layer's pixels.
fn read_layers(
    r: &mut Reader,
    width: usize,
    height: usize,
    bytes: usize,
    grey: bool,
    layers: &mut Vec<Layer>,
    groups: &mut Vec<LayerGroup>,
) -> io::Result<()> {
    let count = r.i16()?.unsigned_abs() as usize;
    let mut records = Vec::with_capacity(count.min(4096));
    for _ in 0..count {
        let (top, left, bottom, right) = (r.i32()?, r.i32()?, r.i32()?, r.i32()?);
        let channel_count = r.u16()? as usize;
        let mut channels = Vec::with_capacity(channel_count.min(64));
        for _ in 0..channel_count {
            channels.push((r.i16()?, r.u32()? as usize));
        }
        if r.take(4)? != b"8BIM" {
            return Err(invalid("lista de capas dañada"));
        }
        let blend_key = r.take(4)?;
        let opacity = r.u8()? as f32 / 255.0;
        let clip = r.u8()? != 0;
        let flags = r.u8()?;
        r.u8()?;
        let mut extra = r.section()?;
        extra.section()?; // layer mask
        extra.section()?; // blending ranges
        let name_length = extra.u8()? as usize;
        let mut name: String = extra.take(name_length)?.iter().map(|&b| b as char).collect();
        extra.at = (extra.at + 3) & !3; // the short name is padded to four bytes
        let mut section = 0;
        let mut section_key: Option<&[u8]> = None;
        while extra.left() >= 12 {
            let signature = extra.take(4)?;
            if signature != b"8BIM" && signature != b"8B64" {
                break;
            }
            let key = extra.take(4)?;
            let length = extra.u32()? as usize;
            let mut body = Reader { data: extra.take(length.min(extra.left()))?, at: 0 };
            match key {
                // The full name; the short one above is cut to 255 bytes of an old encoding.
                b"luni" => {
                    let units = body.u32()? as usize;
                    let raw = body.take((units * 2).min(body.left()))?;
                    let wide: Vec<u16> = raw.chunks_exact(2).map(|pair| u16::from_be_bytes([pair[0], pair[1]])).collect();
                    name = String::from_utf16_lossy(&wide).trim_end_matches('\0').to_owned();
                }
                b"lsct" | b"lsdk" => {
                    section = body.u32()?;
                    if body.left() >= 8 {
                        body.take(4)?;
                        section_key = Some(body.take(4)?);
                    }
                }
                _ => {}
            }
            if length % 2 == 1 && extra.left() > 0 {
                extra.at += 1; // blocks are padded to an even length
            }
        }
        let rect = IRect::new(left, top, right, bottom);
        let sane = |v: i32| (-(1 << 20)..(1 << 20)).contains(&v);
        if ![top, left, bottom, right].into_iter().all(sane) {
            return Err(invalid("medidas de capa no válidas"));
        }
        let heading_key = section_key.unwrap_or(blend_key);
        records.push(Record {
            rect,
            channels,
            blend: blend_from_key(if section == 1 || section == 2 { heading_key } else { blend_key }),
            pass_through: heading_key == b"pass",
            opacity,
            clip,
            visible: flags & 2 == 0,
            alpha_lock: flags & 1 != 0,
            name,
            section,
        });
    }

    // Layers of the groups still open, innermost last. Only outermost groups
    // become groups here; the settings of inner ones are folded into their layers.
    let mut open: Vec<Vec<usize>> = Vec::new();
    for record in &records {
        let (w, h) = (record.rect.width(), record.rect.height());
        let mut planes: [Option<Vec<u8>>; 4] = [None, None, None, None]; // red, green, blue, alpha
        for &(id, length) in &record.channels {
            let stored = r.take(length)?;
            let slot = match id {
                0..=2 => id as usize,
                -1 => 3,
                _ => continue, // masks and anything else
            };
            if w == 0 || h == 0 || stored.len() < 2 {
                continue;
            }
            planes[slot] = Some(channel(stored, w, h, bytes)?);
        }
        match record.section {
            3 => open.push(Vec::new()),
            1 | 2 => {
                let members = open.pop().unwrap_or_default();
                if let Some(outer) = open.last_mut() {
                    for &index in &members {
                        let props = &mut layers[index].props;
                        props.visible &= record.visible;
                        props.opacity *= record.opacity;
                    }
                    outer.extend(members);
                } else if !members.is_empty() {
                    let id = groups.len() as u64 + 1;
                    groups.push(LayerGroup {
                        id,
                        name: record.name.clone(),
                        visible: record.visible,
                        opacity: record.opacity,
                        blend: if record.pass_through { BlendMode::Normal } else { record.blend },
                        collapsed: record.section == 2,
                    });
                    for index in members {
                        layers[index].group = id;
                    }
                }
            }
            _ => {
                let [red, green, blue, alpha] = planes;
                // Without pixels it is an adjustment or some other kind of layer.
                let Some(red) = red else { continue };
                let (green, blue) = if grey { (&red, &red) } else { (green.as_ref().unwrap_or(&red), blue.as_ref().unwrap_or(&red)) };
                let mut layer = Layer::new(width, height, record.name.as_str());
                layer.props.visible = record.visible;
                layer.props.opacity = record.opacity;
                layer.props.blend = record.blend;
                layer.props.clip = record.clip;
                layer.props.alpha_lock = record.alpha_lock;
                put_pixels(&mut layer, record.rect, &red, green, blue, alpha.as_deref());
                if let Some(group) = open.last_mut() {
                    group.push(layers.len());
                }
                layers.push(layer);
            }
        }
    }
    Ok(())
}

/// One stored channel of a layer, as 8-bit samples.
fn channel(stored: &[u8], width: usize, height: usize, bytes: usize) -> io::Result<Vec<u8>> {
    let mut r = Reader { data: stored, at: 0 };
    let row = width * bytes;
    let raw: Vec<u8> = match r.u16()? {
        0 => r.take(row * height)?.to_vec(),
        1 => {
            let lengths: Vec<usize> = (0..height).map(|_| r.u16().map(|n| n as usize)).collect::<io::Result<_>>()?;
            let mut out = Vec::with_capacity(row * height);
            for length in lengths {
                out.extend(unpack(r.take(length)?, row));
            }
            out
        }
        kind @ (2 | 3) => {
            let mut out = vec![0u8; row * height];
            ZlibDecoder::new(&stored[2..]).read_exact(&mut out)?;
            if kind == 3 {
                // Each sample is stored as the difference from the one to its left.
                for line in out.chunks_exact_mut(row) {
                    if bytes == 1 {
                        for x in 1..width {
                            line[x] = line[x].wrapping_add(line[x - 1]);
                        }
                    } else {
                        for x in 1..width {
                            let previous = u16::from_be_bytes([line[x * 2 - 2], line[x * 2 - 1]]);
                            let value = u16::from_be_bytes([line[x * 2], line[x * 2 + 1]]).wrapping_add(previous);
                            line[x * 2..x * 2 + 2].copy_from_slice(&value.to_be_bytes());
                        }
                    }
                }
            }
            out
        }
        _ => return Err(invalid("compresión de capa desconocida")),
    };
    Ok(raw.chunks_exact(bytes).map(|sample| sample[0]).collect())
}

/// PackBits: a count byte, then that many literal bytes or one byte to repeat.
/// Always returns exactly `length` bytes.
fn unpack(packed: &[u8], length: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(length);
    let mut i = 0;
    while i < packed.len() && out.len() < length {
        let n = packed[i] as i8;
        i += 1;
        if n >= 0 {
            let count = (n as usize + 1).min(packed.len() - i).min(length - out.len());
            out.extend_from_slice(&packed[i..i + count]);
            i += n as usize + 1;
        } else if n != -128 {
            let count = ((1 - n as isize) as usize).min(length - out.len());
            if let Some(&value) = packed.get(i) {
                out.extend(std::iter::repeat_n(value, count));
            }
            i += 1;
        }
    }
    out.resize(length, 0);
    out
}

/// Writes 8-bit colour that is not premultiplied into a layer; `rect` is where
/// the samples sit on the canvas, and whatever falls outside it is dropped.
fn put_pixels(layer: &mut Layer, rect: IRect, red: &[u8], green: &[u8], blue: &[u8], alpha: Option<&[u8]>) {
    let width = rect.width();
    let area = rect.intersect(&layer.bounds());
    for y in area.y0..area.y1 {
        for x in area.x0..area.x1 {
            let i = (y - rect.y0) as usize * width + (x - rect.x0) as usize;
            let a = alpha.map_or(255, |plane| plane.get(i).copied().unwrap_or(0)) as u32;
            if a == 0 {
                continue;
            }
            let a15 = (a * ONE + 127) / 255;
            let part = |plane: &[u8]| ((plane.get(i).copied().unwrap_or(0) as u32 * ONE + 127) / 255 * a15 + ONE / 2) >> 15;
            let px: Px = [part(red) as u16, part(green) as u16, part(blue) as u16, a15 as u16];
            let (ux, uy) = (x as usize, y as usize);
            let index = layer.tile_index(ux / TILE, uy / TILE);
            let tile = Arc::make_mut(layer.tiles[index].get_or_insert_with(|| Arc::new(Tile::empty())));
            tile.0[(uy % TILE) * TILE + ux % TILE] = px;
        }
    }
}

// --- Writing -----------------------------------------------------------------

/// PackBits for one row.
fn pack(row: &[u8], out: &mut Vec<u8>) {
    let mut i = 0;
    while i < row.len() {
        let mut run = 1;
        while i + run < row.len() && run < 128 && row[i + run] == row[i] {
            run += 1;
        }
        if run >= 2 {
            out.push((1 - run as i32) as u8);
            out.push(row[i]);
            i += run;
            continue;
        }
        // Literals up to the next run of three, which is where repeating pays.
        let start = i;
        while i < row.len() && i - start < 128 {
            if i + 2 < row.len() && row[i] == row[i + 1] && row[i] == row[i + 2] {
                break;
            }
            i += 1;
        }
        out.push((i - start - 1) as u8);
        out.extend_from_slice(&row[start..i]);
    }
}

/// A channel as stored in a layer: compression kind, row lengths, packed rows.
fn packed_channel(plane: &[u8], width: usize, height: usize) -> Vec<u8> {
    if width == 0 || height == 0 {
        return vec![0, 0];
    }
    let mut rows = Vec::new();
    let mut lengths = Vec::with_capacity(height * 2);
    for row in plane.chunks_exact(width) {
        let before = rows.len();
        pack(row, &mut rows);
        lengths.extend(((rows.len() - before) as u16).to_be_bytes());
    }
    let mut out = vec![0, 1];
    out.extend(lengths);
    out.extend(rows);
    out
}

/// One entry of the layer list as it goes into the file.
struct Entry {
    name: String,
    rect: IRect,
    /// Alpha, red, green, blue.
    planes: [Vec<u8>; 4],
    blend_key: [u8; 4],
    opacity: f32,
    clip: bool,
    visible: bool,
    alpha_lock: bool,
    /// Group marker: 3 below a group, 1 or 2 above it (open or folded).
    section: Option<(u32, [u8; 4])>,
}

impl Entry {
    fn marker(name: &str, section: u32, key: [u8; 4], visible: bool, opacity: f32) -> Entry {
        Entry {
            name: name.to_owned(),
            rect: IRect::new(0, 0, 0, 0),
            planes: Default::default(),
            blend_key: key,
            opacity,
            clip: false,
            visible,
            alpha_lock: false,
            section: Some((section, key)),
        }
    }
}

/// The part of a layer that has anything in it, as 8-bit planes.
fn layer_entry(layer: &Layer) -> Entry {
    let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0usize, 0usize);
    for ty in 0..layer.tiles_y {
        for tx in 0..layer.tiles_x {
            let Some(tile) = &layer.tiles[layer.tile_index(tx, ty)] else { continue };
            for py in 0..TILE {
                for px in 0..TILE {
                    if tile.0[py * TILE + px][3] != 0 {
                        let (x, y) = (tx * TILE + px, ty * TILE + py);
                        x0 = x0.min(x);
                        x1 = x1.max(x);
                        y0 = y0.min(y);
                        y1 = y1.max(y);
                    }
                }
            }
        }
    }
    let rect = if x0 == usize::MAX {
        IRect::new(0, 0, 0, 0)
    } else {
        IRect::new(x0 as i32, y0 as i32, x1 as i32 + 1, y1 as i32 + 1).intersect(&layer.bounds())
    };
    let size = rect.width() * rect.height();
    let mut planes: [Vec<u8>; 4] = std::array::from_fn(|_| Vec::with_capacity(size));
    for y in rect.y0..rect.y1 {
        for x in rect.x0..rect.x1 {
            let px = layer.pixel(x, y);
            let a = px[3] as u32;
            planes[0].push(((a * 255 + ONE / 2) >> 15) as u8);
            for c in 0..3 {
                // Back to colour that is not premultiplied.
                let value = if a == 0 { 0 } else { ((px[c] as u32).min(a) * 255 + a / 2) / a };
                planes[c + 1].push(value as u8);
            }
        }
    }
    let props = &layer.props;
    Entry {
        name: props.name.clone(),
        rect,
        planes,
        blend_key: *key_from_blend(props.blend),
        opacity: props.opacity,
        clip: props.clip,
        visible: props.visible,
        alpha_lock: props.alpha_lock,
        section: None,
    }
}

impl Document {
    /// Writes the document as an 8-bit RGB Photoshop file. The paper becomes a
    /// filled layer at the bottom, so the picture looks the same there.
    pub fn write_psd(&self, w: &mut impl Write) -> io::Result<()> {
        let (width, height) = (self.width, self.height);
        let mut entries = Vec::new();
        let size = width * height;
        entries.push(Entry {
            name: PAPER_LAYER.into(),
            rect: self.bounds(),
            planes: [vec![255; size], vec![self.paper[0]; size], vec![self.paper[1]; size], vec![self.paper[2]; size]],
            blend_key: *b"norm",
            opacity: 1.0,
            clip: false,
            visible: true,
            alpha_lock: false,
            section: None,
        });
        let layers = self.layers();
        for (i, layer) in layers.iter().enumerate() {
            let group = self.group(layer.group);
            if let Some(group) = group.filter(|g| i == 0 || layers[i - 1].group != g.id) {
                let _ = group;
                entries.push(Entry::marker("</Layer group>", 3, *b"norm", true, 1.0));
            }
            entries.push(layer_entry(layer));
            if let Some(group) = group.filter(|g| layers.get(i + 1).is_none_or(|next| next.group != g.id)) {
                let key = if group.blend == BlendMode::Normal { *b"pass" } else { *key_from_blend(group.blend) };
                entries.push(Entry::marker(&group.name, if group.collapsed { 2 } else { 1 }, key, group.visible, group.opacity));
            }
        }

        // The layer list, then every layer's channels.
        let mut info = Vec::new();
        info.extend((entries.len() as i16).to_be_bytes());
        let mut pixels = Vec::new();
        for entry in &entries {
            let (w0, h0) = (entry.rect.width(), entry.rect.height());
            for edge in [entry.rect.y0, entry.rect.x0, entry.rect.y1, entry.rect.x1] {
                info.extend(edge.to_be_bytes());
            }
            info.extend(4u16.to_be_bytes());
            for (id, plane) in [-1i16, 0, 1, 2].into_iter().zip(&entry.planes) {
                let stored = packed_channel(plane, w0, h0);
                info.extend(id.to_be_bytes());
                info.extend((stored.len() as u32).to_be_bytes());
                pixels.extend(stored);
            }
            info.extend(b"8BIM");
            info.extend(entry.blend_key);
            info.push((entry.opacity.clamp(0.0, 1.0) * 255.0).round() as u8);
            info.push(entry.clip as u8);
            info.push(entry.alpha_lock as u8 | (!entry.visible as u8) << 1);
            info.push(0);

            let mut extra = Vec::new();
            extra.extend(0u32.to_be_bytes()); // no mask
            extra.extend(0u32.to_be_bytes()); // no blending ranges
            // The short name, in the old one-byte encoding, padded to four bytes.
            let short: Vec<u8> = entry.name.chars().take(255).map(|c| if c.is_ascii() { c as u8 } else { b'?' }).collect();
            extra.push(short.len() as u8);
            extra.extend(&short);
            extra.resize(extra.len().div_ceil(4) * 4, 0);
            // The full name.
            let wide: Vec<u16> = entry.name.encode_utf16().collect();
            let mut body = (wide.len() as u32).to_be_bytes().to_vec();
            body.extend(wide.iter().flat_map(|unit| unit.to_be_bytes()));
            body.resize(body.len().div_ceil(4) * 4, 0);
            extra.extend(b"8BIMluni");
            extra.extend((body.len() as u32).to_be_bytes());
            extra.extend(body);
            if let Some((kind, key)) = entry.section {
                extra.extend(b"8BIMlsct");
                extra.extend(12u32.to_be_bytes());
                extra.extend(kind.to_be_bytes());
                extra.extend(b"8BIM");
                extra.extend(key);
            }
            info.extend((extra.len() as u32).to_be_bytes());
            info.extend(extra);
        }
        info.extend(pixels);
        if info.len() % 2 == 1 {
            info.push(0);
        }

        w.write_all(b"8BPS")?;
        w.write_all(&1u16.to_be_bytes())?;
        w.write_all(&[0; 6])?;
        w.write_all(&3u16.to_be_bytes())?;
        w.write_all(&(height as u32).to_be_bytes())?;
        w.write_all(&(width as u32).to_be_bytes())?;
        w.write_all(&8u16.to_be_bytes())?;
        w.write_all(&3u16.to_be_bytes())?;
        w.write_all(&0u32.to_be_bytes())?; // no colour table
        w.write_all(&0u32.to_be_bytes())?; // no image resources
        w.write_all(&(info.len() as u32 + 8).to_be_bytes())?;
        w.write_all(&(info.len() as u32).to_be_bytes())?;
        w.write_all(&info)?;
        w.write_all(&0u32.to_be_bytes())?; // no global mask

        // The flattened picture, for programs that only show a preview.
        let flat = self.render_rgba8(self.bounds());
        let mut lengths = Vec::with_capacity(3 * height * 2);
        let mut rows = Vec::new();
        let mut line = vec![0u8; width];
        for channel in 0..3 {
            for row in flat.chunks_exact(width * 4) {
                for (out, px) in line.iter_mut().zip(row.chunks_exact(4)) {
                    *out = px[channel];
                }
                let before = rows.len();
                pack(&line, &mut rows);
                lengths.extend(((rows.len() - before) as u16).to_be_bytes());
            }
        }
        w.write_all(&1u16.to_be_bytes())?;
        w.write_all(&lengths)?;
        w.write_all(&rows)
    }
}
