//! Reads a Photoshop document, lists its layers and groups, and writes the
//! flattened picture as a PNG. With a third argument, also writes the document
//! back out as a `.psd`.
//! Usage: cargo run --example psd -- entrada.psd salida.png [copia.psd]

use sharkpaint::engine::{read_psd, write_png};

fn main() {
    let mut args = std::env::args().skip(1);
    let input = args.next().expect("archivo .psd");
    let output = args.next().unwrap_or_else(|| "psd.png".into());
    let doc = read_psd(&std::fs::read(&input).expect("leer el archivo")).expect("abrir el documento");
    println!("{} x {} px, {} capas, {} grupos", doc.width, doc.height, doc.layers().len(), doc.groups().len());
    for group in doc.groups() {
        println!("  grupo «{}» visible={} opacidad={:.2} modo={}", group.name, group.visible, group.opacity, group.blend.name());
    }
    for layer in doc.layers().iter().rev() {
        let props = &layer.props;
        let group = doc.group(layer.group).map_or(String::new(), |g| format!(" [{}]", g.name));
        println!(
            "  «{}»{group} visible={} opacidad={:.2} modo={} recorte={}",
            props.name, props.visible, props.opacity, props.blend.name(), props.clip
        );
    }
    let rgba = doc.render_rgba8(doc.bounds());
    write_png(output.as_ref(), doc.width as u32, doc.height as u32, &rgba).expect("escribir el PNG");
    if let Some(copy) = args.next() {
        let mut file = std::fs::File::create(&copy).expect("crear la copia");
        doc.write_psd(&mut file).expect("escribir el .psd");
        println!("copia en {copy}");
    }
}
