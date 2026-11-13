//! Desktop interface: menus, tool and brush panels, and the canvas viewport.

use eframe::egui::{
    self, Color32, ColorImage, CursorIcon, Event, Key, Mesh, PointerButton, Pos2, Rect, Sense,
    Shape, Stroke, TextureFilter, TextureHandle, TextureOptions, TouchPhase, Vec2,
};

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use sharkpaint::engine::{
    brush_preview, write_png, BlendMode, BrushPreset, BrushSettings, BrushTip, Document, IRect,
    LayerTransform, Reference, SelectMode, StrokePoint, PROJECT_EXTENSION,
};

use crate::brushes::{Brush, BrushLibrary, OWN_CATEGORY};
use crate::icons::icon;
use crate::settings::{self, Settings};
use crate::view::{View, MAX_ZOOM, MIN_ZOOM};

const DEFAULT_SIZE: [usize; 2] = [3000, 2000];
const MAX_BRUSH_SIZE: f32 = 500.0;
/// How many half-size copies of the canvas are kept for zoomed-out display.
const REDUCED_LEVELS: usize = 3;

/// An RGBA image at half the size, each pixel the average of the four (or, at
/// an odd edge, fewer) it replaces.
fn halved(rgba: &[u8], width: usize, height: usize) -> (Vec<u8>, usize, usize) {
    let (w, h) = (width.div_ceil(2), height.div_ceil(2));
    let mut out = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        let (y0, y1) = (y * 2, (y * 2 + 1).min(height - 1));
        for x in 0..w {
            let (x0, x1) = (x * 2, (x * 2 + 1).min(width - 1));
            for c in 0..4 {
                let at = |px: usize, py: usize| rgba[(py * width + px) * 4 + c] as u32;
                out.push(((at(x0, y0) + at(x1, y0) + at(x0, y1) + at(x1, y1) + 2) / 4) as u8);
            }
        }
    }
    (out, w, h)
}
/// Seconds between automatic recovery saves.
const AUTOSAVE_SECONDS: f64 = 120.0;
/// Letters that move, rotate and flip the canvas, so tools cannot take them.
const RESERVED_KEYS: [Key; 3] = [Key::M, Key::R, Key::F];
/// Reference images are shrunk to this side at most; they are for looking at.
const MAX_REFERENCE_SIDE: u32 = 2048;
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
    Transformar,
    Seleccion,
}

impl Tool {
    /// (tool, icon, name, shortcut letter) in tool bar order.
    const ALL: [(Tool, &'static str, &'static str, &'static str); 8] = [
        (Tool::Pincel, "pincel", "Pincel", "B"),
        (Tool::Borrador, "borrador", "Borrador", "E"),
        (Tool::Difuminar, "difuminar", "Difuminar", "D"),
        (Tool::Relleno, "relleno", "Relleno", "G"),
        (Tool::Cuentagotas, "cuentagotas", "Cuentagotas", "I"),
        (Tool::Mover, "mover", "Mover capa", "V"),
        (Tool::Transformar, "transformar", "Transformar", "T"),
        (Tool::Seleccion, "seleccion", "Selección", "S"),
    ];

    fn name(self) -> &'static str {
        Tool::ALL.iter().find(|t| t.0 == self).map_or("", |t| t.2)
    }

    fn default_key(self) -> Key {
        let letter = Tool::ALL.iter().find(|t| t.0 == self).map_or("B", |t| t.3);
        Key::from_name(letter).unwrap_or(Key::B)
    }

    /// The tool paints with a round tip whose size the cursor shows.
    fn has_tip(self) -> bool {
        matches!(self, Tool::Pincel | Tool::Borrador | Tool::Difuminar)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SelectShape {
    Rectangulo,
    Elipse,
    Libre,
    /// A click selects the area of similar colour.
    Varita,
}

impl SelectShape {
    const ALL: [(SelectShape, &'static str); 4] = [
        (SelectShape::Rectangulo, "Rectángulo"),
        (SelectShape::Elipse, "Elipse"),
        (SelectShape::Libre, "Forma libre"),
        (SelectShape::Varita, "Varita mágica"),
    ];

    /// The outline of a selection dragged from `from` to `to`, in canvas
    /// coordinates. `trail` is the path of a freehand drag; `regular` makes
    /// rectangles square and ellipses round.
    fn outline(self, from: Pos2, to: Pos2, trail: &[Pos2], regular: bool) -> Vec<(f32, f32)> {
        let mut to = to;
        if regular && matches!(self, SelectShape::Rectangulo | SelectShape::Elipse) {
            let side = (to.x - from.x).abs().max((to.y - from.y).abs());
            to = Pos2::new(from.x + side * (to.x - from.x).signum(), from.y + side * (to.y - from.y).signum());
        }
        let rect = Rect::from_two_pos(from, to);
        match self {
            SelectShape::Rectangulo => {
                vec![(rect.min.x, rect.min.y), (rect.max.x, rect.min.y), (rect.max.x, rect.max.y), (rect.min.x, rect.max.y)]
            }
            SelectShape::Elipse => (0..96)
                .map(|i| {
                    let a = i as f32 / 96.0 * std::f32::consts::TAU;
                    (rect.center().x + rect.width() * 0.5 * a.cos(), rect.center().y + rect.height() * 0.5 * a.sin())
                })
                .collect(),
            SelectShape::Libre | SelectShape::Varita => trail.iter().map(|p| (p.x, p.y)).collect(),
        }
    }
}

/// A selection being dragged out.
struct SelectDrag {
    from: Pos2,
    to: Pos2,
    /// Path of the pointer, for freehand selections.
    trail: Vec<Pos2>,
    /// Screen position of the press, to tell a click from a drag.
    pressed_at: Pos2,
    moved: bool,
}

#[derive(Clone, Copy, PartialEq)]
enum TransformDrag {
    Move,
    Rotate,
    /// Dragging the corner with these signs (-1 or 1 on each axis).
    Scale(f32, f32),
}

/// The on-canvas box of the transform tool.
struct TransformBox {
    /// Bounding box of the layer's contents before the transform, in canvas pixels.
    bounds: Rect,
    transform: LayerTransform,
    /// What is being dragged, where the drag began and the transform at that moment.
    drag: Option<(TransformDrag, Pos2, LayerTransform)>,
}

impl TransformBox {
    fn rotate(v: Vec2, angle: f32) -> Vec2 {
        let (s, c) = angle.sin_cos();
        Vec2::new(v.x * c - v.y * s, v.x * s + v.y * c)
    }

    fn center_of(&self, t: &LayerTransform) -> Pos2 {
        self.bounds.center() + Vec2::new(t.offset.0, t.offset.1)
    }

    /// Canvas position of the corner with the given signs.
    fn corner_of(&self, t: &LayerTransform, sx: f32, sy: f32) -> Pos2 {
        let half = self.bounds.size() * 0.5;
        self.center_of(t) + Self::rotate(Vec2::new(sx * half.x * t.scale.0, sy * half.y * t.scale.1), t.rotation)
    }

    fn corners(&self) -> [Pos2; 4] {
        [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)].map(|(x, y)| self.corner_of(&self.transform, x, y))
    }

    fn contains(&self, canvas: Pos2) -> bool {
        let t = &self.transform;
        let local = Self::rotate(canvas - self.center_of(t), -t.rotation);
        let half = self.bounds.size() * 0.5;
        local.x.abs() <= half.x * t.scale.0 && local.y.abs() <= half.y * t.scale.1
    }

    /// Updates the transform for a drag that has reached `canvas`.
    fn drag_to(&mut self, canvas: Pos2, free: bool) {
        let Some((kind, from, start)) = self.drag else { return };
        let half = self.bounds.size() * 0.5;
        let mut t = start;
        match kind {
            TransformDrag::Move => {
                let d = canvas - from;
                t.offset = (start.offset.0 + d.x, start.offset.1 + d.y);
            }
            TransformDrag::Rotate => {
                let center = self.center_of(&start);
                let turned = (canvas - center).angle() - (from - center).angle();
                t.rotation = start.rotation + turned;
                if free {
                    // Shift snaps to steps of 15°.
                    let step = 15f32.to_radians();
                    t.rotation = (t.rotation / step).round() * step;
                }
            }
            TransformDrag::Scale(sx, sy) => {
                // The opposite corner stays where it is.
                let anchor = self.corner_of(&start, -sx, -sy);
                let reach = Self::rotate(canvas - anchor, -start.rotation);
                let mut scale_x = (reach.x * sx / (2.0 * half.x)).max(0.01);
                let mut scale_y = (reach.y * sy / (2.0 * half.y)).max(0.01);
                if !free {
                    // Keep the proportions unless Shift is held.
                    let ratio = (scale_x / start.scale.0).max(scale_y / start.scale.1);
                    scale_x = start.scale.0 * ratio;
                    scale_y = start.scale.1 * ratio;
                }
                t.scale = (scale_x, scale_y);
                let center = anchor + Self::rotate(Vec2::new(sx * half.x * scale_x, sy * half.y * scale_y), start.rotation);
                let moved = center - self.bounds.center();
                t.offset = (moved.x, moved.y);
            }
        }
        self.transform = t;
    }
}

/// Menu and keyboard commands, applied once per frame after the UI is built.
#[derive(Clone, Copy, PartialEq)]
enum Command {
    NewDialog,
    Open,
    InsertImage,
    PasteImage,
    Copy(bool),
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
    ToggleReferences,
    ToggleNavigator,
    ToggleSymmetry,
    Preferences,
    Fullscreen,
    FinishTransform(bool),
    SelectAll,
    Deselect,
    InvertSelection,
    DeleteSelected,
    FlipContents(bool),
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
    /// The canvas at 1/2, 1/4 and 1/8 size, each pixel the average of the ones
    /// it stands for, shown when zoomed out so strokes stay smooth.
    reduced: Vec<TextureHandle>,
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
    /// The transform tool's box, while a transform is open.
    xform: Option<TransformBox>,
    /// Layer (id, version) and selection found empty, so the transform tool
    /// does not rescan them.
    xform_empty: Option<(u64, u64, u64)>,
    select_shape: SelectShape,
    select_drag: Option<SelectDrag>,
    /// How a new selection combines with the current one.
    select_mode: SelectMode,
    /// How far a colour may be from the one clicked for the magic wand to take it.
    select_tolerance: f32,
    /// The magic wand stops at the edges of the area clicked.
    select_contiguous: bool,
    /// The magic wand looks at the whole picture rather than the active layer.
    select_all_layers: bool,
    /// What Copy or Cut last took: where it was on the canvas and its pixels.
    clip: Option<(IRect, Vec<u8>)>,
    /// The brushes on offer and the tip shapes they use.
    library: BrushLibrary,
    /// Library brush the current settings came from, while they still match.
    brush_index: Option<usize>,
    /// Library brush the current settings started from, even after tweaking.
    brush_source: Option<usize>,
    /// The library as last written to disk.
    saved_brushes: String,
    /// Sample stroke for the brush on screen, with the settings it was drawn from.
    preview: Option<(BrushSettings, TextureHandle)>,
    /// Sample strokes for the library list, by position in the library, each
    /// with the settings it was drawn from.
    brush_thumbs: HashMap<usize, (BrushSettings, TextureHandle)>,
    /// The less common brush settings are on show.
    brush_advanced: bool,
    /// Text the library list is narrowed by.
    brush_search: String,
    /// Group the library list shows; empty for all of them.
    brush_category: String,
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
    /// The fix for moving between monitors of different scale has been tried.
    window_hooked: bool,
    /// The floating brush settings window is showing.
    brush_window_open: bool,
    /// The reference panel is showing.
    refs_open: bool,
    /// The side panel shows the navigator (a small view of the whole canvas)
    /// in place of the colour picker.
    nav_open: bool,
    /// Which reference is on screen.
    ref_index: usize,
    /// Pan (points) and extra zoom of the reference on screen.
    ref_view: (Vec2, f32),
    /// Texture of the reference on screen, with the pixels it was made from.
    ref_texture: Option<(usize, TextureHandle)>,
    /// Where the reference panel was last drawn, to route dropped files.
    refs_rect: Option<Rect>,
    /// The pointer is over the reference panel, so pasted images go there.
    pointer_on_references: bool,
    /// Colours the user saved, and the ones used lately (most recent first).
    palette: Vec<[u8; 3]>,
    recent: Vec<[u8; 3]>,
    /// Shortcut key of each tool, in `Tool::ALL` order.
    tool_keys: Vec<Key>,
    /// The settings as last written to disk.
    saved_settings: Settings,
    prefs_open: bool,
    /// Strokes are mirrored across the canvas's vertical centre line.
    symmetry: bool,
    /// When and at which document revision the recovery file was last written.
    autosave_time: f64,
    autosave_revision: u64,
    autosave_thread: Option<std::thread::JoinHandle<()>>,
    /// A recovery file from an earlier session, waiting for the user's answer.
    recovery_prompt: Option<PathBuf>,
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
            reduced: Vec::new(),
            texture_options: TextureOptions::default(),
            thumbnails: HashMap::new(),
            viewport: Rect::ZERO,
            tool: Tool::Pincel,
            brush: BrushPreset::Tinta.settings(),
            eraser,
            smudge: BrushSettings {
                flow: 0.7,
                hardness: 0.3,
                spacing: 0.1,
                smoothing: 0.0,
                pressure_flow: true,
                ..BrushSettings::round(60.0)
            },
            fill_tolerance: 0.15,
            fill_all_layers: true,
            move_origin: None,
            picking: false,
            xform: None,
            xform_empty: None,
            select_shape: SelectShape::Rectangulo,
            select_drag: None,
            select_mode: SelectMode::Replace,
            select_tolerance: 0.15,
            select_contiguous: true,
            select_all_layers: true,
            clip: None,
            library: BrushLibrary { brushes: Vec::new(), tips: Vec::new(), papers: Vec::new() },
            brush_index: Some(1),
            brush_source: None,
            saved_brushes: String::new(),
            preview: None,
            brush_thumbs: HashMap::new(),
            brush_advanced: false,
            brush_search: String::new(),
            brush_category: String::new(),
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
            window_hooked: false,
            brush_window_open: true,
            refs_open: false,
            nav_open: false,
            ref_index: 0,
            ref_view: (Vec2::ZERO, 1.0),
            ref_texture: None,
            refs_rect: None,
            pointer_on_references: false,
            palette: Vec::new(),
            recent: Vec::new(),
            tool_keys: Tool::ALL.iter().map(|t| t.0.default_key()).collect(),
            saved_settings: Settings::default(),
            prefs_open: false,
            symmetry: false,
            autosave_time: 0.0,
            autosave_revision: 0,
            autosave_thread: None,
            recovery_prompt: None,
        };
        app.library = BrushLibrary::load();
        app.saved_brushes = app.library.to_text();
        app.brush_index = app.library.brushes.iter().position(|b| b.settings == app.brush);
        app.brush_source = app.brush_index;
        let stored = Settings::load();
        app.palette = stored.palette.clone();
        app.recent = stored.recent.clone();
        for (tool, key) in &stored.tool_keys {
            let slot = Tool::ALL.iter().position(|t| t.1 == tool);
            if let (Some(slot), Some(key)) = (slot, Key::from_name(key)) {
                if !RESERVED_KEYS.contains(&key) {
                    app.tool_keys[slot] = key;
                }
            }
        }
        app.saved_settings = app.current_settings();
        // A drawing saved automatically by a session that did not close cleanly.
        if open_path.is_none() {
            app.recovery_prompt = settings::recovery_path().filter(|path| path.exists());
        }
        // A project opens as such; any other file is taken as an image to draw over.
        if let Some(path) = open_path {
            if path.extension().is_some_and(|e| e.eq_ignore_ascii_case(PROJECT_EXTENSION)) {
                app.load_project(path);
            } else if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("abr")) {
                app.import_abr(&path);
            } else {
                app.insert_image_file(&path);
                app.saved_revision = app.doc.revision();
            }
        }
        app
    }

    fn current_settings(&self) -> Settings {
        Settings {
            palette: self.palette.clone(),
            recent: self.recent.clone(),
            tool_keys: Tool::ALL
                .iter()
                .zip(&self.tool_keys)
                .map(|(tool, key)| (tool.1.to_owned(), key.name().to_owned()))
                .collect(),
        }
    }

    /// Writes the preferences when they have changed.
    fn store_settings(&mut self) {
        let current = self.current_settings();
        if current != self.saved_settings {
            current.save();
            self.saved_settings = current;
        }
        let brushes = self.library.to_text();
        if brushes != self.saved_brushes {
            self.library.save();
            self.saved_brushes = brushes;
        }
    }

    /// Remembers a colour that was just painted with.
    fn note_color(&mut self) {
        if self.recent.first() != Some(&self.color) {
            self.recent.retain(|c| *c != self.color);
            self.recent.insert(0, self.color);
            self.recent.truncate(14);
        }
    }

    /// Saves the drawing to the recovery file now and then, on another thread,
    /// so work survives a crash or a power cut.
    fn autosave(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        let busy = self.autosave_thread.as_ref().is_some_and(|t| !t.is_finished());
        if !self.modified() || self.doc.revision() == self.autosave_revision || busy || self.doc.is_stroking() {
            return;
        }
        if now - self.autosave_time < AUTOSAVE_SECONDS {
            ctx.request_repaint_after(std::time::Duration::from_secs(30));
            return;
        }
        let Some(path) = settings::recovery_path() else { return };
        let snapshot = self.doc.snapshot();
        self.autosave_time = now;
        self.autosave_revision = self.doc.revision();
        self.autosave_thread = Some(std::thread::spawn(move || {
            let _ = snapshot.save(&path);
        }));
    }

    /// Removes the recovery file: the drawing is safely saved or was let go.
    fn drop_recovery(&mut self) {
        if let Some(thread) = self.autosave_thread.take() {
            let _ = thread.join();
        }
        if let Some(path) = settings::recovery_path() {
            let _ = std::fs::remove_file(path);
        }
        self.autosave_revision = self.doc.revision();
    }

    fn recovery_window(&mut self, ctx: &egui::Context) {
        let Some(path) = self.recovery_prompt.clone() else { return };
        let mut choice = None;
        egui::Window::new("Recuperar dibujo")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                ui.label("SharkPaint no se cerró bien la última vez y guardó una copia del dibujo.");
                ui.label("¿Quieres recuperarla?");
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button("Recuperar").clicked() {
                        choice = Some(true);
                    }
                    if ui.button("Descartar").clicked() {
                        choice = Some(false);
                    }
                });
            });
        match choice {
            Some(true) => {
                match Document::load(&path) {
                    Ok(doc) => {
                        self.replace_document(doc, None);
                        // Recovered work has never been saved by the user.
                        self.saved_revision = u64::MAX;
                        self.autosave_revision = self.doc.revision();
                        self.status = "Dibujo recuperado. Guárdalo para conservarlo.".into();
                    }
                    Err(e) => self.status = format!("No se pudo recuperar el dibujo: {e}"),
                }
                self.recovery_prompt = None;
            }
            Some(false) => {
                self.recovery_prompt = None;
                self.drop_recovery();
            }
            None => {}
        }
    }

    /// Preferences: the shortcut key of each tool.
    fn preferences_window(&mut self, ctx: &egui::Context) {
        let mut open = self.prefs_open;
        egui::Window::new("Preferencias")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                ui.strong("Atajos de las herramientas");
                ui.label("Si eliges una tecla que ya usa otra herramienta, se intercambian.");
                ui.add_space(4.0);
                let mut change = None;
                egui::Grid::new("atajos_herramientas").num_columns(2).spacing([16.0, 6.0]).show(ui, |ui| {
                    for (slot, (_, _, label, _)) in Tool::ALL.iter().enumerate() {
                        ui.label(*label);
                        egui::ComboBox::from_id_salt(("tecla", slot))
                            .selected_text(self.tool_keys[slot].name())
                            .width(60.0)
                            .show_ui(ui, |ui| {
                                for letter in 'A'..='Z' {
                                    let Some(key) = Key::from_name(&letter.to_string()) else { continue };
                                    if RESERVED_KEYS.contains(&key) {
                                        continue;
                                    }
                                    if ui.selectable_label(self.tool_keys[slot] == key, key.name()).clicked() {
                                        change = Some((slot, key));
                                    }
                                }
                            });
                        ui.end_row();
                    }
                });
                if let Some((slot, key)) = change {
                    if let Some(other) = self.tool_keys.iter().position(|k| *k == key) {
                        self.tool_keys[other] = self.tool_keys[slot];
                    }
                    self.tool_keys[slot] = key;
                }
                ui.add_space(4.0);
                ui.label("M, R y F quedan reservadas para mover, rotar y voltear el lienzo.");
                if ui.button("Restablecer atajos").clicked() {
                    self.tool_keys = Tool::ALL.iter().map(|t| t.0.default_key()).collect();
                }
            });
        self.prefs_open = open;
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
        // Layer ids start over in every document, so old previews must go.
        self.thumbnails.clear();
        self.reduced.clear();
        self.ref_texture = None;
        self.xform = None;
        self.xform_empty = None;
        self.ref_index = 0;
        self.ref_view = (Vec2::ZERO, 1.0);
        self.refs_open = !self.doc.references().is_empty();
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
                self.drop_recovery();
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
                self.drop_recovery();
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
            if hit(cmd, Key::A) {
                commands.push(Command::SelectAll);
            }
            if hit(cmd, Key::D) {
                commands.push(Command::Deselect);
            }
            if hit(cmd | egui::Modifiers::SHIFT, Key::I) {
                commands.push(Command::InvertSelection);
            }
            if hit(none, Key::Delete) {
                commands.push(Command::DeleteSelected);
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
            if hit(none, Key::F) {
                commands.push(Command::Flip);
            }
            if self.tool == Tool::Transformar && self.xform.is_some() {
                if hit(none, Key::Enter) {
                    commands.push(Command::FinishTransform(true));
                }
                if hit(none, Key::Escape) {
                    commands.push(Command::FinishTransform(false));
                }
            }
            if hit(none, Key::F11) {
                commands.push(Command::Fullscreen);
            }
            if hit(none, Key::F5) {
                commands.push(Command::ToggleBrushWindow);
            }
            if hit(none, Key::F6) {
                commands.push(Command::ToggleReferences);
            }
            if hit(none, Key::F7) {
                commands.push(Command::ToggleNavigator);
            }
            for (slot, (tool, ..)) in Tool::ALL.iter().enumerate() {
                if hit(none, self.tool_keys[slot]) {
                    self.tool = *tool;
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
                if ui.button("Insertar imagen…").clicked() {
                    commands.push(Command::InsertImage);
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
                if ui.button("Preferencias…").clicked() {
                    commands.push(Command::Preferences);
                }
                ui.separator();
                if ui.add(egui::Button::new("Cortar").shortcut_text("Ctrl+X")).clicked() {
                    commands.push(Command::Copy(true));
                }
                if ui.add(egui::Button::new("Copiar").shortcut_text("Ctrl+C")).clicked() {
                    commands.push(Command::Copy(false));
                }
                if ui.add(egui::Button::new("Pegar").shortcut_text("Ctrl+V")).clicked() {
                    commands.push(Command::PasteImage);
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
                if ui.button("Voltear horizontalmente").clicked() {
                    commands.push(Command::FlipContents(true));
                }
                if ui.button("Voltear verticalmente").clicked() {
                    commands.push(Command::FlipContents(false));
                }
                ui.separator();
                if ui.button("Limpiar capa").clicked() {
                    commands.push(Command::ClearLayer);
                }
                if ui.add_enabled(many, egui::Button::new("Eliminar capa")).clicked() {
                    commands.push(Command::LayerDelete);
                }
            });
            ui.menu_button("Selección", |ui| {
                let selected = self.doc.selection().is_some();
                if ui.add(egui::Button::new("Seleccionar todo").shortcut_text("Ctrl+A")).clicked() {
                    commands.push(Command::SelectAll);
                }
                if ui.add_enabled(selected, egui::Button::new("Deseleccionar").shortcut_text("Ctrl+D")).clicked() {
                    commands.push(Command::Deselect);
                }
                if ui.add(egui::Button::new("Invertir selección").shortcut_text("Ctrl+Mayús+I")).clicked() {
                    commands.push(Command::InvertSelection);
                }
                ui.separator();
                if ui.add_enabled(selected, egui::Button::new("Borrar lo seleccionado").shortcut_text("Supr")).clicked() {
                    commands.push(Command::DeleteSelected);
                }
                ui.separator();
                let what = if selected { "la selección" } else { "la capa" };
                if ui.button(format!("Voltear {what} horizontalmente")).clicked() {
                    commands.push(Command::FlipContents(true));
                }
                if ui.button(format!("Voltear {what} verticalmente")).clicked() {
                    commands.push(Command::FlipContents(false));
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
                if item(ui, "Voltear horizontalmente", "F") {
                    commands.push(Command::Flip);
                }
                ui.separator();
                if item(ui, "Ajustes del pincel", "F5") {
                    commands.push(Command::ToggleBrushWindow);
                }
                if ui.add(egui::Button::new("Simetría").selected(self.symmetry)).clicked() {
                    commands.push(Command::ToggleSymmetry);
                }
                if item(ui, "Referencias", "F6") {
                    commands.push(Command::ToggleReferences);
                }
                if item(ui, "Navegador", "F7") {
                    commands.push(Command::ToggleNavigator);
                }
                if item(ui, "Pantalla completa", "F11") {
                    commands.push(Command::Fullscreen);
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
                        ("Herramientas", "Ver y cambiar en Editar → Preferencias"),
                        ("Intro / Esc", "Aplicar / cancelar la transformación"),
                        ("Ctrl+A / Ctrl+D", "Seleccionar todo / Deseleccionar"),
                        ("Supr", "Borrar lo seleccionado"),
                        ("[ / ]", "Tamaño del pincel"),
                        ("F5", "Mostrar u ocultar los ajustes del pincel"),
                        ("F6", "Mostrar u ocultar las referencias"),
                        ("Alt + clic", "Tomar color del lienzo"),
                        ("M + arrastrar", "Mover el lienzo"),
                        ("Espacio + arrastrar", "Mover el lienzo"),
                        ("Botón central", "Mover el lienzo"),
                        ("Rueda del ratón", "Zoom"),
                        ("R + arrastrar", "Rotar el lienzo"),
                        ("F", "Voltear horizontalmente"),
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
        for (slot, (tool, name, label, _)) in Tool::ALL.into_iter().enumerate() {
            let tip = format!("{label} ({})\nClic de nuevo: ajustes", self.tool_keys[slot].name());
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
        let button = egui::Button::image(icon("imagen", 20.0, text)).selected(self.refs_open).min_size(size);
        if ui.add(button).on_hover_text("Referencias (F6)").clicked() {
            commands.push(Command::ToggleReferences);
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

    /// Adds an image to the reference panel, shrunk if it is very large.
    fn add_reference(&mut self, name: &str, width: usize, height: usize, rgba: Vec<u8>) {
        if width == 0 || height == 0 || rgba.len() != width * height * 4 {
            self.status = "La imagen no se pudo leer.".into();
            return;
        }
        let (mut width, mut height, mut rgba) = (width as u32, height as u32, rgba);
        let largest = width.max(height);
        if largest > MAX_REFERENCE_SIDE {
            let scale = MAX_REFERENCE_SIDE as f64 / largest as f64;
            let (w, h) = (((width as f64 * scale) as u32).max(1), ((height as f64 * scale) as u32).max(1));
            let Some(source) = image::RgbaImage::from_raw(width, height, rgba) else { return };
            rgba = image::imageops::resize(&source, w, h, image::imageops::FilterType::Triangle).into_raw();
            (width, height) = (w, h);
        }
        self.doc.add_reference(Reference {
            name: name.to_owned(),
            width: width as usize,
            height: height as usize,
            rgba: std::sync::Arc::new(rgba),
        });
        self.ref_index = self.doc.references().len() - 1;
        self.ref_view = (Vec2::ZERO, 1.0);
        self.refs_open = true;
    }

    fn add_reference_file(&mut self, path: &std::path::Path) {
        match image::open(path) {
            Ok(decoded) => {
                let rgba = decoded.into_rgba8();
                let name = path.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "Referencia".into());
                let (w, h) = (rgba.width() as usize, rgba.height() as usize);
                self.add_reference(&name, w, h, rgba.into_raw());
            }
            Err(e) => self.status = format!("No se pudo abrir {}: {e}", path.display()),
        }
    }

    /// The reference panel: images to look at while drawing. They live beside
    /// the picture, with their own zoom, and are never exported.
    /// A small view of the whole canvas, in the side panel. The outline marks
    /// what the main view shows; a click or a drag moves the main view there.
    fn navigator_panel(&mut self, ui: &mut egui::Ui, commands: &mut Vec<Command>) {
        let (viewport, doc_size) = (self.viewport, self.doc_size());
        let side = ui.available_width().clamp(120.0, 260.0);
        let scale = (side / doc_size.x).min(side * 0.8 / doc_size.y);
        let (rect, response) = ui.allocate_exact_size(doc_size * scale, Sense::click_and_drag());
        let flip = self.view.flip;
        // Canvas pixel <-> point in the picture, mirrored like the main view.
        let to_nav = |c: Pos2| {
            let x = if flip { doc_size.x - c.x } else { c.x };
            rect.min + Vec2::new(x, c.y) * scale
        };
        let to_canvas = |p: Pos2| {
            let v = (p - rect.min) / scale;
            Pos2::new(if flip { doc_size.x - v.x } else { v.x }, v.y)
        };
        ui.painter().rect_filled(rect, 0.0, Color32::from_gray(128));
        // The smallest copy of the canvas is plenty at this size.
        let level = self.reduced.len();
        if let Some(texture) = self.reduced.last().or(self.texture.as_ref()) {
            let reach = if level == 0 {
                Vec2::splat(1.0)
            } else {
                Vec2::new(
                    (doc_size.x / (texture.size()[0] << level) as f32).min(1.0),
                    (doc_size.y / (texture.size()[1] << level) as f32).min(1.0),
                )
            };
            let (left, right) = if flip { (reach.x, 0.0) } else { (0.0, reach.x) };
            let uv = Rect::from_min_max(Pos2::new(left, 0.0), Pos2::new(right, reach.y));
            ui.painter().image(texture.id(), rect, uv, Color32::WHITE);
        }
        // What the main view shows, as an outline; it turns with the view.
        let outline: Vec<Pos2> =
            [viewport.left_top(), viewport.right_top(), viewport.right_bottom(), viewport.left_bottom()]
                .into_iter()
                .map(|corner| to_nav(self.view.to_canvas(corner, viewport, doc_size)))
                .collect();
        let painter = ui.painter().with_clip_rect(rect);
        painter.add(egui::Shape::closed_line(outline.clone(), Stroke::new(3.0, Color32::from_black_alpha(140))));
        painter.add(egui::Shape::closed_line(outline, Stroke::new(1.5, Color32::from_rgb(111, 227, 240))));
        ui.painter().rect_stroke(rect, 0.0, Stroke::new(1.0, Color32::from_gray(60)), egui::StrokeKind::Outside);

        // Press or drag: bring that spot to the middle of the main view.
        if let Some(pos) = response.interact_pointer_pos().filter(|_| response.is_pointer_button_down_on()) {
            let target = to_canvas(rect.clamp(pos));
            self.view.pan += viewport.center() - self.view.to_screen(target, viewport, doc_size);
        }
        if response.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
                self.view.zoom_about(viewport.center(), (scroll * 0.004).exp(), viewport, doc_size);
            }
        }

        ui.add_space(2.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().slider_width = side - 150.0;
            let mut zoom = self.view.zoom;
            let slider = egui::Slider::new(&mut zoom, MIN_ZOOM..=MAX_ZOOM)
                .logarithmic(true)
                .custom_formatter(|v, _| format!("{:.0} %", v * 100.0))
                .custom_parser(|text| text.trim().trim_end_matches('%').trim().parse::<f64>().ok().map(|v| v / 100.0));
            if ui.add(slider).on_hover_text("Zoom").changed() {
                self.view.zoom_about(viewport.center(), zoom / self.view.zoom, viewport, doc_size);
            }
            if ui.small_button("Ajustar").on_hover_text("Ver el lienzo completo").clicked() {
                commands.push(Command::Fit);
            }
        });
        ui.horizontal(|ui| {
            ui.spacing_mut().slider_width = side - 150.0;
            // Shown from -180° to 180°, so a small turn either way reads as small.
            let mut degrees = self.view.rotation.to_degrees();
            if degrees > 180.0 {
                degrees -= 360.0;
            }
            let before = degrees;
            let slider = egui::Slider::new(&mut degrees, -180.0..=180.0).fixed_decimals(0).suffix("°");
            if ui.add(slider).on_hover_text("Rotación").changed() {
                self.view.rotate_about(viewport.center(), (degrees - before).to_radians(), viewport, doc_size);
            }
            if ui.small_button("Recto").on_hover_text("Quitar la rotación").clicked() {
                commands.push(Command::ResetRotation);
            }
        });
    }

    fn references_window(&mut self, ctx: &egui::Context) {
        let mut open = self.refs_open;
        let mut add_files = false;
        let mut paste = false;
        let mut remove = false;
        let shown = egui::Window::new("Referencias")
            .open(&mut open)
            .default_size([340.0, 380.0])
            .min_size([220.0, 200.0])
            .default_pos(self.viewport.right_top() + Vec2::new(-360.0, 12.0))
            .show(ctx, |ui| {
                let count = self.doc.references().len();
                self.ref_index = self.ref_index.min(count.saturating_sub(1));
                ui.horizontal(|ui| {
                    add_files = ui.button("Añadir…").clicked();
                    paste = ui.button("Pegar").on_hover_text("Añade la imagen del portapapeles").clicked();
                    remove = ui.add_enabled(count > 0, egui::Button::new("Quitar")).clicked();
                    if count > 1 {
                        ui.separator();
                        if ui.small_button("◀").clicked() {
                            self.ref_index = (self.ref_index + count - 1) % count;
                            self.ref_view = (Vec2::ZERO, 1.0);
                        }
                        ui.label(format!("{} / {count}", self.ref_index + 1));
                        if ui.small_button("▶").clicked() {
                            self.ref_index = (self.ref_index + 1) % count;
                            self.ref_view = (Vec2::ZERO, 1.0);
                        }
                    }
                });
                let Some(reference) = self.doc.references().get(self.ref_index).cloned() else {
                    ui.add_space(8.0);
                    ui.label("Añade imágenes de consulta: se quedan aquí, fuera del lienzo y de las capas.");
                    ui.label("Se guardan con el proyecto y nunca se incluyen al exportar.");
                    ui.label("También puedes soltar un archivo sobre esta ventana.");
                    ui.allocate_space(ui.available_size());
                    return;
                };
                // One texture, remade when another image comes on screen.
                let key = std::sync::Arc::as_ptr(&reference.rgba) as usize;
                if self.ref_texture.as_ref().is_none_or(|(k, _)| *k != key) {
                    let image = ColorImage::from_rgba_unmultiplied([reference.width, reference.height], &reference.rgba);
                    self.ref_texture = Some((key, ui.ctx().load_texture("referencia", image, TextureOptions::LINEAR)));
                }
                let (response, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
                let area = response.rect;
                painter.rect_filled(area, 2.0, Color32::from_gray(24));
                let size = Vec2::new(reference.width as f32, reference.height as f32);
                let fit = (area.width() / size.x).min(area.height() / size.y);
                if response.dragged() {
                    self.ref_view.0 += response.drag_delta();
                }
                if response.double_clicked() {
                    self.ref_view = (Vec2::ZERO, 1.0);
                }
                let hover = response.hover_pos();
                if let Some(pos) = hover {
                    let scroll = ui.input(|i| i.smooth_scroll_delta.y);
                    if scroll != 0.0 {
                        // Zoom around the pointer.
                        let old = fit * self.ref_view.1;
                        self.ref_view.1 = (self.ref_view.1 * (scroll * 0.0025).exp()).clamp(0.2, 40.0);
                        let center = area.center() + self.ref_view.0;
                        self.ref_view.0 += (pos - center) * (1.0 - fit * self.ref_view.1 / old);
                    }
                    ui.ctx().set_cursor_icon(CursorIcon::Crosshair);
                }
                let scale = fit * self.ref_view.1;
                let shown = Rect::from_center_size(area.center() + self.ref_view.0, size * scale);
                if let Some((_, texture)) = &self.ref_texture {
                    let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
                    painter.with_clip_rect(area).image(texture.id(), shown, uv, Color32::WHITE);
                }
                // A click takes the colour under the pointer.
                if let Some(pos) = hover.filter(|_| response.clicked() && !response.double_clicked()) {
                    let at = (pos - shown.min) / scale;
                    let (x, y) = (at.x.floor() as i64, at.y.floor() as i64);
                    if x >= 0 && y >= 0 && (x as usize) < reference.width && (y as usize) < reference.height {
                        let i = (y as usize * reference.width + x as usize) * 4;
                        self.color = [reference.rgba[i], reference.rgba[i + 1], reference.rgba[i + 2]];
                    }
                }
            });
        self.refs_rect = shown.map(|window| window.response.rect);
        self.refs_open = open;
        if add_files {
            let picked = rfd::FileDialog::new()
                .set_title("Añadir referencias")
                .add_filter("Imágenes", &["png", "jpg", "jpeg", "webp", "bmp", "gif"])
                .pick_files();
            for path in picked.unwrap_or_default() {
                self.add_reference_file(&path);
            }
        }
        if paste {
            match arboard::Clipboard::new().and_then(|mut clipboard| clipboard.get_image()) {
                Ok(image) => self.add_reference("Referencia pegada", image.width, image.height, image.bytes.into_owned()),
                Err(_) => self.status = "No hay ninguna imagen en el portapapeles.".into(),
            }
        }
        if remove {
            self.doc.remove_reference(self.ref_index);
            self.ref_view = (Vec2::ZERO, 1.0);
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
        let mut color = Color32::from_rgb(self.color[0], self.color[1], self.color[2]);
        ui.scope(|ui| {
            // The picker takes its width from the slider width; make it fill the panel.
            ui.spacing_mut().slider_width = (ui.available_width() - 8.0).clamp(120.0, 230.0);
            if egui::color_picker::color_picker_color32(ui, &mut color, egui::color_picker::Alpha::Opaque) {
                self.color = [color.r(), color.g(), color.b()];
            }
        });

        // Saved colours, then the ones used lately.
        let swatch = |ui: &mut egui::Ui, color: [u8; 3], tip: &str| {
            let (rect, response) = ui.allocate_exact_size(Vec2::splat(18.0), Sense::click());
            ui.painter().rect_filled(rect, 3.0, Color32::from_rgb(color[0], color[1], color[2]));
            let edge = if response.hovered() { Color32::WHITE } else { Color32::from_gray(70) };
            ui.painter().rect_stroke(rect, 3.0, Stroke::new(1.0, edge), egui::StrokeKind::Inside);
            response.on_hover_text(format!("#{:02X}{:02X}{:02X}\n{tip}", color[0], color[1], color[2]))
        };
        ui.add_space(2.0);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::splat(3.0);
            ui.label("Paleta");
            let mut remove = None;
            for (i, color) in self.palette.clone().into_iter().enumerate() {
                let response = swatch(ui, color, "Clic: usar · Clic derecho: quitar");
                if response.clicked() {
                    self.color = color;
                }
                if response.secondary_clicked() {
                    remove = Some(i);
                }
            }
            if let Some(i) = remove {
                self.palette.remove(i);
            }
            let add = egui::Button::new("+").small().min_size(Vec2::splat(18.0));
            if ui.add(add).on_hover_text("Guardar el color actual en la paleta").clicked()
                && !self.palette.contains(&self.color)
            {
                self.palette.push(self.color);
            }
        });
        if !self.recent.is_empty() {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = Vec2::splat(3.0);
                ui.label("Recientes");
                for color in self.recent.clone() {
                    if swatch(ui, color, "Clic: usar").clicked() {
                        self.color = color;
                    }
                }
            });
        }
    }

    fn transform_panel(&mut self, ui: &mut egui::Ui) {
        let Some(xform) = &mut self.xform else {
            ui.label("La capa activa está vacía u oculta: no hay nada que transformar.");
            return;
        };
        let mut t = xform.transform;
        let (mut width, mut height) = (t.scale.0 * 100.0, t.scale.1 * 100.0);
        let mut degrees = t.rotation.to_degrees();
        egui::Grid::new("transformar").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
            ui.label("Ancho");
            ui.add(egui::DragValue::new(&mut width).range(1.0..=2000.0).speed(0.5).fixed_decimals(0).suffix(" %"));
            ui.end_row();
            ui.label("Alto");
            ui.add(egui::DragValue::new(&mut height).range(1.0..=2000.0).speed(0.5).fixed_decimals(0).suffix(" %"));
            ui.end_row();
            ui.label("Rotación");
            ui.add(egui::DragValue::new(&mut degrees).speed(0.5).fixed_decimals(1).suffix("°"));
            ui.end_row();
        });
        t.scale = (width / 100.0, height / 100.0);
        t.rotation = degrees.to_radians();
        let mut apply = false;
        let mut cancel = false;
        ui.horizontal(|ui| {
            apply = ui.button("Aplicar").on_hover_text("Intro").clicked();
            cancel = ui.button("Cancelar").on_hover_text("Esc").clicked();
            if ui.button("Restablecer").clicked() {
                t = LayerTransform::IDENTITY;
            }
        });
        ui.add_space(4.0);
        ui.label("Esquinas: cambiar tamaño (Mayús: sin mantener proporción).");
        ui.label("Dentro de la caja: mover. Fuera: rotar (Mayús: de 15° en 15°).");
        if t != xform.transform {
            xform.transform = t;
            self.doc.set_transform(t);
        }
        if apply {
            self.finish_transform(true);
        } else if cancel {
            self.finish_transform(false);
        }
    }

    /// Applies or discards the open transform.
    fn finish_transform(&mut self, apply: bool) {
        if self.xform.take().is_some() {
            if apply {
                self.doc.end_stroke();
            } else {
                self.doc.cancel_stroke();
            }
        }
    }

    /// Opens a transform on the active layer while the transform tool is
    /// selected, and applies it when the tool is put away.
    fn sync_transform(&mut self) {
        if self.tool != Tool::Transformar {
            self.finish_transform(true);
            return;
        }
        if self.xform.is_some() && !self.doc.is_transforming() {
            self.xform = None; // something else (undo, saving…) closed it
        }
        if self.xform.is_none() && self.doc.active().props.visible {
            let key = (self.doc.active().id, self.doc.active().version, self.doc.selection_stamp());
            if self.xform_empty == Some(key) {
                return;
            }
            match self.doc.begin_transform() {
                Some(b) => {
                    let bounds = Rect::from_min_max(
                        Pos2::new(b.x0 as f32, b.y0 as f32),
                        Pos2::new(b.x1 as f32, b.y1 as f32),
                    );
                    self.xform = Some(TransformBox { bounds, transform: LayerTransform::IDENTITY, drag: None });
                }
                None => self.xform_empty = Some(key),
            }
        }
    }

    /// The library of brushes: pick one, save the current settings as a new
    /// one, update, rename or delete.
    /// The brushes on offer: a search box, a group picker and a list where
    /// each brush shows a sample stroke.
    fn brush_library(&mut self, ui: &mut egui::Ui) {
        let width = ui.available_width();
        ui.add(egui::TextEdit::singleline(&mut self.brush_search).hint_text("Buscar pincel").desired_width(width));
        let mut categories: Vec<String> = Vec::new();
        for brush in &self.library.brushes {
            if !categories.contains(&brush.category) {
                categories.push(brush.category.clone());
            }
        }
        if !self.brush_category.is_empty() && !categories.contains(&self.brush_category) {
            self.brush_category.clear();
        }
        ui.horizontal(|ui| {
            let shown = if self.brush_category.is_empty() { "Todos los pinceles" } else { self.brush_category.as_str() };
            egui::ComboBox::from_id_salt("categoria_pinceles").selected_text(shown).width(width - 44.0).show_ui(ui, |ui| {
                ui.selectable_value(&mut self.brush_category, String::new(), "Todos los pinceles");
                for category in &categories {
                    ui.selectable_value(&mut self.brush_category, category.clone(), category);
                }
            });
            ui.menu_button("⋯", |ui| {
                if ui.button("Importar de Photoshop (.abr)…").clicked() {
                    ui.close();
                    if let Some(path) = rfd::FileDialog::new()
                        .set_title("Importar pinceles de Photoshop")
                        .add_filter("Pinceles de Photoshop", &["abr"])
                        .pick_file()
                    {
                        self.import_abr(&path);
                    }
                }
                if ui.button("Restaurar los pinceles iniciales").clicked() {
                    ui.close();
                    self.library.brushes = self.library.default_brushes();
                    self.brush_index = self.library.brushes.iter().position(|b| b.settings == self.brush);
                    self.brush_source = self.brush_index;
                }
            })
            .response
            .on_hover_text("Importar pinceles de Photoshop (también arrastrando el archivo a la ventana)\no volver a los pinceles iniciales");
        });

        let search = self.brush_search.trim().to_lowercase();
        let listed: Vec<usize> = (0..self.library.brushes.len())
            .filter(|&i| {
                let brush = &self.library.brushes[i];
                (self.brush_category.is_empty() || brush.category == self.brush_category)
                    && (search.is_empty() || brush.name.to_lowercase().contains(&search))
            })
            .collect();
        // Samples are drawn a few per frame so a long list never stalls the window.
        let mut budget = 3;
        const ROW: f32 = 50.0;
        const SAMPLE: [usize; 2] = [232, 28];
        let source = self.brush_source;
        // Tall enough for a handful of brushes; shorter when there are fewer.
        let height = ROW * (listed.len() as f32).clamp(1.0, 5.5);
        ui.spacing_mut().item_spacing.y = 4.0;
        egui::ScrollArea::vertical().id_salt("biblioteca").max_height(height).min_scrolled_height(height).auto_shrink([false, false]).show_rows(
            ui,
            ROW,
            listed.len(),
            |ui, rows| {
                for &i in &listed[rows] {
                    let settings = &self.library.brushes[i].settings;
                    if budget > 0 && self.brush_thumbs.get(&i).is_none_or(|(drawn, _)| drawn != settings) {
                        budget -= 1;
                        let rgba = brush_preview(settings, SAMPLE[0] * 2, SAMPLE[1] * 2, [230, 232, 238], [20, 22, 28]);
                        let image = ColorImage::from_rgba_unmultiplied([SAMPLE[0] * 2, SAMPLE[1] * 2], &rgba);
                        let texture = ui.ctx().load_texture(format!("pincel_{i}"), image, TextureOptions::LINEAR);
                        self.brush_thumbs.insert(i, (settings.clone(), texture));
                    }
                    let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW - 4.0), Sense::click());
                    let chosen = source == Some(i);
                    let fill = if chosen {
                        ui.visuals().selection.bg_fill
                    } else if response.hovered() {
                        ui.visuals().widgets.hovered.weak_bg_fill
                    } else {
                        ui.visuals().faint_bg_color
                    };
                    ui.painter().rect_filled(rect, 4.0, fill);
                    let sample = Rect::from_min_size(rect.min + Vec2::new(4.0, 4.0), Vec2::new(rect.width() - 8.0, SAMPLE[1] as f32));
                    match self.brush_thumbs.get(&i).filter(|(drawn, _)| drawn == settings) {
                        Some((_, texture)) => {
                            let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
                            ui.painter().image(texture.id(), sample, uv, Color32::WHITE);
                        }
                        None => ui.ctx().request_repaint(),
                    }
                    let brush = &self.library.brushes[i];
                    let text = if chosen { ui.visuals().strong_text_color() } else { ui.visuals().text_color() };
                    let line = rect.bottom() - 8.0;
                    let font = egui::FontId::proportional(11.5);
                    let size = ui.painter().text(
                        Pos2::new(rect.left() + 6.0, line),
                        egui::Align2::LEFT_CENTER,
                        format!("{:.0}", brush.settings.size),
                        font.clone(),
                        if chosen { text } else { ui.visuals().weak_text_color() },
                    );
                    ui.painter().text(
                        Pos2::new(size.right().max(rect.left() + 30.0) + 6.0, line),
                        egui::Align2::LEFT_CENTER,
                        &brush.name,
                        font,
                        text,
                    );
                    if response.on_hover_text(format!("{} · {}", brush.name, brush.category)).clicked() {
                        self.brush = brush.settings.clone();
                        self.brush_index = Some(i);
                        self.brush_source = Some(i);
                    }
                }
            },
        );
        if listed.is_empty() {
            ui.weak("Ningún pincel coincide.");
        }
    }

    /// Name and group of the brush in use, and saving it to the library.
    fn brush_storage(&mut self, ui: &mut egui::Ui) {
        if let Some(i) = self.brush_source.filter(|&i| i < self.library.brushes.len()) {
            egui::Grid::new("pincel_guardado").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
                ui.label("Nombre");
                ui.add(egui::TextEdit::singleline(&mut self.library.brushes[i].name).desired_width(150.0));
                ui.end_row();
                ui.label("Grupo");
                ui.add(egui::TextEdit::singleline(&mut self.library.brushes[i].category).desired_width(150.0))
                    .on_hover_text("Los pinceles con el mismo grupo se listan juntos");
                ui.end_row();
            });
        }
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            if ui.small_button("Guardar nuevo").on_hover_text("Añade los ajustes actuales como un pincel nuevo").clicked() {
                let name = format!("Pincel {}", self.library.brushes.len() + 1);
                self.library.brushes.push(Brush { name, category: OWN_CATEGORY.to_owned(), settings: self.brush.clone() });
                self.brush_index = Some(self.library.brushes.len() - 1);
                self.brush_source = self.brush_index;
            }
            let stored = self.brush_index.filter(|&i| i < self.library.brushes.len());
            // The settings on screen have drifted from the brush they came from.
            let source = self.brush_source.filter(|&i| i < self.library.brushes.len());
            let update = egui::Button::new("Actualizar").small();
            if ui.add_enabled(source.is_some() && stored.is_none(), update)
                .on_hover_text("Guarda los cambios en el pincel elegido")
                .clicked()
            {
                if let Some(i) = source {
                    self.library.brushes[i].settings = self.brush.clone();
                    self.brush_index = Some(i);
                }
            }
            let delete = egui::Button::new("Eliminar").small();
            if ui.add_enabled(source.is_some() && self.library.brushes.len() > 1, delete).clicked() {
                if let Some(i) = source {
                    self.library.brushes.remove(i);
                    self.brush_index = None;
                    self.brush_source = None;
                }
            }
        });
    }

    /// A sample stroke of the brush on screen.
    fn brush_sample(&mut self, ui: &mut egui::Ui) {
        const SIZE: [usize; 2] = [248, 64];
        let brush = self.active_brush().clone();
        if self.preview.as_ref().is_none_or(|(drawn, _)| *drawn != brush) {
            let rgba = brush_preview(&brush, SIZE[0] * 2, SIZE[1] * 2, [230, 232, 238], [20, 22, 28]);
            let image = ColorImage::from_rgba_unmultiplied([SIZE[0] * 2, SIZE[1] * 2], &rgba);
            let texture = ui.ctx().load_texture("muestra_pincel", image, TextureOptions::LINEAR);
            self.preview = Some((brush, texture));
        }
        if let Some((_, texture)) = &self.preview {
            let (rect, _) = ui.allocate_exact_size(Vec2::new(SIZE[0] as f32, SIZE[1] as f32), Sense::hover());
            let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
            ui.painter().image(texture.id(), rect, uv, Color32::WHITE);
            ui.painter().rect_stroke(rect, 3.0, Stroke::new(1.0, Color32::from_gray(60)), egui::StrokeKind::Inside);
        }
    }

    /// The shape of the tip and how each dab varies.
    fn tip_settings(&mut self, ui: &mut egui::Ui) {
        let percent = |v: f64, _: std::ops::RangeInclusive<usize>| format!("{:.0} %", v * 100.0);
        let tips = self.library.tips.clone();
        let mut import = false;
        let b = self.active_brush();
        egui::Grid::new("punta").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
            ui.label("Forma");
            ui.horizontal(|ui| {
                let current = match &b.tip {
                    BrushTip::Round => "Redonda".to_owned(),
                    BrushTip::Image(tip) => tip.name.clone(),
                };
                egui::ComboBox::from_id_salt("forma_punta").selected_text(current).width(120.0).show_ui(ui, |ui| {
                    if ui.selectable_label(b.tip == BrushTip::Round, "Redonda").clicked() {
                        b.tip = BrushTip::Round;
                    }
                    for tip in &tips {
                        let this = BrushTip::Image(tip.clone());
                        if ui.selectable_label(b.tip == this, &tip.name).clicked() {
                            b.tip = this;
                        }
                    }
                });
                import = ui.small_button("Importar…").on_hover_text(
                    "Usa una imagen como punta: lo oscuro pinta.\nSi la imagen tiene transparencia, pinta lo opaco.",
                ).clicked();
            });
            ui.end_row();
            ui.label("Ángulo");
            ui.add(egui::Slider::new(&mut b.angle, -180.0..=180.0).fixed_decimals(0).suffix("°"));
            ui.end_row();
            ui.label("Redondez");
            ui.add(egui::Slider::new(&mut b.roundness, 0.05..=1.0).custom_formatter(percent));
            ui.end_row();
            ui.label("Giro al azar");
            ui.add(egui::Slider::new(&mut b.angle_jitter, 0.0..=1.0).custom_formatter(percent));
            ui.end_row();
            ui.label("Dispersión");
            ui.add(egui::Slider::new(&mut b.scatter, 0.0..=3.0).custom_formatter(percent));
            ui.end_row();
            ui.label("Tamaño al azar");
            ui.add(egui::Slider::new(&mut b.size_jitter, 0.0..=1.0).custom_formatter(percent));
            ui.end_row();
        });
        ui.checkbox(&mut b.follow_direction, "Girar con la dirección del trazo");
        if import {
            let picked = rfd::FileDialog::new()
                .set_title("Importar punta")
                .add_filter("Imágenes", &["png", "jpg", "jpeg", "webp", "bmp", "gif"])
                .pick_file();
            if let Some(path) = picked {
                match self.library.import_tip(&path) {
                    Ok(tip) => {
                        self.status = format!("Punta «{}» importada.", tip.name);
                        self.active_brush().tip = BrushTip::Image(tip);
                    }
                    Err(e) => self.status = format!("No se pudo importar la punta: {e}"),
                }
            }
        }
    }

    /// The paper under the brush and how much of its grain shows.
    fn paper_settings(&mut self, ui: &mut egui::Ui) {
        let percent = |v: f64, _: std::ops::RangeInclusive<usize>| format!("{:.0} %", v * 100.0);
        let papers = self.library.papers.clone();
        let b = self.active_brush();
        egui::Grid::new("papel").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
            ui.label("Papel");
            let current = b.paper.as_ref().map_or("Ninguno".to_owned(), |p| p.name.clone());
            egui::ComboBox::from_id_salt("papel_pincel").selected_text(current).width(150.0).show_ui(ui, |ui| {
                if ui.selectable_label(b.paper.is_none(), "Ninguno").clicked() {
                    b.paper = None;
                }
                for paper in &papers {
                    if ui.selectable_label(b.paper.as_ref() == Some(paper), &paper.name).clicked() {
                        b.paper = Some(paper.clone());
                    }
                }
            });
            ui.end_row();
            let has_paper = b.paper.is_some();
            ui.label("Intensidad");
            ui.add_enabled(has_paper, egui::Slider::new(&mut b.grain, 0.0..=1.0).custom_formatter(percent));
            ui.end_row();
            ui.label("Tamaño del grano");
            ui.add_enabled(
                has_paper,
                egui::Slider::new(&mut b.grain_scale, 0.25..=6.0).logarithmic(true).custom_formatter(percent),
            );
            ui.end_row();
        });
        ui.label("Con poca presión el trazo solo toca los relieves del papel; con más, lo cubre.");
    }

    /// How much the brush mixes with the paint already on the canvas.
    fn mix_settings(&mut self, ui: &mut egui::Ui) {
        let percent = |v: f64, _: std::ops::RangeInclusive<usize>| format!("{:.0} %", v * 100.0);
        let b = self.active_brush();
        egui::Grid::new("mezcla").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
            ui.label("Mezcla");
            ui.add(egui::Slider::new(&mut b.mix, 0.0..=1.0).custom_formatter(percent))
                .on_hover_text("0 %: pinta solo con su color.\n100 %: no añade color, solo mueve el que ya hay.");
            ui.end_row();
            ui.label("Arrastre");
            ui.add_enabled(b.mix > 0.0, egui::Slider::new(&mut b.drag, 0.0..=1.0).custom_formatter(percent))
                .on_hover_text("Cuánto tiempo conserva el pincel el color que recoge");
            ui.end_row();
        });
        if b.mix > 0.0 {
            ui.label("Al mezclar, el pincel pinta directamente: la opacidad no se aplica; usa el flujo.");
        } else {
            ui.label("Sube la mezcla para que el pincel recoja el color del lienzo y lo arrastre.");
        }
    }

    fn brush_panel(&mut self, ui: &mut egui::Ui) {
        let percent = |v: f64, _: std::ops::RangeInclusive<usize>| format!("{:.0} %", v * 100.0);
        match self.tool {
            Tool::Pincel | Tool::Borrador | Tool::Difuminar => {}
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
            Tool::Transformar => {
                self.transform_panel(ui);
                return;
            }
            Tool::Seleccion => {
                ui.horizontal_wrapped(|ui| {
                    for (shape, name) in SelectShape::ALL {
                        ui.selectable_value(&mut self.select_shape, shape, name);
                    }
                });
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    for (mode, name, tip) in [
                        (SelectMode::Replace, "Nueva", "Sustituye la selección"),
                        (SelectMode::Add, "Añadir", "Suma a la selección"),
                        (SelectMode::Subtract, "Quitar", "Resta de la selección"),
                        (SelectMode::Intersect, "Intersecar", "Deja solo lo que coincide con la selección"),
                    ] {
                        ui.selectable_value(&mut self.select_mode, mode, name).on_hover_text(tip);
                    }
                });
                ui.add_space(4.0);
                if self.select_shape == SelectShape::Varita {
                    ui.horizontal(|ui| {
                        ui.label("Tolerancia");
                        ui.add(egui::Slider::new(&mut self.select_tolerance, 0.0..=1.0).custom_formatter(percent));
                    });
                    ui.checkbox(&mut self.select_contiguous, "Solo la zona tocada")
                        .on_hover_text("Desmarcado, toma ese color en todo el lienzo");
                    ui.checkbox(&mut self.select_all_layers, "Tener en cuenta todas las capas");
                    ui.label("Haz clic sobre un color para seleccionar su zona.");
                } else {
                    ui.label("Arrastra sobre el lienzo para seleccionar. Mayús: cuadrado o círculo.");
                    if self.select_mode == SelectMode::Replace {
                        ui.label("Un clic sin arrastrar quita la selección.");
                    }
                }
                ui.horizontal(|ui| {
                    if ui.small_button("Invertir").on_hover_text("Ctrl+Mayús+I").clicked() {
                        self.doc.invert_selection();
                    }
                    if ui.small_button("Deseleccionar").on_hover_text("Ctrl+D").clicked() {
                        self.doc.deselect();
                    }
                });
                ui.label("Pintar, borrar, rellenar, transformar y voltear afectan solo a lo seleccionado.");
                return;
            }
        }

        let smudging = self.tool == Tool::Difuminar;
        ui.set_width(248.0);
        if !smudging {
            self.brush_sample(ui);
        }
        // What is in hand, and the switch for the settings used less often.
        ui.horizontal(|ui| {
            let source = self.brush_source.filter(|&i| i < self.library.brushes.len());
            let title = match (self.tool, source) {
                (Tool::Pincel, Some(i)) => {
                    let changed = self.brush_index.is_none();
                    format!("{}{}", self.library.brushes[i].name, if changed { " (modificado)" } else { "" })
                }
                (Tool::Pincel, None) => "Pincel sin guardar".to_owned(),
                _ => self.tool.name().to_owned(),
            };
            ui.add(egui::Label::new(egui::RichText::new(title).strong()).truncate());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.toggle_value(&mut self.brush_advanced, egui::RichText::new("⚙").size(16.0)).on_hover_text("Más ajustes: punta, papel, mezcla, presión y guardado");
            });
        });

        // The four settings reached for all the time, each on its own line.
        {
            ui.spacing_mut().slider_width = 176.0;
            let b = self.active_brush();
            let caption = |ui: &mut egui::Ui, text: &str| {
                ui.add_space(2.0);
                ui.label(egui::RichText::new(text).size(12.0));
            };
            caption(ui, "Tamaño");
            ui.add(egui::Slider::new(&mut b.size, 1.0..=MAX_BRUSH_SIZE).logarithmic(true).fixed_decimals(0).suffix(" px"));
            let rows: Vec<(&str, &mut f32, f32)> = if smudging {
                vec![("Intensidad", &mut b.flow, 0.01), ("Dureza", &mut b.hardness, 0.0)]
            } else {
                vec![("Opacidad", &mut b.opacity, 0.01), ("Flujo", &mut b.flow, 0.01), ("Dureza", &mut b.hardness, 0.0)]
            };
            for (label, value, min) in rows {
                caption(ui, label);
                ui.add(egui::Slider::new(value, min..=1.0).custom_formatter(percent));
            }
        }

        if self.brush_advanced {
            ui.add_space(4.0);
            ui.separator();
            ui.spacing_mut().slider_width = 100.0;
            if !smudging {
                let b = self.active_brush();
                egui::Grid::new("ajustes").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
                    ui.label("Espaciado");
                    ui.add(egui::Slider::new(&mut b.spacing, 0.01..=2.0).custom_formatter(percent));
                    ui.end_row();
                    ui.label("Estabilizador");
                    ui.add(egui::Slider::new(&mut b.smoothing, 0.0..=0.95).custom_formatter(percent));
                    ui.end_row();
                });
                ui.checkbox(&mut self.symmetry, "Simetría")
                    .on_hover_text("Repite cada trazo en espejo al otro lado del centro del lienzo");
            }
            egui::CollapsingHeader::new("Presión del lápiz").default_open(true).show(ui, |ui| {
                let b = self.active_brush();
                ui.checkbox(&mut b.pressure_size, "Controla el tamaño");
                ui.add_enabled_ui(b.pressure_size, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Tamaño mínimo");
                        ui.add(egui::Slider::new(&mut b.min_size, 0.0..=1.0).custom_formatter(percent));
                    });
                });
                ui.checkbox(&mut b.pressure_flow, if smudging { "Controla la intensidad" } else { "Controla el flujo" });
            });
            if !smudging {
                egui::CollapsingHeader::new("Punta").default_open(false).show(ui, |ui| self.tip_settings(ui));
                egui::CollapsingHeader::new("Textura de papel").default_open(false).show(ui, |ui| self.paper_settings(ui));
                egui::CollapsingHeader::new("Mezcla de color").default_open(false).show(ui, |ui| self.mix_settings(ui));
            }
            if self.tool == Tool::Pincel {
                egui::CollapsingHeader::new("Guardar en la biblioteca").default_open(false).show(ui, |ui| self.brush_storage(ui));
            }
        }
        if self.tool == Tool::Pincel {
            ui.add_space(4.0);
            ui.separator();
            self.brush_library(ui);
        }
        if self.tool == Tool::Pincel {
            // Remember which brush these settings started from, and whether
            // they still match it.
            if let Some(i) = self.brush_index {
                self.brush_source = Some(i);
            }
            let source = self.brush_source.filter(|&i| i < self.library.brushes.len());
            self.brush_index = source.filter(|&i| self.library.brushes[i].settings == self.brush);
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
        ui.add_enabled_ui(active > 0, |ui| {
            ui.checkbox(&mut props.clip, "Recortar a la capa inferior")
                .on_hover_text("La capa solo se ve donde la capa de debajo tiene dibujo.\nÚtil para sombras y luces sin salirse del color base.");
        });
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
                    // A clipped layer is indented, with a bar tying it to its base.
                    let indent = if props.clip && i > 0 { 12.0 } else { 0.0 };
                    if indent > 0.0 {
                        let x = row.left() + 6.0;
                        painter.line_segment(
                            [Pos2::new(x, row.top() + 4.0), Pos2::new(x, row.bottom() + 2.0)],
                            Stroke::new(2.0, ui.visuals().selection.bg_fill.gamma_multiply(1.6)),
                        );
                    }
                    let slot = Rect::from_min_size(
                        row.min + Vec2::new(4.0 + indent, (ROW_HEIGHT - THUMB_SIZE.1 as f32) * 0.5),
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
        let stale = self.texture.as_ref().is_none_or(|t| t.size() != size) || self.reduced.len() != REDUCED_LEVELS;
        let dirty = if stale {
            self.doc.take_dirty();
            Some(self.doc.bounds())
        } else {
            self.doc.take_dirty()
        };
        if let Some(dirty) = dirty {
            // Whole blocks of the coarsest level, so every level can be rebuilt
            // from the pixels fetched here.
            let block = 1 << REDUCED_LEVELS;
            let area = IRect::new(
                dirty.x0 / block * block,
                dirty.y0 / block * block,
                ((dirty.x1 + block - 1) / block * block).min(size[0] as i32),
                ((dirty.y1 + block - 1) / block * block).min(size[1] as i32),
            );
            let mut pixels = self.doc.render_rgba8(area);
            let (mut w, mut h) = (area.width(), area.height());
            let (mut x, mut y) = (area.x0 as usize, area.y0 as usize);
            let image = ColorImage::from_rgba_unmultiplied([w, h], &pixels);
            match (&mut self.texture, stale) {
                (Some(texture), false) => texture.set_partial([x, y], image, options),
                _ => self.texture = Some(ctx.load_texture("lienzo", image, options)),
            }
            if stale {
                self.reduced.clear();
            }
            for level in 0..REDUCED_LEVELS {
                (pixels, w, h) = halved(&pixels, w, h);
                (x, y) = (x / 2, y / 2);
                let image = ColorImage::from_rgba_unmultiplied([w, h], &pixels);
                match self.reduced.get_mut(level) {
                    Some(texture) => texture.set_partial([x, y], image, TextureOptions::LINEAR),
                    None => self.reduced.push(ctx.load_texture(
                        format!("lienzo_reducido_{level}"),
                        image,
                        TextureOptions::LINEAR,
                    )),
                }
            }
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
        if self.tool == Tool::Pincel {
            self.note_color();
        }
        let brush = self.active_brush().clone();
        let point = StrokePoint::new(at.x, at.y, pressure);
        match self.tool {
            Tool::Difuminar => self.doc.begin_smudge(&brush, point),
            tool => self.doc.begin_stroke(&brush, self.color, tool == Tool::Borrador, point),
        }
    }

    /// Puts an image on a new layer, centred, shrunk to fit if it is larger
    /// than the canvas. `rgba` is 8-bit and not premultiplied.
    fn insert_image(&mut self, name: &str, width: usize, height: usize, rgba: Vec<u8>) {
        if width == 0 || height == 0 || rgba.len() != width * height * 4 {
            self.status = "La imagen no se pudo leer.".into();
            return;
        }
        let fit = (self.doc.width as f64 / width as f64).min(self.doc.height as f64 / height as f64);
        let (mut width, mut height, mut rgba) = (width, height, rgba);
        let mut note = String::new();
        if fit < 1.0 {
            let (w, h) = (
                ((width as f64 * fit).round() as u32).max(1),
                ((height as f64 * fit).round() as u32).max(1),
            );
            let Some(source) = image::RgbaImage::from_raw(width as u32, height as u32, rgba) else { return };
            let scaled = image::imageops::resize(&source, w, h, image::imageops::FilterType::Lanczos3);
            (width, height, rgba) = (w as usize, h as usize, scaled.into_raw());
            note = format!(" (reducida al {:.0} % para caber en el lienzo)", fit * 100.0);
        }
        let x = (self.doc.width as i32 - width as i32) / 2;
        let y = (self.doc.height as i32 - height as i32) / 2;
        self.doc.add_image_layer(name, width, height, &rgba, x, y);
        // The move tool is what comes next: placing the image.
        self.tool = Tool::Mover;
        self.status = format!("Imagen insertada en una capa nueva{note}. Arrástrala para colocarla.");
    }

    fn insert_image_file(&mut self, path: &std::path::Path) {
        match image::open(path) {
            Ok(decoded) => {
                let rgba = decoded.into_rgba8();
                let name = path.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "Imagen".into());
                let (w, h) = (rgba.width() as usize, rgba.height() as usize);
                self.insert_image(&name, w, h, rgba.into_raw());
            }
            Err(e) => self.status = format!("No se pudo abrir {}: {e}", path.display()),
        }
    }

    fn pick_image_file(&mut self) {
        let picked = rfd::FileDialog::new()
            .set_title("Insertar imagen")
            .add_filter("Imágenes", &["png", "jpg", "jpeg", "webp", "bmp", "gif"])
            .pick_file();
        if let Some(path) = picked {
            self.insert_image_file(&path);
        }
    }

    /// Copies the selected part of the active layer (all of it without a
    /// selection) to the clipboard; `cut` also removes it from the layer.
    fn copy(&mut self, cut: bool) {
        self.doc.end_stroke();
        let Some((rect, rgba)) = self.doc.copy_pixels() else {
            self.status = "No hay nada que copiar en la capa activa.".into();
            return;
        };
        // Also offer it to other applications; failing that is not a problem.
        if let Ok(mut clipboard) = arboard::Clipboard::new() {
            let _ = clipboard.set_image(arboard::ImageData {
                width: rect.width(),
                height: rect.height(),
                bytes: std::borrow::Cow::Borrowed(&rgba),
            });
        }
        self.clip = Some((rect, rgba));
        if cut {
            if self.doc.selection().is_some() {
                self.doc.clear_selection_contents();
            } else {
                self.doc.clear_active_layer();
            }
        }
        self.status = if cut { "Cortado." } else { "Copiado." }.into();
    }

    /// Pastes an image that arrived from the clipboard. What SharkPaint itself
    /// copied goes back exactly where it was; anything else is centred.
    fn paste_image(&mut self, width: usize, height: usize, rgba: Vec<u8>) {
        if self.pointer_on_references {
            self.add_reference("Referencia pegada", width, height, rgba);
            return;
        }
        match &self.clip {
            Some((rect, own)) if (rect.width(), rect.height()) == (width, height) => {
                let (rect, own) = (*rect, own.clone());
                self.doc.add_image_layer("Capa pegada", width, height, &own, rect.x0, rect.y0);
                self.tool = Tool::Mover;
                self.status = "Pegado en una capa nueva. Arrástrala para colocarla.".into();
            }
            _ => self.insert_image("Imagen pegada", width, height, rgba),
        }
    }

    /// Pastes whatever image is on the clipboard: the system's, or failing
    /// that the last thing copied here.
    fn paste_clipboard_image(&mut self) {
        let image = arboard::Clipboard::new().and_then(|mut clipboard| clipboard.get_image());
        match (image, &self.clip) {
            (Ok(image), _) => self.paste_image(image.width, image.height, image.bytes.into_owned()),
            (Err(_), Some((rect, own))) => {
                let (w, h, own) = (rect.width(), rect.height(), own.clone());
                self.paste_image(w, h, own);
            }
            (Err(_), None) => self.status = "No hay ninguna imagen en el portapapeles.".into(),
        }
    }

    /// Adds the brushes of a Photoshop brush file to the library and picks the first.
    fn import_abr(&mut self, path: &std::path::Path) {
        let before = self.library.brushes.len();
        match self.library.import_abr(path) {
            Ok(count) => {
                self.status = format!("{count} pinceles importados de «{}».", path.file_name().unwrap_or_default().to_string_lossy());
                // New brushes are at the end; re-imported ones keep their place.
                if let Some(brush) = self.library.brushes.get(before) {
                    self.brush = brush.settings.clone();
                    self.brush_index = Some(before);
                    self.brush_source = Some(before);
                    self.tool = Tool::Pincel;
                    // Show the group that has just arrived.
                    self.brush_category = brush.category.clone();
                    self.brush_search.clear();
                }
                self.library.save();
            }
            Err(e) => self.status = format!("No se pudieron importar los pinceles: {e}"),
        }
    }

    /// Clipboard shortcuts and files dropped on the window.
    fn incoming_images(&mut self, ctx: &egui::Context) {
        let (events, dropped) = ctx.input(|i| (i.events.clone(), i.raw.dropped_files.clone()));
        let typing = ctx.text_edit_focused();
        let pointer = ctx.input(|i| i.pointer.latest_pos());
        self.pointer_on_references = self.refs_rect.zip(pointer).is_some_and(|(rect, pos)| rect.contains(pos));
        for event in events {
            match event {
                Event::Copy if !typing => self.copy(false),
                Event::Cut if !typing => self.copy(true),
                Event::PasteImage(image) => {
                    let rgba: Vec<u8> = image.pixels.iter().flat_map(|c| c.to_srgba_unmultiplied()).collect();
                    self.paste_image(image.size[0], image.size[1], rgba);
                }
                // The clipboard also had text, which wins; look for an image anyway
                // unless the text is headed for a text field.
                Event::Paste(_) if !typing => self.paste_clipboard_image(),
                _ => {}
            }
        }
        for file in dropped {
            let path = file.path().to_path_buf();
            if path.as_os_str().is_empty() {
                continue;
            }
            let on_references = self.pointer_on_references;
            if path.extension().is_some_and(|e| e.eq_ignore_ascii_case(PROJECT_EXTENSION)) {
                self.status = "Para abrir un proyecto usa Archivo → Abrir.".into();
            } else if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("abr")) {
                self.import_abr(&path);
            } else if on_references {
                // Dropped on the reference panel: it is for looking at, not drawing on.
                self.add_reference_file(&path);
            } else {
                self.insert_image_file(&path);
            }
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
        let (space, move_key, rotate_key, alt, events, hover, now) = ctx.input(|i| {
            (
                i.key_down(Key::Space),
                i.key_down(Key::M),
                i.key_down(Key::R),
                i.modifiers.alt,
                i.events.clone(),
                i.pointer.hover_pos(),
                i.time,
            )
        });
        let typing = ctx.text_edit_focused();
        let navigating = !typing && (space || move_key || rotate_key);
        let rotating = rotate_key && !space && !move_key;
        // The pointer is over the canvas itself, not a menu or window on top of it.
        // `contains_pointer` is false while the pointer is on the resize edge of
        // a floating window, which reaches slightly past the window itself.
        let canvas_has_pointer = response.contains_pointer();
        let over_canvas = |pos: Pos2| {
            canvas_has_pointer
                && viewport.contains(pos)
                && ctx.layer_id_at(pos).is_none_or(|l| l == response.layer_id)
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

        self.doc.symmetry = self.symmetry;
        self.sync_transform();
        let free = ctx.input(|i| i.modifiers.shift);
        let mut transform_changed = false;

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
                                    self.select_drag = None;
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
                    if self.tool == Tool::Seleccion && !alt && self.select_shape == SelectShape::Varita {
                        let (x, y) = (c.x.floor() as i32, c.y.floor() as i32);
                        self.doc.select_similar(
                            x,
                            y,
                            self.select_tolerance,
                            self.select_all_layers,
                            self.select_contiguous,
                            self.select_mode,
                        );
                        continue;
                    }
                    if self.tool == Tool::Seleccion && !alt {
                        self.select_drag = Some(SelectDrag { from: c, to: c, trail: vec![c], pressed_at: *pos, moved: false });
                        continue;
                    }
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
                            self.note_color();
                            self.doc.flood_fill(x, y, self.color, self.fill_tolerance, self.fill_all_layers);
                            continue;
                        }
                        Tool::Mover => {
                            self.doc.begin_layer_move();
                            self.move_origin = Some(c);
                            continue;
                        }
                        Tool::Transformar => {
                            if let Some(xform) = &mut self.xform {
                                // A corner scales, inside the box moves, outside rotates.
                                let near = |sx: f32, sy: f32| {
                                    let corner = xform.corner_of(&xform.transform, sx, sy);
                                    (view.to_screen(corner, viewport, doc_size) - *pos).length() < 12.0
                                };
                                let corner = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
                                    .into_iter()
                                    .find(|&(sx, sy)| near(sx, sy));
                                let kind = match corner {
                                    Some((sx, sy)) => TransformDrag::Scale(sx, sy),
                                    None if xform.contains(c) => TransformDrag::Move,
                                    None => TransformDrag::Rotate,
                                };
                                xform.drag = Some((kind, c, xform.transform));
                            }
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
                Event::PointerMoved(pos) if self.xform.as_ref().is_some_and(|x| x.drag.is_some()) => {
                    if let Some(xform) = &mut self.xform {
                        xform.drag_to(to_canvas(*pos), free);
                        transform_changed = true;
                    }
                }
                Event::PointerMoved(_) if self.tool == Tool::Transformar => {}
                Event::PointerMoved(pos) if self.select_drag.is_some() => {
                    if let Some(drag) = &mut self.select_drag {
                        drag.to = to_canvas(*pos);
                        drag.trail.push(drag.to);
                        drag.moved |= (*pos - drag.pressed_at).length() > 3.0;
                    }
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
                    if self.tool == Tool::Transformar {
                        // The transform stays open between drags; only the drag ends.
                        if let Some(xform) = &mut self.xform {
                            xform.drag = None;
                        }
                        continue;
                    }
                    if let Some(drag) = self.select_drag.take() {
                        if drag.moved {
                            let outline = self.select_shape.outline(drag.from, drag.to, &drag.trail, free);
                            self.doc.select_polygon_with(&outline, self.select_mode);
                        } else if self.select_mode == SelectMode::Replace {
                            self.doc.deselect();
                        }
                        continue;
                    }
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

        if transform_changed {
            if let Some(xform) = &self.xform {
                self.doc.set_transform(xform.transform);
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
        // Zoomed out, each screen pixel covers several canvas pixels: draw the
        // reduced copy whose pixels are closest to screen size.
        let on_screen = self.view.zoom * ctx.pixels_per_point();
        let level = match on_screen {
            s if s >= 0.75 => 0,
            s if s >= 0.375 => 1,
            s if s >= 0.19 => 2,
            _ => 3,
        };
        let shown = match level {
            0 => self.texture.as_ref(),
            n => self.reduced.get(n - 1).or(self.texture.as_ref()),
        };
        if let Some(texture) = shown {
            // A reduced copy may be a pixel larger than the canvas it stands for.
            let reach = Vec2::new(
                doc_size.x / (texture.size()[0] << level) as f32,
                doc_size.y / (texture.size()[1] << level) as f32,
            );
            let (reach_x, reach_y) = if std::ptr::eq(texture, self.texture.as_ref().unwrap_or(texture)) {
                (1.0, 1.0)
            } else {
                (reach.x.min(1.0), reach.y.min(1.0))
            };
            let mut mesh = Mesh::with_texture(texture.id());
            for (corner, uv) in
                corners.iter().zip([(0.0, 0.0), (reach_x, 0.0), (reach_x, reach_y), (0.0, reach_y)])
            {
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

        // The symmetry axis, as a guide.
        if self.symmetry {
            let top = self.view.to_screen(Pos2::new(doc_size.x * 0.5, 0.0), viewport, doc_size);
            let bottom = self.view.to_screen(Pos2::new(doc_size.x * 0.5, doc_size.y), viewport, doc_size);
            let guide = Stroke::new(1.0, Color32::from_rgb(111, 227, 240));
            painter.extend(Shape::dashed_line(&[top, bottom], guide, 8.0, 6.0));
        }

        // The selection, and the one being dragged out, as a dashed outline.
        let dashed = |outline: &[(f32, f32)]| {
            let mut points: Vec<Pos2> =
                outline.iter().map(|&(x, y)| self.view.to_screen(Pos2::new(x, y), viewport, doc_size)).collect();
            if let Some(first) = points.first().copied() {
                points.push(first);
            }
            painter.add(Shape::line(points.clone(), Stroke::new(1.0, Color32::WHITE)));
            painter.extend(Shape::dashed_line(&points, Stroke::new(1.0, Color32::BLACK), 5.0, 5.0));
        };
        if let Some(drag) = self.select_drag.as_ref().filter(|d| d.moved) {
            dashed(&self.select_shape.outline(drag.from, drag.to, &drag.trail, free));
        }
        if let Some(selection) = self.doc.selection() {
            // The border as runs along pixel edges; only those in view are drawn.
            let visible = painter.clip_rect().expand(2.0);
            let mut drawn = 0;
            for [a, b] in &selection.edges {
                let a = self.view.to_screen(Pos2::new(a.0, a.1), viewport, doc_size);
                let b = self.view.to_screen(Pos2::new(b.0, b.1), viewport, doc_size);
                if !visible.intersects(Rect::from_two_pos(a, b)) {
                    continue;
                }
                drawn += 1;
                painter.line_segment([a, b], Stroke::new(1.0, Color32::WHITE));
                // Dashes cost more; an intricate border goes without once it is long.
                if drawn < 6000 {
                    painter.extend(Shape::dashed_line(&[a, b], Stroke::new(1.0, Color32::BLACK), 5.0, 5.0));
                }
            }
        }

        // The transform box: outline and corner handles.
        if let Some(xform) = &self.xform {
            let points: Vec<Pos2> = xform.corners().iter().map(|c| self.view.to_screen(*c, viewport, doc_size)).collect();
            painter.add(Shape::closed_line(points.clone(), Stroke::new(3.0, Color32::from_black_alpha(140))));
            painter.add(Shape::closed_line(points.clone(), Stroke::new(1.0, Color32::WHITE)));
            for p in points {
                let handle = Rect::from_center_size(p, Vec2::splat(9.0));
                painter.rect_filled(handle, 1.0, Color32::WHITE);
                painter.rect_stroke(handle, 1.0, Stroke::new(1.0, Color32::BLACK), egui::StrokeKind::Outside);
            }
        }

        // Brush outline in place of the system cursor.
        if let Some(pos) = hover.filter(|p| over_canvas(*p)) {
            if navigating {
                ctx.set_cursor_icon(if rotating { CursorIcon::Alias } else { CursorIcon::Grab });
            } else if alt || matches!(self.tool, Tool::Cuentagotas | Tool::Relleno | Tool::Seleccion) {
                ctx.set_cursor_icon(CursorIcon::Crosshair);
            } else if self.tool == Tool::Mover {
                ctx.set_cursor_icon(CursorIcon::Move);
            } else if self.tool == Tool::Transformar {
                ctx.set_cursor_icon(CursorIcon::Default);
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
            Command::InsertImage => self.pick_image_file(),
            Command::PasteImage => self.paste_clipboard_image(),
            Command::Copy(cut) => self.copy(cut),
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
            Command::ToggleReferences => self.refs_open = !self.refs_open,
            Command::ToggleNavigator => self.nav_open = !self.nav_open,
            Command::ToggleSymmetry => self.symmetry = !self.symmetry,
            Command::Preferences => self.prefs_open = true,
            Command::FinishTransform(apply) => self.finish_transform(apply),
            Command::SelectAll => self.doc.select_all(),
            Command::Deselect => self.doc.deselect(),
            Command::InvertSelection => self.doc.invert_selection(),
            Command::DeleteSelected => self.doc.clear_selection_contents(),
            Command::FlipContents(horizontal) => self.doc.flip(horizontal),
            Command::Fullscreen => {
                let on = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(!on));
            }
            Command::Quit => self.discarding(Discarding::Quit, ctx),
        }
    }
}

impl eframe::App for SharkPaintApp {
    fn on_exit(&mut self) {
        self.store_settings();
        // Closing normally leaves nothing to recover.
        if !self.modified() || self.allow_close {
            self.drop_recovery();
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        if !self.window_hooked {
            self.window_hooked = true;
            crate::dpi_fix::install(frame);
        }
        let ctx = ui.ctx().clone();
        let mut commands = Vec::new();
        self.keyboard_commands(&ctx, &mut commands);
        self.incoming_images(&ctx);

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
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.nav_open, false, egui::RichText::new("Color").strong());
                    ui.selectable_value(&mut self.nav_open, true, egui::RichText::new("Navegador").strong())
                        .on_hover_text("Vista del lienzo completo (F7)");
                });
                ui.add_space(2.0);
                if self.nav_open {
                    self.navigator_panel(ui, &mut commands);
                } else {
                    self.color_panel(ui);
                }
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
        if self.refs_open {
            self.references_window(&ctx);
        } else {
            self.refs_rect = None;
        }
        if self.prefs_open {
            self.preferences_window(&ctx);
        }
        self.recovery_window(&ctx);
        self.autosave(&ctx);
        if !ctx.input(|i| i.pointer.any_down()) {
            self.store_settings();
        }
        self.new_canvas_window(&ctx);
        self.confirm_window(&ctx);
        self.update_window_title(&ctx);
        for command in commands {
            self.run(command, &ctx);
        }
    }
}
