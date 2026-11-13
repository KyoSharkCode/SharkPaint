//! Desktop interface: menus, tool and brush panels, and the canvas viewport.

use eframe::egui::{
    self, Color32, ColorImage, CursorIcon, Event, Key, Mesh, PointerButton, Pos2, Rect, Sense,
    Shape, Stroke, TextureFilter, TextureHandle, TextureOptions, TouchPhase, Vec2,
};

use std::path::PathBuf;

use sharkpaint::engine::{
    write_png, BrushPreset, BrushSettings, Document, IRect, StrokePoint, PROJECT_EXTENSION,
};

use crate::view::View;

const DEFAULT_SIZE: [usize; 2] = [3000, 2000];
const MAX_BRUSH_SIZE: f32 = 500.0;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tool {
    Pincel,
    Borrador,
}

/// Menu and keyboard commands, applied once per frame after the UI is built.
#[derive(Clone, Copy, PartialEq)]
enum Command {
    NewDialog,
    Open,
    Save,
    SaveAs,
    ExportPng,
    Undo,
    Redo,
    ClearLayer,
    Fit,
    ActualSize,
    Zoom(f32),
    Rotate(f32),
    ResetRotation,
    Flip,
    Quit,
}

/// An action that discards the current drawing, held until unsaved changes
/// are saved or explicitly dropped.
#[derive(Clone, Copy, PartialEq)]
enum Discarding {
    New,
    Open,
    Quit,
}

struct NewCanvasDialog {
    width: usize,
    height: usize,
}

pub struct SharkPaintApp {
    doc: Document,
    view: View,
    texture: Option<TextureHandle>,
    texture_options: TextureOptions,
    /// Screen area of the canvas viewport on the last frame.
    viewport: Rect,
    tool: Tool,
    brush: BrushSettings,
    eraser: BrushSettings,
    preset: Option<BrushPreset>,
    color: [u8; 3],
    /// Pressure of the pen currently touching the tablet, if any.
    pen_pressure: Option<f32>,
    /// Pressure used for the most recent stroke sample (status bar readout).
    last_pressure: f32,
    /// Canvas position of a press whose first dab waits for a pressure reading.
    pending_start: Option<Pos2>,
    /// Latest pen pressure reported since the current press began.
    contact_pressure: Option<f32>,
    /// A pen or finger is currently touching the tablet.
    touch_active: bool,
    /// The current press came from a pen or finger rather than the mouse.
    press_is_touch: bool,
    pen_seen: bool,
    fit_pending: bool,
    new_dialog: Option<NewCanvasDialog>,
    status: String,
    /// Project file the document was opened from or last saved to.
    path: Option<PathBuf>,
    /// Document revision at the last save or load.
    saved_revision: u64,
    /// Action waiting on the "unsaved changes" prompt.
    confirm: Option<Discarding>,
    /// The window may close even with unsaved changes (the user chose to).
    allow_close: bool,
    window_title: String,
}

impl SharkPaintApp {
    pub fn new(cc: &eframe::CreationContext<'_>, open_path: Option<PathBuf>) -> Self {
        cc.egui_ctx.set_theme(egui::Theme::Dark);
        // Ctrl +/- must zoom the canvas, not scale the whole interface.
        cc.egui_ctx.options_mut(|o| o.zoom_with_keyboard = false);
        let mut eraser = BrushPreset::Marcador.settings();
        eraser.opacity = 1.0;
        let mut app = SharkPaintApp {
            doc: Document::new(DEFAULT_SIZE[0], DEFAULT_SIZE[1]),
            view: View::default(),
            texture: None,
            texture_options: TextureOptions::default(),
            viewport: Rect::ZERO,
            tool: Tool::Pincel,
            brush: BrushPreset::Tinta.settings(),
            eraser,
            preset: Some(BrushPreset::Tinta),
            color: [24, 24, 32],
            pen_pressure: None,
            last_pressure: 1.0,
            pending_start: None,
            contact_pressure: None,
            touch_active: false,
            press_is_touch: false,
            pen_seen: false,
            fit_pending: true,
            new_dialog: None,
            status: String::new(),
            path: None,
            saved_revision: 0,
            confirm: None,
            allow_close: false,
            window_title: String::new(),
        };
        if let Some(path) = open_path {
            app.load_project(path);
        }
        app
    }

    fn modified(&self) -> bool {
        self.doc.revision() != self.saved_revision
    }

    fn replace_document(&mut self, doc: Document, path: Option<PathBuf>) {
        self.saved_revision = doc.revision();
        self.doc = doc;
        self.path = path;
        self.texture = None;
        self.view = View::default();
        self.fit_pending = true;
        self.pending_start = None;
    }

    fn load_project(&mut self, path: PathBuf) {
        match Document::load(&path) {
            Ok(doc) => {
                self.status = format!("Abierto: {}", path.display());
                self.replace_document(doc, Some(path));
            }
            Err(e) => self.status = format!("No se pudo abrir {}: {e}", path.display()),
        }
    }

    /// Saves to the current project file, asking for one if there is none.
    /// Returns whether the document is now saved.
    fn save_project(&mut self, ask_path: bool) -> bool {
        self.doc.end_stroke();
        let path = match (&self.path, ask_path) {
            (Some(path), false) => path.clone(),
            _ => {
                let name = self
                    .path
                    .as_ref()
                    .and_then(|p| p.file_name())
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| format!("dibujo.{PROJECT_EXTENSION}"));
                let Some(mut path) = rfd::FileDialog::new()
                    .set_title("Guardar proyecto")
                    .add_filter("Proyecto de SharkPaint", &[PROJECT_EXTENSION])
                    .set_file_name(name)
                    .save_file()
                else {
                    return false;
                };
                if path.extension().is_none() {
                    path.set_extension(PROJECT_EXTENSION);
                }
                path
            }
        };
        match self.doc.save(&path) {
            Ok(()) => {
                self.status = format!("Guardado: {}", path.display());
                self.saved_revision = self.doc.revision();
                self.path = Some(path);
                true
            }
            Err(e) => {
                self.status = format!("No se pudo guardar: {e}");
                false
            }
        }
    }

    /// Runs an action that discards the drawing, asking first if it has unsaved changes.
    fn discarding(&mut self, action: Discarding, ctx: &egui::Context) {
        if self.modified() {
            self.confirm = Some(action);
        } else {
            self.discard_and(action, ctx);
        }
    }

    fn discard_and(&mut self, action: Discarding, ctx: &egui::Context) {
        match action {
            Discarding::New => {
                self.new_dialog = Some(NewCanvasDialog { width: self.doc.width, height: self.doc.height });
            }
            Discarding::Open => {
                let picked = rfd::FileDialog::new()
                    .set_title("Abrir proyecto")
                    .add_filter("Proyecto de SharkPaint", &[PROJECT_EXTENSION])
                    .pick_file();
                if let Some(path) = picked {
                    self.load_project(path);
                }
            }
            Discarding::Quit => {
                self.allow_close = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }

    fn confirm_window(&mut self, ctx: &egui::Context) {
        let Some(action) = self.confirm else { return };
        let mut choice = None;
        egui::Window::new("Cambios sin guardar")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                ui.label("El dibujo tiene cambios sin guardar. ¿Quieres guardarlos antes de continuar?");
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button("Guardar").clicked() {
                        choice = Some(0);
                    }
                    if ui.button("No guardar").clicked() {
                        choice = Some(1);
                    }
                    if ui.button("Cancelar").clicked() {
                        choice = Some(2);
                    }
                });
            });
        match choice {
            Some(0) => {
                self.confirm = None;
                if self.save_project(false) {
                    self.discard_and(action, ctx);
                }
            }
            Some(1) => {
                self.confirm = None;
                self.discard_and(action, ctx);
            }
            Some(2) => self.confirm = None,
            _ => {}
        }
    }

    fn update_window_title(&mut self, ctx: &egui::Context) {
        let name = self
            .path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Sin título".to_owned());
        let mark = if self.modified() { "*" } else { "" };
        let title = format!("{name}{mark} — SharkPaint");
        if title != self.window_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.window_title = title;
        }
    }

    fn doc_size(&self) -> Vec2 {
        Vec2::new(self.doc.width as f32, self.doc.height as f32)
    }

    fn active_brush(&mut self) -> &mut BrushSettings {
        match self.tool {
            Tool::Pincel => &mut self.brush,
            Tool::Borrador => &mut self.eraser,
        }
    }

    fn keyboard_commands(&mut self, ctx: &egui::Context, commands: &mut Vec<Command>) {
        if ctx.text_edit_focused() {
            return;
        }
        ctx.input_mut(|i| {
            let cmd = egui::Modifiers::COMMAND;
            let none = egui::Modifiers::NONE;
            let mut hit = |mods, key| i.consume_key(mods, key);
            if hit(cmd | egui::Modifiers::SHIFT, Key::Z) || hit(cmd, Key::Y) {
                commands.push(Command::Redo);
            }
            if hit(cmd, Key::Z) {
                commands.push(Command::Undo);
            }
            if hit(cmd | egui::Modifiers::SHIFT, Key::S) {
                commands.push(Command::SaveAs);
            }
            if hit(cmd, Key::S) {
                commands.push(Command::Save);
            }
            if hit(cmd, Key::O) {
                commands.push(Command::Open);
            }
            if hit(cmd, Key::E) {
                commands.push(Command::ExportPng);
            }
            if hit(cmd, Key::N) {
                commands.push(Command::NewDialog);
            }
            if hit(cmd, Key::Num0) {
                commands.push(Command::Fit);
            }
            if hit(cmd, Key::Num1) {
                commands.push(Command::ActualSize);
            }
            if hit(cmd, Key::Plus) || hit(cmd, Key::Equals) {
                commands.push(Command::Zoom(1.25));
            }
            if hit(cmd, Key::Minus) {
                commands.push(Command::Zoom(0.8));
            }
            if hit(none, Key::Num4) {
                commands.push(Command::Rotate(-15.0));
            }
            if hit(none, Key::Num6) {
                commands.push(Command::Rotate(15.0));
            }
            if hit(none, Key::Num5) {
                commands.push(Command::ResetRotation);
            }
            if hit(none, Key::M) {
                commands.push(Command::Flip);
            }
            if hit(none, Key::B) {
                self.tool = Tool::Pincel;
            }
            if hit(none, Key::E) {
                self.tool = Tool::Borrador;
            }
            let brush = match self.tool {
                Tool::Pincel => &mut self.brush,
                Tool::Borrador => &mut self.eraser,
            };
            if hit(none, Key::OpenBracket) {
                brush.size = (brush.size / 1.15).max(1.0);
            }
            if hit(none, Key::CloseBracket) {
                brush.size = (brush.size * 1.15 + 0.5).min(MAX_BRUSH_SIZE);
            }
        });
    }

    fn menu_bar(&mut self, ui: &mut egui::Ui, commands: &mut Vec<Command>) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("Archivo", |ui| {
                if ui.add(egui::Button::new("Nuevo lienzo…").shortcut_text("Ctrl+N")).clicked() {
                    commands.push(Command::NewDialog);
                }
                if ui.add(egui::Button::new("Abrir…").shortcut_text("Ctrl+O")).clicked() {
                    commands.push(Command::Open);
                }
                ui.separator();
                if ui.add(egui::Button::new("Guardar").shortcut_text("Ctrl+S")).clicked() {
                    commands.push(Command::Save);
                }
                if ui.add(egui::Button::new("Guardar como…").shortcut_text("Ctrl+Mayús+S")).clicked() {
                    commands.push(Command::SaveAs);
                }
                if ui.add(egui::Button::new("Exportar PNG…").shortcut_text("Ctrl+E")).clicked() {
                    commands.push(Command::ExportPng);
                }
                ui.separator();
                if ui.button("Salir").clicked() {
                    commands.push(Command::Quit);
                }
            });
            ui.menu_button("Editar", |ui| {
                let undo = egui::Button::new("Deshacer").shortcut_text("Ctrl+Z");
                if ui.add_enabled(self.doc.can_undo(), undo).clicked() {
                    commands.push(Command::Undo);
                }
                let redo = egui::Button::new("Rehacer").shortcut_text("Ctrl+Mayús+Z");
                if ui.add_enabled(self.doc.can_redo(), redo).clicked() {
                    commands.push(Command::Redo);
                }
                ui.separator();
                if ui.button("Limpiar capa").clicked() {
                    commands.push(Command::ClearLayer);
                }
            });
            ui.menu_button("Ver", |ui| {
                let item = |ui: &mut egui::Ui, text: &str, key: &str| {
                    ui.add(egui::Button::new(text).shortcut_text(key)).clicked()
                };
                if item(ui, "Ajustar a la ventana", "Ctrl+0") {
                    commands.push(Command::Fit);
                }
                if item(ui, "Tamaño real (100 %)", "Ctrl+1") {
                    commands.push(Command::ActualSize);
                }
                if item(ui, "Acercar", "Ctrl++") {
                    commands.push(Command::Zoom(1.25));
                }
                if item(ui, "Alejar", "Ctrl+-") {
                    commands.push(Command::Zoom(0.8));
                }
                ui.separator();
                if item(ui, "Rotar a la izquierda", "4") {
                    commands.push(Command::Rotate(-15.0));
                }
                if item(ui, "Rotar a la derecha", "6") {
                    commands.push(Command::Rotate(15.0));
                }
                if item(ui, "Restablecer rotación", "5") {
                    commands.push(Command::ResetRotation);
                }
                if item(ui, "Voltear horizontalmente", "M") {
                    commands.push(Command::Flip);
                }
            });
            ui.menu_button("Ayuda", |ui| {
                ui.label("Atajos del lienzo");
                ui.separator();
                egui::Grid::new("atajos").num_columns(2).show(ui, |ui| {
                    for (keys, what) in [
                        ("B / E", "Pincel / Borrador"),
                        ("[ / ]", "Tamaño del pincel"),
                        ("Alt + clic", "Tomar color del lienzo"),
                        ("Espacio + arrastrar", "Mover el lienzo"),
                        ("Botón central", "Mover el lienzo"),
                        ("Rueda del ratón", "Zoom"),
                        ("R + arrastrar", "Rotar el lienzo"),
                    ] {
                        ui.label(keys);
                        ui.label(what);
                        ui.end_row();
                    }
                });
            });
        });
    }

    fn tool_panel(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        for (tool, label, tip) in [
            (Tool::Pincel, "Pincel", "Pincel (B)"),
            (Tool::Borrador, "Borrador", "Borrador (E)"),
        ] {
            let button = egui::Button::selectable(self.tool == tool, label)
                .min_size(Vec2::new(ui.available_width(), 32.0));
            if ui.add(button).on_hover_text(tip).clicked() {
                self.tool = tool;
            }
        }
    }

    fn settings_panel(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        ui.strong("Color");
        let mut color = Color32::from_rgb(self.color[0], self.color[1], self.color[2]);
        ui.scope(|ui| {
            // The picker takes its width from the slider width; make it fill the panel.
            ui.spacing_mut().slider_width = (ui.available_width() - 8.0).clamp(120.0, 260.0);
            if egui::color_picker::color_picker_color32(ui, &mut color, egui::color_picker::Alpha::Opaque) {
                self.color = [color.r(), color.g(), color.b()];
            }
        });

        ui.separator();
        match self.tool {
            Tool::Pincel => {
                ui.strong("Pinceles");
                ui.horizontal_wrapped(|ui| {
                    for preset in BrushPreset::ALL {
                        if ui.selectable_label(self.preset == Some(preset), preset.name()).clicked() {
                            self.brush = preset.settings();
                            self.preset = Some(preset);
                        }
                    }
                });
            }
            Tool::Borrador => {
                ui.strong("Borrador");
            }
        }

        ui.add_space(4.0);
        let before = self.active_brush().clone();
        let b = self.active_brush();
        let percent = |v: f64, _: std::ops::RangeInclusive<usize>| format!("{:.0} %", v * 100.0);
        egui::Grid::new("ajustes").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
            ui.label("Tamaño");
            ui.add(
                egui::Slider::new(&mut b.size, 1.0..=MAX_BRUSH_SIZE)
                    .logarithmic(true)
                    .fixed_decimals(0)
                    .suffix(" px"),
            );
            ui.end_row();
            for (label, value, max) in [
                ("Opacidad", &mut b.opacity, 1.0),
                ("Flujo", &mut b.flow, 1.0),
                ("Dureza", &mut b.hardness, 1.0),
                ("Espaciado", &mut b.spacing, 1.0),
                ("Estabilizador", &mut b.smoothing, 0.95),
            ] {
                ui.label(label);
                let min = if label == "Espaciado" || label == "Flujo" || label == "Opacidad" {
                    0.01
                } else {
                    0.0
                };
                ui.add(egui::Slider::new(value, min..=max).custom_formatter(percent));
                ui.end_row();
            }
        });
        ui.add_space(4.0);
        ui.strong("Presión del lápiz");
        ui.checkbox(&mut b.pressure_size, "Controla el tamaño");
        ui.add_enabled_ui(b.pressure_size, |ui| {
            ui.horizontal(|ui| {
                ui.label("Tamaño mínimo");
                ui.add(egui::Slider::new(&mut b.min_size, 0.0..=1.0).custom_formatter(percent));
            });
        });
        ui.checkbox(&mut b.pressure_flow, "Controla el flujo");
        if self.tool == Tool::Pincel && self.brush != before {
            // Hand-tuned settings no longer match the preset they started from.
            if self.preset.map(|p| p.settings()) != Some(self.brush.clone()) {
                self.preset = None;
            }
        }
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(format!("{} × {} px", self.doc.width, self.doc.height));
            ui.separator();
            ui.label(format!("Zoom {:.0} %", self.view.zoom * 100.0));
            ui.separator();
            ui.label(format!("Rotación {:.0}°", self.view.rotation.to_degrees()));
            ui.separator();
            if self.pen_seen {
                ui.label(format!("Presión {:.0} %", self.last_pressure * 100.0));
            } else {
                ui.label("Lápiz: sin detectar (usando ratón)");
            }
            if !self.status.is_empty() {
                ui.separator();
                ui.label(&self.status);
            }
        });
    }

    /// Uploads the parts of the document that changed since the last frame.
    fn sync_texture(&mut self, ctx: &egui::Context) {
        let options = TextureOptions {
            magnification: if self.view.zoom >= 3.0 { TextureFilter::Nearest } else { TextureFilter::Linear },
            minification: TextureFilter::Linear,
            ..Default::default()
        };
        let size = [self.doc.width, self.doc.height];
        let stale = self.texture.as_ref().is_none_or(|t| t.size() != size);
        if stale {
            self.doc.take_dirty();
            let image = ColorImage::from_rgba_unmultiplied(size, &self.doc.render_rgba8(self.doc.bounds()));
            self.texture = Some(ctx.load_texture("lienzo", image, options));
        } else if let (Some(texture), Some(dirty)) = (&mut self.texture, self.doc.take_dirty()) {
            let image = ColorImage::from_rgba_unmultiplied(
                [dirty.width(), dirty.height()],
                &self.doc.render_rgba8(dirty),
            );
            texture.set_partial([dirty.x0 as usize, dirty.y0 as usize], image, options);
        } else if let Some(texture) = &mut self.texture {
            if self.texture_options != options {
                // Filtering changes with zoom; re-upload a single pixel to apply it.
                let image = ColorImage::from_rgba_unmultiplied(
                    [1, 1],
                    &self.doc.render_rgba8(IRect::new(0, 0, 1, 1)),
                );
                texture.set_partial([0, 0], image, options);
            }
        }
        self.texture_options = options;
    }

    fn begin_stroke(&mut self, at: Pos2, pressure: f32) {
        let erase = self.tool == Tool::Borrador;
        let brush = self.active_brush().clone();
        self.doc.begin_stroke(&brush, self.color, erase, StrokePoint::new(at.x, at.y, pressure));
    }

    fn pick_color(&mut self, canvas: Pos2) {
        let (x, y) = (canvas.x.floor() as i32, canvas.y.floor() as i32);
        let rect = IRect::new(x, y, x + 1, y + 1);
        if rect.intersect(&self.doc.bounds()) == rect {
            let px = self.doc.render_rgba8(rect);
            self.color = [px[0], px[1], px[2]];
        }
    }

    fn canvas(&mut self, ui: &mut egui::Ui) {
        let (response, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
        let viewport = response.rect;
        self.viewport = viewport;
        let doc_size = self.doc_size();
        if self.fit_pending && viewport.width() > 1.0 {
            self.view.fit(viewport, doc_size);
            self.fit_pending = false;
        }

        let ctx = ui.ctx().clone();
        let (space, rotate_key, alt, events, hover) = ctx.input(|i| {
            (
                i.key_down(Key::Space),
                i.key_down(Key::R),
                i.modifiers.alt,
                i.events.clone(),
                i.pointer.hover_pos(),
            )
        });
        let typing = ctx.text_edit_focused();
        let navigating = !typing && (space || rotate_key);
        // The pointer is over the canvas itself, not a menu or window on top of it.
        let over_canvas = |pos: Pos2| {
            viewport.contains(pos) && ctx.layer_id_at(pos).is_none_or(|l| l == response.layer_id)
        };

        // --- Navigation -----------------------------------------------------
        if response.dragged_by(PointerButton::Middle) || (space && response.dragged_by(PointerButton::Primary)) {
            self.view.pan += response.drag_delta();
        } else if rotate_key && !typing && response.dragged_by(PointerButton::Primary) {
            if let Some(pos) = hover {
                let center = self.view.center(viewport);
                let now = pos - center;
                let before = now - response.drag_delta();
                if now.length() > 2.0 && before.length() > 2.0 {
                    self.view.rotation = (self.view.rotation + now.angle() - before.angle())
                        .rem_euclid(std::f32::consts::TAU);
                }
            }
        }
        if let Some(pos) = hover.filter(|p| over_canvas(*p)) {
            let (scroll, pinch) = ctx.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta()));
            let factor = (scroll * 0.0025).exp() * pinch;
            if factor != 1.0 {
                self.view.zoom_about(pos, factor, viewport, doc_size);
            }
        }

        // --- Painting -------------------------------------------------------
        // Every input sample of the frame is replayed in order so fast strokes
        // keep all their points, each with the pressure that arrived with it.
        let view = self.view;
        let to_canvas = |pos: Pos2| view.to_canvas(pos, viewport, doc_size);
        for event in &events {
            match event {
                Event::Touch { phase, force, .. } => match phase {
                    TouchPhase::Start | TouchPhase::Move => {
                        if *phase == TouchPhase::Start {
                            self.touch_active = true;
                            self.pen_pressure = None;
                        }
                        match force {
                            Some(f) => {
                                let f = f.clamp(0.0, 1.0);
                                self.pen_pressure = Some(f);
                                self.contact_pressure = Some(f);
                                self.pen_seen = true;
                            }
                            // Windows reports zero pressure as "no reading". Once the
                            // pen has reported pressure in this contact, that means 0.
                            None if self.pen_pressure.is_some() => self.pen_pressure = Some(0.0),
                            None => {}
                        }
                    }
                    TouchPhase::End | TouchPhase::Cancel => {
                        self.touch_active = false;
                        self.pen_pressure = None;
                    }
                },
                Event::PointerButton { pos, button: PointerButton::Primary, pressed: true, .. } => {
                    if navigating || !over_canvas(*pos) {
                        continue;
                    }
                    let c = to_canvas(*pos);
                    if alt {
                        self.pick_color(c);
                        continue;
                    }
                    // The first dab waits for a real pressure reading: a pen touching
                    // down reports no pressure at first, and assuming full pressure
                    // there stamped a full-size dot at the start of every stroke.
                    self.doc.end_stroke();
                    self.pending_start = Some(c);
                    self.press_is_touch = self.touch_active;
                    self.contact_pressure = self.pen_pressure;
                }
                Event::PointerMoved(pos) if self.pending_start.is_some() || self.doc.is_stroking() => {
                    let c = to_canvas(*pos);
                    let pressure = match (self.press_is_touch, self.pen_pressure) {
                        (_, Some(p)) => p,
                        (false, None) => 1.0, // mouse
                        (true, None) => {
                            // Pen is down but has not reported pressure yet: no ink.
                            if self.pending_start.is_some() {
                                self.pending_start = Some(c);
                            }
                            continue;
                        }
                    };
                    self.last_pressure = pressure;
                    if let Some(start) = self.pending_start.take() {
                        self.begin_stroke(start, pressure);
                    }
                    self.doc.stroke_to(StrokePoint::new(c.x, c.y, pressure));
                }
                Event::PointerButton { button: PointerButton::Primary, pressed: false, .. }
                | Event::PointerGone => {
                    // A tap without movement still leaves one dab.
                    if let Some(start) = self.pending_start.take() {
                        let fallback = if self.press_is_touch { 0.0 } else { 1.0 };
                        let pressure = self.contact_pressure.unwrap_or(fallback);
                        self.last_pressure = pressure;
                        self.begin_stroke(start, pressure);
                    }
                    self.doc.end_stroke();
                }
                _ => {}
            }
        }

        // --- Drawing --------------------------------------------------------
        self.sync_texture(&ctx);
        painter.rect_filled(viewport, 0.0, Color32::from_gray(38));
        let corners = [
            Pos2::ZERO,
            Pos2::new(doc_size.x, 0.0),
            doc_size.to_pos2(),
            Pos2::new(0.0, doc_size.y),
        ]
        .map(|c| self.view.to_screen(c, viewport, doc_size));
        if let Some(texture) = &self.texture {
            let mut mesh = Mesh::with_texture(texture.id());
            for (corner, uv) in corners.iter().zip([(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)]) {
                mesh.vertices.push(egui::epaint::Vertex {
                    pos: *corner,
                    uv: Pos2::new(uv.0, uv.1),
                    color: Color32::WHITE,
                });
            }
            mesh.indices.extend([0, 1, 2, 0, 2, 3]);
            painter.add(Shape::mesh(mesh));
        }
        painter.add(Shape::closed_line(corners.to_vec(), Stroke::new(1.0, Color32::from_gray(15))));

        // Brush outline in place of the system cursor.
        if let Some(pos) = hover.filter(|p| over_canvas(*p)) {
            if navigating {
                ctx.set_cursor_icon(if space { CursorIcon::Grab } else { CursorIcon::Alias });
            } else if alt {
                ctx.set_cursor_icon(CursorIcon::Crosshair);
            } else {
                let radius = self.active_brush().size * 0.5 * self.view.zoom;
                if radius >= 4.0 {
                    ctx.set_cursor_icon(CursorIcon::None);
                    painter.circle_stroke(pos, radius, Stroke::new(2.0, Color32::from_black_alpha(160)));
                    painter.circle_stroke(pos, radius, Stroke::new(1.0, Color32::WHITE));
                } else {
                    ctx.set_cursor_icon(CursorIcon::Crosshair);
                }
            }
        }
    }

    fn new_canvas_window(&mut self, ctx: &egui::Context) {
        let Some(dialog) = &mut self.new_dialog else { return };
        let max_side = ctx.input(|i| i.max_texture_side).max(256);
        let mut create = false;
        let mut cancel = false;
        egui::Window::new("Nuevo lienzo")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                egui::Grid::new("nuevo").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
                    ui.label("Ancho");
                    ui.add(egui::DragValue::new(&mut dialog.width).range(16..=max_side).suffix(" px"));
                    ui.end_row();
                    ui.label("Alto");
                    ui.add(egui::DragValue::new(&mut dialog.height).range(16..=max_side).suffix(" px"));
                    ui.end_row();
                });
                ui.horizontal_wrapped(|ui| {
                    for (name, w, h) in [
                        ("Cuadrado 2048", 2048, 2048),
                        ("Full HD", 1920, 1080),
                        ("4K", 3840, 2160),
                        ("A4 300 ppp", 2480, 3508),
                    ] {
                        if ui.small_button(name).clicked() {
                            dialog.width = w.min(max_side);
                            dialog.height = h.min(max_side);
                        }
                    }
                });
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    create = ui.button("Crear").clicked();
                    cancel = ui.button("Cancelar").clicked();
                });
            });
        if create {
            let doc = Document::new(dialog.width, dialog.height);
            self.replace_document(doc, None);
            self.status.clear();
        }
        if create || cancel {
            self.new_dialog = None;
        }
    }

    fn export_png(&mut self) {
        self.doc.end_stroke();
        let Some(mut path) = rfd::FileDialog::new()
            .set_title("Exportar PNG")
            .add_filter("Imagen PNG", &["png"])
            .set_file_name("dibujo.png")
            .save_file()
        else {
            return;
        };
        if path.extension().is_none() {
            path.set_extension("png");
        }
        let rgba = self.doc.render_rgba8(self.doc.bounds());
        self.status = match write_png(&path, self.doc.width as u32, self.doc.height as u32, &rgba) {
            Ok(()) => format!("Exportado: {}", path.display()),
            Err(e) => format!("No se pudo exportar: {e}"),
        };
    }

    fn run(&mut self, command: Command, ctx: &egui::Context) {
        let viewport = self.viewport;
        let doc_size = self.doc_size();
        match command {
            Command::NewDialog => self.discarding(Discarding::New, ctx),
            Command::Open => self.discarding(Discarding::Open, ctx),
            Command::Save => {
                self.save_project(false);
            }
            Command::SaveAs => {
                self.save_project(true);
            }
            Command::ExportPng => self.export_png(),
            Command::Undo => {
                self.doc.undo();
            }
            Command::Redo => {
                self.doc.redo();
            }
            Command::ClearLayer => self.doc.clear_active_layer(),
            Command::Fit => self.fit_pending = true,
            Command::ActualSize => {
                self.view.zoom = 1.0;
                self.view.pan = Vec2::ZERO;
            }
            Command::Zoom(factor) => self.view.zoom_about(viewport.center(), factor, viewport, doc_size),
            Command::Rotate(degrees) => self.view.rotate_by(degrees.to_radians(), viewport, doc_size),
            Command::ResetRotation => self.view.rotate_by(-self.view.rotation, viewport, doc_size),
            Command::Flip => self.view.flip = !self.view.flip,
            Command::Quit => self.discarding(Discarding::Quit, ctx),
        }
    }
}

impl eframe::App for SharkPaintApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let mut commands = Vec::new();
        self.keyboard_commands(&ctx, &mut commands);

        egui::Panel::top("menu").show(ui, |ui| self.menu_bar(ui, &mut commands));
        egui::Panel::bottom("estado").show(ui, |ui| self.status_bar(ui));
        egui::Panel::left("herramientas")
            .resizable(false)
            .default_size(96.0)
            .show(ui, |ui| self.tool_panel(ui));
        egui::Panel::right("ajustes")
            .resizable(false)
            .default_size(260.0)
            .show(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| self.settings_panel(ui));
            });
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| self.canvas(ui));

        // Closing the window with unsaved changes asks first.
        if ctx.input(|i| i.viewport().close_requested()) && !self.allow_close && self.modified() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.confirm = Some(Discarding::Quit);
        }
        self.new_canvas_window(&ctx);
        self.confirm_window(&ctx);
        self.update_window_title(&ctx);
        for command in commands {
            self.run(command, &ctx);
        }
    }
}
