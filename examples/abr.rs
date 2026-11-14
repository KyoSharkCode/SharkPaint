//! Lists the brushes in a Photoshop brush file and draws their tips on a sheet.
//! Usage: cargo run --example abr -- pinceles.abr salida.png

use sharkpaint::engine::{read_abr, write_png};

fn main() {
    let mut args = std::env::args().skip(1);
    let input = args.next().expect("archivo .abr");
    let output = args.next().unwrap_or_else(|| "abr.png".into());
    let brushes = read_abr(&std::fs::read(&input).expect("leer el archivo")).expect("leer los pinceles");
    const CELL: usize = 128;
    let columns = 8.min(brushes.len());
    let rows = brushes.len().div_ceil(columns);
    let (w, h) = (columns * CELL, rows * CELL);
    let mut sheet = vec![255u8; w * h * 4];
    for (i, brush) in brushes.iter().enumerate() {
        println!(
            "{:3}  {}x{}  nombre={:?} tamaño={:?} espaciado={:?} ángulo={:?} redondez={:?} dureza={:?} presión: tamaño={} (mín. {:?}) flujo={}",
            i + 1, brush.width, brush.height, brush.name, brush.diameter, brush.spacing, brush.angle, brush.roundness,
            brush.hardness, brush.pressure_size, brush.min_size, brush.pressure_flow
        );
        if brush.is_round() {
            continue;
        }
        let scale = (brush.width.max(brush.height) as f32 / (CELL - 8) as f32).max(1.0);
        let (ox, oy) = (i % columns * CELL + 4, i / columns * CELL + 4);
        for y in 0..CELL - 8 {
            for x in 0..CELL - 8 {
                let (sx, sy) = ((x as f32 * scale) as usize, (y as f32 * scale) as usize);
                if sx < brush.width && sy < brush.height {
                    let value = 255 - brush.coverage[sy * brush.width + sx];
                    let at = ((oy + y) * w + ox + x) * 4;
                    sheet[at..at + 3].fill(value);
                }
            }
        }
    }
    write_png(output.as_ref(), w as u32, h as u32, &sheet).expect("escribir la hoja");
    println!("{} pinceles; hoja en {output}", brushes.len());
}
