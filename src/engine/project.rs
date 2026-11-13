//! The SharkPaint project file format.
//!
//! Layout (all integers little-endian):
//!
//! ```text
//! magic      8 bytes  "SHARKPNT"
//! version    u32      currently 1
//! width      u32
//! height     u32
//! paper      3 bytes  sRGB paper colour
//! layers     u32      layer count
//! active     u32      index of the active layer
//! per layer:
//!   name     u32 length + UTF-8 bytes
//!   flags    u8       bit 0 = visible, bit 1 = transparency locked,
//!                     bit 2 = clipped to the layer beneath
//!   opacity  u16      0..=32768
//!   blend    u8       0 = normal
//!   tiles    u32      number of stored tiles
//!   per tile:
//!     index  u32      position in the layer's tile grid
//!     size   u32      compressed byte count
//!     data   zlib stream of 64×64 premultiplied RGBA pixels, u16 per channel
//! ```
//!
//! Tiles that are fully transparent are not stored. Layers are listed bottom first.
//!
//! An optional trailer holds the reference images (files without it are valid,
//! and readers that do not know it stop before it):
//!
//! ```text
//! magic      4 bytes  "REFS"
//! count      u32
//! per reference:
//!   name     u32 length + UTF-8 bytes
//!   width    u32
//!   height   u32
//!   size     u32      compressed byte count
//!   data     zlib stream of width × height RGBA pixels, u8 per channel
//! ```

use std::fs::File;
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::Path;
use std::sync::Arc;

use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use flate2::Compression;

use super::document::{Document, ProjectSnapshot, Reference};
use super::layer::{BlendMode, Layer, Tile, ONE, TILE};

/// File extension of SharkPaint projects, without the dot.
pub const PROJECT_EXTENSION: &str = "spaint";

const MAGIC: &[u8; 8] = b"SHARKPNT";
const VERSION: u32 = 1;
/// Largest canvas side accepted when reading a file.
const MAX_SIDE: u32 = 32_768;
const TILE_BYTES: usize = TILE * TILE * 8;
const REFERENCES_MAGIC: &[u8; 4] = b"REFS";
/// Largest reference image side accepted when reading a file.
const MAX_REFERENCE_SIDE: u32 = 16_384;

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.to_string())
}

fn read_u32(r: &mut impl Read) -> io::Result<u32> {
    let mut b = [0u8; 4];
    r.read_exact(&mut b)?;
    Ok(u32::from_le_bytes(b))
}

fn read_u16(r: &mut impl Read) -> io::Result<u16> {
    let mut b = [0u8; 2];
    r.read_exact(&mut b)?;
    Ok(u16::from_le_bytes(b))
}

fn read_u8(r: &mut impl Read) -> io::Result<u8> {
    let mut b = [0u8; 1];
    r.read_exact(&mut b)?;
    Ok(b[0])
}

impl ProjectSnapshot {
    /// Writes the project to `path`. The file is written to a temporary
    /// sibling first so a failed save never truncates an existing project.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        let mut tmp = path.as_os_str().to_owned();
        tmp.push(".tmp");
        let tmp = Path::new(&tmp);
        let result = (|| {
            let mut w = BufWriter::new(File::create(tmp)?);
            self.write_to(&mut w)?;
            w.into_inner().map_err(|e| e.into_error())?.sync_all()
        })();
        match result {
            Ok(()) => std::fs::rename(tmp, path),
            Err(e) => {
                let _ = std::fs::remove_file(tmp);
                Err(e)
            }
        }
    }

    pub fn write_to(&self, w: &mut impl Write) -> io::Result<()> {
        w.write_all(MAGIC)?;
        w.write_all(&VERSION.to_le_bytes())?;
        w.write_all(&(self.width as u32).to_le_bytes())?;
        w.write_all(&(self.height as u32).to_le_bytes())?;
        w.write_all(&self.paper)?;
        w.write_all(&(self.layers.len() as u32).to_le_bytes())?;
        w.write_all(&(self.active_layer as u32).to_le_bytes())?;
        let mut raw = vec![0u8; TILE_BYTES];
        for layer in &self.layers {
            let props = &layer.props;
            w.write_all(&(props.name.len() as u32).to_le_bytes())?;
            w.write_all(props.name.as_bytes())?;
            w.write_all(&[props.visible as u8 | (props.alpha_lock as u8) << 1 | (props.clip as u8) << 2])?;
            w.write_all(&(props.opacity15() as u16).to_le_bytes())?;
            w.write_all(&[props.blend.id()])?;

            let stored: Vec<(usize, &Tile)> = layer
                .tiles
                .iter()
                .enumerate()
                .filter_map(|(idx, t)| t.as_deref().map(|t| (idx, t)))
                .filter(|(_, t)| t.0.iter().any(|px| px[3] != 0))
                .collect();
            w.write_all(&(stored.len() as u32).to_le_bytes())?;
            for (idx, tile) in stored {
                for (px, out) in tile.0.iter().zip(raw.chunks_exact_mut(8)) {
                    for (c, o) in px.iter().zip(out.chunks_exact_mut(2)) {
                        o.copy_from_slice(&c.to_le_bytes());
                    }
                }
                let mut encoder = ZlibEncoder::new(Vec::new(), Compression::fast());
                encoder.write_all(&raw)?;
                let data = encoder.finish()?;
                w.write_all(&(idx as u32).to_le_bytes())?;
                w.write_all(&(data.len() as u32).to_le_bytes())?;
                w.write_all(&data)?;
            }
        }
        if !self.references.is_empty() {
            w.write_all(REFERENCES_MAGIC)?;
            w.write_all(&(self.references.len() as u32).to_le_bytes())?;
            for reference in &self.references {
                w.write_all(&(reference.name.len() as u32).to_le_bytes())?;
                w.write_all(reference.name.as_bytes())?;
                w.write_all(&(reference.width as u32).to_le_bytes())?;
                w.write_all(&(reference.height as u32).to_le_bytes())?;
                let mut encoder = ZlibEncoder::new(Vec::new(), Compression::fast());
                encoder.write_all(&reference.rgba)?;
                let data = encoder.finish()?;
                w.write_all(&(data.len() as u32).to_le_bytes())?;
                w.write_all(&data)?;
            }
        }
        Ok(())
    }
}

impl Document {
    /// Writes the document to `path`. See [`ProjectSnapshot::save`].
    pub fn save(&self, path: &Path) -> io::Result<()> {
        self.snapshot().save(path)
    }

    pub fn load(path: &Path) -> io::Result<Document> {
        Document::read_from(&mut BufReader::new(File::open(path)?))
    }

    pub fn write_to(&self, w: &mut impl Write) -> io::Result<()> {
        self.snapshot().write_to(w)
    }

    pub fn read_from(r: &mut impl Read) -> io::Result<Document> {
        let mut magic = [0u8; 8];
        r.read_exact(&mut magic)?;
        if &magic != MAGIC {
            return Err(invalid("no es un proyecto de SharkPaint"));
        }
        let version = read_u32(r)?;
        if version != VERSION {
            return Err(invalid("el proyecto se guardó con una versión más reciente de SharkPaint"));
        }
        let (width, height) = (read_u32(r)?, read_u32(r)?);
        if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE {
            return Err(invalid("tamaño de lienzo no válido"));
        }
        let (width, height) = (width as usize, height as usize);
        let mut paper = [0u8; 3];
        r.read_exact(&mut paper)?;
        let layer_count = read_u32(r)? as usize;
        let active = read_u32(r)? as usize;
        if layer_count == 0 || layer_count > 4096 || active >= layer_count {
            return Err(invalid("lista de capas no válida"));
        }

        let mut layers = Vec::with_capacity(layer_count);
        let mut raw = vec![0u8; TILE_BYTES];
        for _ in 0..layer_count {
            let name_len = read_u32(r)? as usize;
            if name_len > 4096 {
                return Err(invalid("nombre de capa no válido"));
            }
            let mut name = vec![0u8; name_len];
            r.read_exact(&mut name)?;
            let name = String::from_utf8(name).map_err(|_| invalid("nombre de capa no válido"))?;
            let (flags, opacity, blend) = (read_u8(r)?, read_u16(r)?, read_u8(r)?);

            let mut layer = Layer::new(width, height, name);
            layer.props.visible = flags & 1 != 0;
            layer.props.alpha_lock = flags & 2 != 0;
            layer.props.clip = flags & 4 != 0;
            layer.props.opacity = (opacity as f32 / ONE as f32).min(1.0);
            layer.props.blend = BlendMode::from_id(blend).ok_or_else(|| invalid("modo de fusión desconocido"))?;
            let tile_count = read_u32(r)? as usize;
            if tile_count > layer.tiles.len() {
                return Err(invalid("número de mosaicos no válido"));
            }
            for _ in 0..tile_count {
                let idx = read_u32(r)? as usize;
                let size = read_u32(r)? as u64;
                if idx >= layer.tiles.len() || size > 2 * TILE_BYTES as u64 + 1024 {
                    return Err(invalid("mosaico no válido"));
                }
                let mut decoder = ZlibDecoder::new(r.by_ref().take(size));
                decoder.read_exact(&mut raw)?;
                // Drain the rest of the stream so the reader ends on the next tile.
                io::copy(&mut decoder, &mut io::sink())?;
                io::copy(&mut decoder.into_inner(), &mut io::sink())?;

                let mut tile = Tile::empty();
                for (px, bytes) in tile.0.iter_mut().zip(raw.chunks_exact(8)) {
                    for (c, b) in px.iter_mut().zip(bytes.chunks_exact(2)) {
                        *c = u16::from_le_bytes([b[0], b[1]]).min(ONE as u16);
                    }
                    // Premultiplied colour can never exceed its alpha.
                    let a = px[3];
                    for c in &mut px[..3] {
                        *c = (*c).min(a);
                    }
                }
                layer.tiles[idx] = Some(Arc::new(tile));
            }
            layers.push(layer);
        }
        // Optional trailer with the reference images.
        let mut references = Vec::new();
        let mut magic = [0u8; 4];
        let trailer = match r.read(&mut magic)? {
            0 => false,
            n => {
                r.read_exact(&mut magic[n..])?;
                &magic == REFERENCES_MAGIC
            }
        };
        if trailer {
            let count = read_u32(r)?;
            if count > 1024 {
                return Err(invalid("lista de referencias no válida"));
            }
            for _ in 0..count {
                let name_len = read_u32(r)? as usize;
                if name_len > 4096 {
                    return Err(invalid("nombre de referencia no válido"));
                }
                let mut name = vec![0u8; name_len];
                r.read_exact(&mut name)?;
                let name = String::from_utf8(name).map_err(|_| invalid("nombre de referencia no válido"))?;
                let (w, h, size) = (read_u32(r)?, read_u32(r)?, read_u32(r)? as u64);
                if w == 0 || h == 0 || w > MAX_REFERENCE_SIDE || h > MAX_REFERENCE_SIDE {
                    return Err(invalid("tamaño de referencia no válido"));
                }
                let mut rgba = vec![0u8; w as usize * h as usize * 4];
                let mut decoder = ZlibDecoder::new(r.by_ref().take(size));
                decoder.read_exact(&mut rgba)?;
                io::copy(&mut decoder, &mut io::sink())?;
                io::copy(&mut decoder.into_inner(), &mut io::sink())?;
                references.push(Reference { name, width: w as usize, height: h as usize, rgba: Arc::new(rgba) });
            }
        }
        Ok(Document::from_snapshot(ProjectSnapshot { width, height, paper, layers, active_layer: active, references }))
    }
}
