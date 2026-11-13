//! Embeds the application icon into the Windows executable.

fn main() {
    println!("cargo:rerun-if-changed=assets/icons/sharkpaint.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut resource = winresource::WindowsResource::new();
        resource.set_icon("assets/icons/sharkpaint.ico");
        resource.compile().expect("no se pudo incrustar el icono");
    }
}
