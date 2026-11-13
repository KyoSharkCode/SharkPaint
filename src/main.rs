// Release builds on Windows run without a console window.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod dpi_fix;
mod icons;
mod settings;
mod view;

fn main() -> eframe::Result {
    // A project path as the first argument opens it (e.g. "Abrir con" in Explorer).
    let open_path = std::env::args_os().nth(1).map(std::path::PathBuf::from);
    let icon = eframe::icon_data::from_png_bytes(include_bytes!("../assets/icons/sharkpaint.png"))
        .expect("icono incrustado válido");
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("SharkPaint")
            .with_icon(icon)
            .with_inner_size([1400.0, 900.0])
            .with_min_inner_size([800.0, 500.0]),
        ..Default::default()
    };
    eframe::run_native(
        "SharkPaint",
        options,
        Box::new(move |cc| Ok(Box::new(app::SharkPaintApp::new(cc, open_path)))),
    )
}
