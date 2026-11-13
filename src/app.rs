//! Desktop interface: menus, tool and brush panels, and the canvas viewport.

use eframe::egui::{
    self, Color32, ColorImage, CursorIcon, Event, Key, Mesh, PointerButton, Pos2, Rect, Sense,
    Shape, Stroke, TextureFilter, TextureHandle, TextureOptions, TouchPhase, Vec2,
};

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use sharkpaint::engine::{
    write_png, BlendMode, BrushPreset, BrushSettings, Document, IRect, StrokePoint,
    PROJECT_EXTENSION,
};

use crate::icons::icon;
use crate::view::View;

const DEFAULT_SIZE: [usize; 2] = [3000, 2000];
const MAX_BRUSH_SIZE: f32 = 500.0;
/// Slot for a layer preview in the layer list, in points.
const THUMB_SIZE: (usize, usize) = (56, 38);
const ROW_HEIGHT: f32 = 46.0;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tool {
    Pincel,
    Borrador,
    Difuminar,
    Relleno,
    Cuentagotas,
    Mover,
}

impl Tool {
    /// (tool, icon, name, shortcut letter) in tool bar order.
    const ALL: [(Tool, &'static str, &'static str, &'static str); 6] = [
        (Tool::Pincel, "pincel", "Pincel", "B"),
        (Tool::Borrador, "borrador", "Borrador", "E"),
        (Tool::Difuminar, "difuminar", "Difuminar", "D"),
        (Tool::Relleno, "relleno", "Relleno", "G"),
        (Tool::Cuentagotas, "cuentagotas", "Cuentagotas", "I"),
        (Tool::Mover, "mover", "Mover capa", "V"),
    ];

    fn name(self) -> &'static str {
        Tool::ALL.iter().find(|t| t.0 == self).map_or("", |t| t.2)
    }

    /// The tool paints with a round tip whose size the cursor shows.
    fn has_tip(self) -> bool {
        matches!(self, Tool::Pincel | Tool::Borrador | Tool::Difuminar)
    }
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
    LayerAdd,
    LayerDuplicate,
    LayerDelete,
    LayerMove(bool),
    LayerMerge,
    Fit,
    ActualSize,
    Zoom(f32),
    Rotate(f32),
    ResetRotation,
    Flip,
    ToggleBrushWindow,
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
    /// Layer previews by layer id, with the layer version they were made from.
    thumbnails: HashMap<u64, (u64, TextureHandle)>,
    /// Screen area of the canvas viewport on the last frame.
    viewport: Rect,
    tool: Tool,
    brush: BrushSettings,
    eraser: BrushSettings,
    smudge: BrushSettings,
    /// How different a pixel may be and still be filled (0..=1).
    fill_tolerance: f32,
    /// The fill looks at the whole picture rather than only the active layer.
    fill_all_layers: bool,
    /// Canvas position where a layer move began.
    move_origin: Option<Pos2>,
    /// The eyedropper is being dragged.
    picking: bool,
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
    /// The pen or first finger is currently touching the tablet.
    touch_active: bool,
    /// Every pen or finger contact currently down.
    touches: HashSet<u64>,
    /// The contact that started first; only it drives painting and pressure.
    primary_touch: Option<u64>,
    /// Two fingers are moving, zooming or rotating the canvas.
    gesture_active: bool,
    /// When the current press began, to tell a second finger from a resting palm.
    press_time: f64,
    /// The layer property widgets are mid-edit (slider drag or typing), so
    /// further changes join the same undo step.
    prop_edit_active: bool,
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
    /// The floating brush settings window is showing.
    brush_window_open: bool,
}

impl SharkPaintApp {
    pub fn new(cc: &eframe::CreationContext<'_>, open_path: Option<PathBuf>) -> Self {
        cc.egui_ctx.set_theme(egui::Theme::Dark);
        egui_extras::install_image_loaders(&cc.egui_ctx);
        // Ctrl +/- must zoom the canvas, not scale the whole interface.
        cc.egui_ctx.options_mut(|o| o.zoom_with_keyboard = false);
        let mut eraser = BrushPreset::Marcador.settings();
        eraser.opacity = 1.0;
        let mut app = SharkPaintApp {
            doc: Document::new(DEFAULT_SIZE[0], DEFAULT_SIZE[1]),
            view: View::default(),
            texture: None,
            texture_options: TextureOptions::default(),
            thumbnails: HashMap::new(),
            viewport: Rect::ZERO,
            tool: Tool::Pincel,
            brush: BrushPreset::Tinta.settings(),
            eraser,
            smudge: BrushSettings {
                size: 60.0,
                opacity: 1.0,
                flow: 0.7,
                hardness: 0.3,
                spacing: 0.1,
                smoothing: 0.0,
                pressure_size: false,
                pressure_flow: true,
                min_size: 1.0,
            },
            fill_tolerance: 0.15,
            fill_all_layers: true,
            move_origin: None,
            picking: false,
            preset: Some(BrushPreset::Tinta),
            color: [24, 24, 32],
            pen_pressure: None,
            last_pressure: 1.0,
            pending_start: None,
            contact_pressure: None,
            touch_active: false,
            touches: HashSet::new(),
            primary_touch: None,
            gesture_active: false,
            press_time: 0.0,
            prop_edit_active: false,
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
            brush_window_open: true,
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
            Tool::Borrador => &mut self.eraser,
            Tool::Difuminar => &mut self.smudge,
            _ => &mut self.brush,
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
            if hit(cmd | egui::Modifiers::SHIFT, Key::N) {
                commands.push(Command::LayerAdd);
            }
            if hit(cmd, Key::N) {
                commands.push(Command::NewDialog);
            }
            if hit(cmd, Key::J) {
                commands.push(Command::LayerDuplicate);
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
            if hit(none, Key::F5) {
                commands.push(Command::ToggleBrushWindow);
            }
            for (tool, key) in [
                (Tool::Pincel, Key::B),
                (Tool::Borrador, Key::E),
                (Tool::Difuminar, Key::D),
                (Tool::Relleno, Key::G),
                (Tool::Cuentagotas, Key::I),
                (Tool::Mover, Key::V),
            ] {
                if hit(none, key) {
                    self.tool = tool;
                }
            }
            let brush = match self.tool {
                Tool::Borrador => &mut self.eraser,
                Tool::Difuminar => &mut self.smudge,
                _ => &mut self.brush,
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
            });
            ui.menu_button("Capa", |ui| {
                let many = self.doc.layers().len() > 1;
                let active = self.doc.active_layer();
                if ui.add(egui::Button::new("Nueva capa").shortcut_text("Ctrl+Mayús+N")).clicked() {
                    commands.push(Command::LayerAdd);
                }
                if ui.add(egui::Button::new("Duplicar capa").shortcut_text("Ctrl+J")).clicked() {
                    commands.push(Command::LayerDuplicate);
                }
                if ui.add_enabled(active > 0, egui::Button::new("Fusionar con la inferior")).clicked() {
                    commands.push(Command::LayerMerge);
                }
                ui.separator();
                if ui.add_enabled(active + 1 < self.doc.layers().len(), egui::Button::new("Subir")).clicked() {
                    commands.push(Command::LayerMove(true));
                }
                if ui.add_enabled(active > 0, egui::Button::new("Bajar")).clicked() {
                    commands.push(Command::LayerMove(false));
                }
                ui.separator();
                if ui.button("Limpiar capa").clicked() {
                    commands.push(Command::ClearLayer);
                }
                if ui.add_enabled(many, egui::Button::new("Eliminar capa")).clicked() {
                    commands.push(Command::LayerDelete);
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
                ui.separator();
                if item(ui, "Ajustes del pincel", "F5") {
                    commands.push(Command::ToggleBrushWindow);
                }
            });
            ui.menu_button("Ayuda", |ui| {
                ui.strong(concat!("SharkPaint ", env!("CARGO_PKG_VERSION")));
                ui.label("Software libre bajo la licencia GPL 3.0");
                ui.separator();
                ui.label("Atajos del lienzo");
                ui.separator();
                egui::Grid::new("atajos").num_columns(2).show(ui, |ui| {
                    for (keys, what) in [
                        ("B / E / D", "Pincel / Borrador / Difuminar"),
                        ("G / I / V", "Relleno / Cuentagotas / Mover capa"),
                        ("[ / ]", "Tamaño del pincel"),
                        ("F5", "Mostrar u ocultar los ajustes del pincel"),
                        ("Alt + clic", "Tomar color del lienzo"),
                        ("R + arrastrar", "Mover el lienzo"),
                        ("Espacio + arrastrar", "Mover el lienzo"),
                        ("Botón central", "Mover el lienzo"),
                        ("Rueda del ratón", "Zoom"),
                        ("Mayús + R + arrastrar", "Rotar el lienzo"),
                        ("Dos dedos", "Mover, rotar y hacer zoom"),
                    ] {
                        ui.label(keys);
                        ui.label(what);
                        ui.end_row();
                    }
                });
            });
        });
    }

    /// The narrow tool bar: one icon per tool. Clicking the tool that is already
    /// selected shows or hides its settings window.
    fn tool_bar(&mut self, ui: &mut egui::Ui, commands: &mut Vec<Command>) {
        ui.add_space(4.0);
        let text = ui.visuals().strong_text_color();
        let size = Vec2::splat(ui.available_width());
        for (tool, name, label, key) in Tool::ALL {
            let tip = format!("{label} ({key})\nClic de nuevo: ajustes");
            let button = egui::Button::image(icon(name, 20.0, text)).selected(self.tool == tool).min_size(size);
            if ui.add(button).on_hover_text(tip).clicked() {
                if self.tool == tool {
                    commands.push(Command::ToggleBrushWindow);
                } else {
                    self.tool = tool;
                }
            }
        }
        ui.separator();
        let button = egui::Button::image(icon("ajustes", 20.0, text))
            .selected(self.brush_window_open)
            .min_size(size);
        if ui.add(button).on_hover_text("Ajustes del pincel (F5)").clicked() {
            commands.push(Command::ToggleBrushWindow);
        }
        ui.separator();
        let faint = ui.visuals().weak_text_color().gamma_multiply(0.5);
        for (name, enabled, tip, command) in [
            ("deshacer", self.doc.can_undo(), "Deshacer (Ctrl+Z)", Command::Undo),
            ("rehacer", self.doc.can_redo(), "Rehacer (Ctrl+Mayús+Z)", Command::Redo),
        ] {
            let color = if enabled { text } else { faint };
            let button = egui::Button::image(icon(name, 20.0, color)).min_size(size);
            if ui.add_enabled(enabled, button).on_hover_text(tip).clicked() {
                commands.push(command);
            }
        }
    }

    /// Floating, movable window with the settings of the current tool.
    fn brush_window(&mut self, ctx: &egui::Context) {
        let mut open = self.brush_window_open;
        egui::Window::new(self.tool.name())
            .id(egui::Id::unique("ajustes_herramienta"))
            .open(&mut open)
            .collapsible(true)
            .resizable(false)
            .default_pos(self.viewport.left_top() + Vec2::new(12.0, 12.0))
            .default_width(250.0)
            .show(ctx, |ui| self.brush_panel(ui));
        self.brush_window_open = open;
    }

    fn color_panel(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        ui.strong("Color");
        let mut color = Color32::from_rgb(self.color[0], self.color[1], self.color[2]);
        ui.scope(|ui| {
            // The picker takes its width from the slider width; make it fill the panel.
            ui.spacing_mut().slider_width = (ui.available_width() - 8.0).clamp(120.0, 230.0);
            if egui::color_picker::color_picker_color32(ui, &mut color, egui::color_picker::Alpha::Opaque) {
                self.color = [color.r(), color.g(), color.b()];
            }
        });
    }

    fn brush_panel(&mut self, ui: &mut egui::Ui) {
        let percent = |v: f64, _: std::ops::RangeInclusive<usize>| format!("{:.0} %", v * 100.0);
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
            Tool::Borrador | Tool::Difuminar => {}
            Tool::Relleno => {
                ui.horizontal(|ui| {
                    ui.label("Tolerancia");
                    ui.add(egui::Slider::new(&mut self.fill_tolerance, 0.0..=1.0).custom_formatter(percent));
                });
                ui.checkbox(&mut self.fill_all_layers, "Tener en cuenta todas las capas")
                    .on_hover_text("El relleno se detiene en las líneas de cualquier capa visible,\nno solo en las de la capa activa");
                ui.label("Rellena la capa activa con el color elegido.");
                return;
            }
            Tool::Cuentagotas => {
                ui.label("Haz clic o arrastra sobre el lienzo para tomar un color.");
                ui.label("Con cualquier otra herramienta: Alt + clic.");
                return;
            }
            Tool::Mover => {
                ui.label("Arrastra sobre el lienzo para mover el contenido de la capa activa.");
                return;
            }
        }

        ui.add_space(4.0);
        let before = self.active_brush().clone();
        let smudging = self.tool == Tool::Difuminar;
        let b = self.active_brush();
        egui::Grid::new("ajustes").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
            ui.label("Tamaño");
            ui.add(
                egui::Slider::new(&mut b.size, 1.0..=MAX_BRUSH_SIZE)
                    .logarithmic(true)
                    .fixed_decimals(0)
                    .suffix(" px"),
            );
            ui.end_row();
            let rows: Vec<(&str, &mut f32, f32, f32)> = if smudging {
                vec![("Intensidad", &mut b.flow, 0.01, 1.0), ("Dureza", &mut b.hardness, 0.0, 1.0)]
            } else {
                vec![
                    ("Opacidad", &mut b.opacity, 0.01, 1.0),
                    ("Flujo", &mut b.flow, 0.01, 1.0),
                    ("Dureza", &mut b.hardness, 0.0, 1.0),
                    ("Espaciado", &mut b.spacing, 0.01, 1.0),
                    ("Estabilizador", &mut b.smoothing, 0.0, 0.95),
                ]
            };
            for (label, value, min, max) in rows {
                ui.label(label);
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
        ui.checkbox(&mut b.pressure_flow, if smudging { "Controla la intensidad" } else { "Controla el flujo" });
        if self.tool == Tool::Pincel && self.brush != before {
            // Hand-tuned settings no longer match the preset they started from.
            if self.preset.map(|p| p.settings()) != Some(self.brush.clone()) {
                self.preset = None;
            }
        }
    }

    /// Keeps one preview texture per layer, redrawn when the layer's pixels change.
    fn sync_thumbnails(&mut self, ctx: &egui::Context) {
        let paper = self.doc.paper;
        for layer in self.doc.layers() {
            if self.thumbnails.get(&layer.id).is_some_and(|(version, _)| *version == layer.version) {
                continue;
            }
            let (w, h, rgba) = layer.thumbnail(THUMB_SIZE.0 * 2, THUMB_SIZE.1 * 2, paper);
            let image = ColorImage::from_rgba_unmultiplied([w, h], &rgba);
            let texture = ctx.load_texture(format!("capa-{}", layer.id), image, TextureOptions::LINEAR);
            self.thumbnails.insert(layer.id, (layer.version, texture));
        }
        let alive: HashSet<u64> = self.doc.layers().iter().map(|l| l.id).collect();
        self.thumbnails.retain(|id, _| alive.contains(id));
    }

    fn layers_panel(&mut self, ui: &mut egui::Ui, commands: &mut Vec<Command>) {
        self.sync_thumbnails(ui.ctx());
        ui.strong("Capas");
        let count = self.doc.layers().len();
        let active = self.doc.active_layer();
        let text = ui.visuals().strong_text_color();
        let faint = ui.visuals().weak_text_color();
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            let tool = |ui: &mut egui::Ui, name: &str, enabled: bool, tip: &str| {
                let color = if enabled { text } else { faint.gamma_multiply(0.5) };
                let button = egui::Button::image(icon(name, 18.0, color)).min_size(Vec2::splat(28.0));
                ui.add_enabled(enabled, button).on_hover_text(tip).on_disabled_hover_text(tip).clicked()
            };
            if tool(ui, "mas", true, "Nueva capa (Ctrl+Mayús+N)") {
                commands.push(Command::LayerAdd);
            }
            if tool(ui, "duplicar", true, "Duplicar capa (Ctrl+J)") {
                commands.push(Command::LayerDuplicate);
            }
            if tool(ui, "subir", active + 1 < count, "Subir capa") {
                commands.push(Command::LayerMove(true));
            }
            if tool(ui, "bajar", active > 0, "Bajar capa") {
                commands.push(Command::LayerMove(false));
            }
            if tool(ui, "fusionar", active > 0, "Fusionar con la capa inferior") {
                commands.push(Command::LayerMerge);
            }
            if tool(ui, "quitar", count > 1, "Eliminar capa") {
                commands.push(Command::LayerDelete);
            }
        });

        // Properties of the active layer.
        let before = self.doc.active().props.clone();
        let mut props = before.clone();
        let mut continuous = false;
        let percent = |v: f64, _: std::ops::RangeInclusive<usize>| format!("{:.0} %", v * 100.0);
        egui::Grid::new("capa_activa").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
            ui.label("Nombre");
            let name = ui.add(egui::TextEdit::singleline(&mut props.name).desired_width(150.0));
            continuous |= name.has_focus();
            ui.end_row();
            ui.label("Modo");
            egui::ComboBox::from_id_salt("modo_fusion")
                .selected_text(props.blend.name())
                .show_ui(ui, |ui| {
                    for mode in BlendMode::ALL {
                        ui.selectable_value(&mut props.blend, mode, mode.name());
                    }
                });
            ui.end_row();
            ui.label("Opacidad");
            let slider = ui.add(egui::Slider::new(&mut props.opacity, 0.0..=1.0).custom_formatter(percent));
            continuous |= slider.dragged();
            ui.end_row();
        });
        ui.checkbox(&mut props.alpha_lock, "Bloquear transparencia")
            .on_hover_text("Pinta solo sobre lo que ya está dibujado en la capa");
        if props != before {
            self.doc.set_layer_props(active, props, self.prop_edit_active);
            self.prop_edit_active = continuous;
        } else if !continuous {
            self.prop_edit_active = false;
        }

        ui.add_space(4.0);
        // The stack, top layer first: visibility, preview, name.
        let mut select = None;
        let mut toggle = None;
        egui::ScrollArea::vertical().id_salt("lista_capas").auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            for (i, layer) in self.doc.layers().iter().enumerate().rev() {
                let props = &layer.props;
                let ink = if props.visible { text } else { faint };
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    let (eye, tip) = if props.visible {
                        ("ojo", "Ocultar capa")
                    } else {
                        ("ojoCerrado", "Mostrar capa")
                    };
                    let button = egui::Button::image(icon(eye, 16.0, ink))
                        .frame(false)
                        .min_size(Vec2::new(26.0, ROW_HEIGHT));
                    if ui.add(button).on_hover_text(tip).clicked() {
                        toggle = Some(i);
                    }

                    let size = Vec2::new(ui.available_width(), ROW_HEIGHT);
                    let (row, response) = ui.allocate_exact_size(size, Sense::click());
                    let painter = ui.painter().with_clip_rect(row);
                    let selected = i == active;
                    if selected {
                        painter.rect_filled(row, 4.0, ui.visuals().selection.bg_fill);
                    } else if response.hovered() {
                        painter.rect_filled(row, 4.0, ui.visuals().widgets.hovered.weak_bg_fill);
                    }

                    // Preview, fitted inside a fixed slot so every row lines up.
                    let slot = Rect::from_min_size(
                        row.min + Vec2::new(4.0, (ROW_HEIGHT - THUMB_SIZE.1 as f32) * 0.5),
                        Vec2::new(THUMB_SIZE.0 as f32, THUMB_SIZE.1 as f32),
                    );
                    if let Some((_, texture)) = self.thumbnails.get(&layer.id) {
                        let [w, h] = texture.size();
                        let fit = (slot.width() / w as f32).min(slot.height() / h as f32);
                        let thumb = Rect::from_center_size(slot.center(), Vec2::new(w as f32, h as f32) * fit);
                        let tint = if props.visible { Color32::WHITE } else { Color32::from_white_alpha(90) };
                        let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
                        painter.image(texture.id(), thumb, uv, tint);
                        let border = if selected { Color32::WHITE } else { Color32::from_gray(90) };
                        painter.rect_stroke(thumb, 1.0, Stroke::new(1.0, border), egui::StrokeKind::Outside);
                    }

                    let ink = if selected { ui.visuals().selection.stroke.color } else { ink };
                    let left = slot.right() + 8.0;
                    let detail = (props.blend != BlendMode::Normal || props.opacity < 1.0)
                        .then(|| format!("{} · {:.0} %", props.blend.name(), props.opacity * 100.0));
                    let name_y = if detail.is_some() { row.center().y - 8.0 } else { row.center().y };
                    painter.text(
                        Pos2::new(left, name_y),
                        egui::Align2::LEFT_CENTER,
                        &props.name,
                        egui::FontId::proportional(13.0),
                        ink,
                    );
                    if let Some(detail) = detail {
                        painter.text(
                            Pos2::new(left, row.center().y + 9.0),
                            egui::Align2::LEFT_CENTER,
                            detail,
                            egui::FontId::proportional(11.0),
                            ink.gamma_multiply(0.7),
                        );
                    }
                    if props.alpha_lock {
                        let at = Rect::from_center_size(
                            Pos2::new(row.right() - 14.0, row.center().y),
                            Vec2::splat(14.0),
                        );
                        icon("candado", 14.0, ink).paint_at(ui, at);
                    }
                    if response.clicked() {
                        select = Some(i);
                    }
                });
            }
        });
        if let Some(i) = toggle {
            let mut props = self.doc.layers()[i].props.clone();
            props.visible = !props.visible;
            self.doc.set_layer_props(i, props, false);
        }
        if let Some(i) = select {
            self.doc.set_active_layer(i);
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
        let brush = self.active_brush().clone();
        let point = StrokePoint::new(at.x, at.y, pressure);
        match self.tool {
            Tool::Difuminar => self.doc.begin_smudge(&brush, point),
            tool => self.doc.begin_stroke(&brush, self.color, tool == Tool::Borrador, point),
        }
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
        let (space, nav_key, shift, alt, events, hover, now) = ctx.input(|i| {
            (
                i.key_down(Key::Space),
                i.key_down(Key::R),
                i.modifiers.shift,
                i.modifiers.alt,
                i.events.clone(),
                i.pointer.hover_pos(),
                i.time,
            )
        });
        let typing = ctx.text_edit_focused();
        let navigating = !typing && (space || nav_key);
        let rotating = nav_key && shift;
        // The pointer is over the canvas itself, not a menu or window on top of it.
        let over_canvas = |pos: Pos2| {
            viewport.contains(pos) && ctx.layer_id_at(pos).is_none_or(|l| l == response.layer_id)
        };

        // --- Navigation -----------------------------------------------------
        let primary_drag = navigating && !self.gesture_active && response.dragged_by(PointerButton::Primary);
        if response.dragged_by(PointerButton::Middle) || (primary_drag && !rotating) {
            self.view.pan += response.drag_delta();
        } else if primary_drag {
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
        let fingers = ctx.multi_touch();
        if let Some(pos) = hover.filter(|p| over_canvas(*p)) {
            // With two fingers down the pinch is applied by the gesture below.
            let (scroll, pinch) = ctx.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta()));
            let factor = (scroll * 0.0025).exp() * if fingers.is_some() { 1.0 } else { pinch };
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
                Event::Touch { id, phase, force, .. } => {
                    let id = id.0;
                    match phase {
                        TouchPhase::Start => {
                            if self.touches.is_empty() {
                                self.primary_touch = Some(id);
                                self.touch_active = true;
                                self.pen_pressure = None;
                            } else if !self.gesture_active {
                                // A second contact right after the first is a two-finger
                                // gesture: drop whatever the first one started to draw.
                                // Later than that it is a palm resting while the pen draws.
                                let drawing = self.pending_start.is_some() || self.doc.is_stroking();
                                if !drawing || now - self.press_time < 0.25 {
                                    self.gesture_active = true;
                                    self.pending_start = None;
                                    self.move_origin = None;
                                    self.picking = false;
                                    self.doc.cancel_stroke();
                                }
                            }
                            self.touches.insert(id);
                        }
                        TouchPhase::Move => {}
                        TouchPhase::End | TouchPhase::Cancel => {
                            self.touches.remove(&id);
                            if self.primary_touch == Some(id) {
                                self.primary_touch = None;
                                self.touch_active = false;
                                self.pen_pressure = None;
                            }
                            if self.touches.is_empty() {
                                self.gesture_active = false;
                            }
                        }
                    }
                    if self.primary_touch == Some(id) {
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
                }
                Event::PointerButton { pos, button: PointerButton::Primary, pressed: true, .. } => {
                    if navigating || self.gesture_active || !over_canvas(*pos) {
                        continue;
                    }
                    let c = to_canvas(*pos);
                    if alt || self.tool == Tool::Cuentagotas {
                        self.pick_color(c);
                        self.picking = self.tool == Tool::Cuentagotas;
                        continue;
                    }
                    if !self.doc.active().props.visible {
                        self.status = "La capa activa está oculta: muéstrala para pintar en ella.".into();
                        continue;
                    }
                    match self.tool {
                        Tool::Relleno => {
                            let (x, y) = (c.x.floor() as i32, c.y.floor() as i32);
                            self.doc.flood_fill(x, y, self.color, self.fill_tolerance, self.fill_all_layers);
                            continue;
                        }
                        Tool::Mover => {
                            self.doc.begin_layer_move();
                            self.move_origin = Some(c);
                            continue;
                        }
                        _ => {}
                    }
                    // The first dab waits for a real pressure reading: a pen touching
                    // down reports no pressure at first, and assuming full pressure
                    // there stamped a full-size dot at the start of every stroke.
                    self.doc.end_stroke();
                    self.pending_start = Some(c);
                    self.press_is_touch = self.touch_active;
                    self.press_time = now;
                    self.contact_pressure = self.pen_pressure;
                }
                Event::PointerMoved(pos) if self.picking => self.pick_color(to_canvas(*pos)),
                Event::PointerMoved(pos) if self.move_origin.is_some() => {
                    let delta = to_canvas(*pos) - self.move_origin.unwrap_or_default();
                    self.doc.move_layer_to(delta.x.round() as i32, delta.y.round() as i32);
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
                    self.picking = false;
                    self.move_origin = None;
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

        // Two fingers move, zoom and rotate the canvas around the point between them.
        if let Some(touch) = fingers.filter(|_| self.gesture_active) {
            self.view.pan += touch.translation_delta;
            self.view.zoom_about(touch.center_pos, touch.zoom_delta, viewport, doc_size);
            self.view.rotate_about(touch.center_pos, touch.rotation_delta, viewport, doc_size);
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
                ctx.set_cursor_icon(if rotating { CursorIcon::Alias } else { CursorIcon::Grab });
            } else if alt || matches!(self.tool, Tool::Cuentagotas | Tool::Relleno) {
                ctx.set_cursor_icon(CursorIcon::Crosshair);
            } else if self.tool == Tool::Mover {
                ctx.set_cursor_icon(CursorIcon::Move);
            } else {
                debug_assert!(self.tool.has_tip());
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
            Command::LayerAdd => self.doc.add_layer(),
            Command::LayerDuplicate => self.doc.duplicate_layer(),
            Command::LayerDelete => {
                self.doc.delete_layer();
            }
            Command::LayerMove(up) => {
                self.doc.move_layer(up);
            }
            Command::LayerMerge => {
                self.doc.merge_down();
            }
            Command::Fit => self.fit_pending = true,
            Command::ActualSize => {
                self.view.zoom = 1.0;
                self.view.pan = Vec2::ZERO;
            }
            Command::Zoom(factor) => self.view.zoom_about(viewport.center(), factor, viewport, doc_size),
            Command::Rotate(degrees) => {
                self.view.rotate_about(viewport.center(), degrees.to_radians(), viewport, doc_size)
            }
            Command::ResetRotation => {
                self.view.rotate_about(viewport.center(), -self.view.rotation, viewport, doc_size)
            }
            Command::Flip => self.view.flip = !self.view.flip,
            Command::ToggleBrushWindow => self.brush_window_open = !self.brush_window_open,
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
        // Left: a slim bar of tool icons. Right: colour and layers.
        egui::Panel::left("herramientas")
            .resizable(false)
            .exact_size(52.0)
            .show(ui, |ui| self.tool_bar(ui, &mut commands));
        egui::Panel::right("ajustes")
            .resizable(false)
            .default_size(260.0)
            .show(ui, |ui| {
                self.color_panel(ui);
                ui.separator();
                self.layers_panel(ui, &mut commands);
            });
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| self.canvas(ui));

        // Closing the window with unsaved changes asks first.
        if ctx.input(|i| i.viewport().close_requested()) && !self.allow_close && self.modified() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.confirm = Some(Discarding::Quit);
        }
        if self.brush_window_open {
            self.brush_window(&ctx);
        }
        self.new_canvas_window(&ctx);
        self.confirm_window(&ctx);
        self.update_window_title(&ctx);
        for command in commands {
            self.run(command, &ctx);
        }
    }
}
