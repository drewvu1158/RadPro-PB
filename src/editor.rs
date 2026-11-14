//! Draft editing, canvas interactions, history, and the studio's three panes.
use crate::{
    alignment::{self, Guides},
    design::*,
    file_dialog, presets,
    template::{self, Template},
    ui_theme as theme,
};
use eframe::egui::{self, Color32, Pos2, Rect, RichText, Sense, Stroke, TextureHandle, Vec2};
use std::time::{Duration, Instant};

#[derive(Clone)]
struct Snapshot {
    draft: Template,
    selected: Option<usize>,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Layers,
    Templates,
    Canvas,
}
#[derive(Clone, Copy)]
enum DragMode {
    Move,
    Resize(usize),
    Rotate,
}
struct Drag {
    pointer: Pos2,
    frame: Frame,
    mode: DragMode,
}
pub struct Editor {
    pub draft: Template,
    selected: Option<usize>,
    texture: Option<TextureHandle>,
    dirty: bool,
    pub message: String,
    pub import_path: String,
    drag_start: Option<Drag>,
    snap_to_guides: bool,
    guides: Guides,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    last_edit: Option<(egui::Id, Instant)>,
    tab: Tab,
    zoom: f32,
    show_grid: bool,
    show_bounds: bool,
    gallery: Vec<Option<TextureHandle>>,
}
impl Editor {
    pub fn new(draft: Template) -> Self {
        Self {
            draft,
            selected: Some(0),
            texture: None,
            dirty: true,
            message: String::new(),
            import_path: String::new(),
            drag_start: None,
            snap_to_guides: true,
            guides: Guides::default(),
            undo: vec![],
            redo: vec![],
            last_edit: None,
            tab: Tab::Layers,
            zoom: 1.0,
            show_grid: false,
            show_bounds: true,
            gallery: (0..presets::PRESETS.len()).map(|_| None).collect(),
        }
    }
    fn snapshot(&self) -> Snapshot {
        Snapshot {
            draft: self.draft.clone(),
            selected: self.selected,
        }
    }
    fn record(&mut self, before: Snapshot, group: Option<egui::Id>) {
        let coalesced = group.is_some()
            && self.last_edit.is_some_and(|(id, time)| {
                Some(id) == group && time.elapsed() < Duration::from_secs(1)
            });
        if !coalesced {
            self.undo.push(before);
            if self.undo.len() > 40 {
                self.undo.remove(0);
            }
        }
        self.redo.clear();
        self.last_edit = group.map(|id| (id, Instant::now()));
        self.dirty = true;
    }
    fn restore(&mut self, s: Snapshot) {
        self.draft = s.draft;
        self.selected = s.selected;
        self.dirty = true;
        self.drag_start = None;
        self.guides = Guides::default();
        self.last_edit = None;
    }
    fn undo(&mut self) {
        if let Some(s) = self.undo.pop() {
            self.redo.push(self.snapshot());
            self.restore(s);
        }
    }
    fn redo(&mut self) {
        if let Some(s) = self.redo.pop() {
            self.undo.push(self.snapshot());
            self.restore(s);
        }
    }
    fn replace_draft(&mut self, draft: Template) {
        let before = self.snapshot();
        self.restore(Snapshot {
            draft,
            selected: Some(0),
        });
        self.record(before, None);
        self.message.clear();
    }
    pub fn import(&mut self, path: &std::path::Path) {
        match Template::load(path) {
            Ok(t) => {
                self.replace_draft(t);
                self.import_path = path.display().to_string();
                self.message = "Imported a copy. Apply to use it in the booth.".into();
            }
            Err(e) => self.message = format!("Import failed: {e}"),
        }
    }
    fn add_layer(&mut self, layer: Layer) -> bool {
        self.insert_layer(layer, self.draft.layers.len())
    }
    fn insert_layer(&mut self, layer: Layer, index: usize) -> bool {
        let before = self.snapshot();
        self.draft.layers.insert(index, layer);
        if let Err(e) = self.draft.validate() {
            self.draft = before.draft;
            self.message = e;
            return false;
        }
        self.selected = Some(index);
        self.tab = Tab::Layers;
        self.record(before, None);
        true
    }
    fn add_image(&mut self, background: bool) {
        match file_dialog::choose(false, true) {
            Ok(Some(path)) => match template::load_image(&path) {
                Ok(asset) => {
                    let w = self.draft.width as f64;
                    let h = self.draft.height as f64;
                    let frame = if background {
                        Frame::new(0.0, 0.0, w, h)
                    } else {
                        let iw = w * 0.55;
                        let ih = (iw * asset.pixels.height() as f64 / asset.pixels.width() as f64)
                            .min(h * 0.6);
                        Frame::new(w * 0.2, h * 0.2, iw, ih)
                    };
                    let layer = Layer::new(
                        path.file_stem()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .chars()
                            .take(100)
                            .collect::<String>(),
                        frame,
                        LayerKind::Image {
                            asset,
                            fit: if background {
                                ImageFit::Cover
                            } else {
                                ImageFit::Contain
                            },
                            flip_x: false,
                            flip_y: false,
                        },
                    );
                    self.insert_layer(
                        layer,
                        if background {
                            0
                        } else {
                            self.draft.layers.len()
                        },
                    );
                }
                Err(e) => self.message = e,
            },
            Ok(None) => {}
            Err(e) => self.message = e,
        }
    }
    fn duplicate(&mut self) {
        if let Some(i) = self.selected {
            if self.draft.layers[i].locked {
                return;
            }
            let mut l = self.draft.layers[i].clone();
            l.frame.x += 24.0;
            l.frame.y += 24.0;
            l.name = format!("{} copy", l.name.chars().take(100).collect::<String>());
            self.add_layer(l);
        }
    }
    fn can_delete(&self) -> bool {
        self.selected.is_some_and(|i| {
            !self.draft.layers[i].locked
                && (!self.draft.layers[i].is_photo() || self.draft.photo_count() > 1)
        })
    }
    fn delete(&mut self) {
        if self.can_delete() {
            let before = self.snapshot();
            let i = self.selected.unwrap();
            self.draft.layers.remove(i);
            self.selected = Some(i.min(self.draft.layers.len() - 1));
            self.record(before, None);
        }
    }
    fn reorder(&mut self, to: usize) {
        if let Some(i) = self.selected {
            if self.draft.layers[i].locked || i == to {
                return;
            }
            let before = self.snapshot();
            let l = self.draft.layers.remove(i);
            self.draft.layers.insert(to, l);
            self.selected = Some(to);
            self.record(before, None);
        }
    }
    fn align(&mut self, axis: usize, anchor: usize) {
        if let Some(i) = self.selected {
            if self.draft.layers[i].locked {
                return;
            }
            let before = self.snapshot();
            let f = &mut self.draft.layers[i].frame;
            let b = f.bounds();
            if axis == 0 {
                f.x += [0.0, self.draft.width as f64 / 2.0, self.draft.width as f64][anchor]
                    - b.x_anchors()[anchor];
            } else {
                f.y += [
                    0.0,
                    self.draft.height as f64 / 2.0,
                    self.draft.height as f64,
                ][anchor]
                    - b.y_anchors()[anchor];
            }
            self.record(before, None);
        }
    }
    fn save_dialog(&mut self) {
        match file_dialog::choose(true, false) {
            Ok(Some(path)) => {
                self.message = match self.draft.save(&path) {
                    Ok(()) => format!("Saved editable design: {}", path.display()),
                    Err(e) => e,
                }
            }
            Ok(None) => {}
            Err(e) => self.message = e,
        }
    }
    fn shortcuts(&mut self, ui: &egui::Ui) {
        if ui.ctx().egui_wants_keyboard_input() {
            return;
        }
        let keys = ui.input(|i| {
            (
                i.modifiers.command,
                i.modifiers.shift,
                i.key_pressed(egui::Key::Z),
                i.key_pressed(egui::Key::Y),
                i.key_pressed(egui::Key::D),
                i.key_pressed(egui::Key::Delete),
                i.key_pressed(egui::Key::ArrowLeft),
                i.key_pressed(egui::Key::ArrowRight),
                i.key_pressed(egui::Key::ArrowUp),
                i.key_pressed(egui::Key::ArrowDown),
                i.key_pressed(egui::Key::S),
            )
        });
        if keys.0 && keys.2 {
            if keys.1 {
                self.redo();
            } else {
                self.undo();
            }
        } else if keys.0 && keys.3 {
            self.redo();
        } else if keys.0 && keys.4 {
            self.duplicate();
        } else if keys.0 && keys.10 {
            self.save_dialog();
        } else if keys.5 {
            self.delete();
        } else if let Some(i) = self.selected {
            let dx = i32::from(keys.7) - i32::from(keys.6);
            let dy = i32::from(keys.9) - i32::from(keys.8);
            if (dx != 0 || dy != 0) && !self.draft.layers[i].locked {
                let before = self.snapshot();
                let step = if keys.1 { 10.0 } else { 1.0 };
                self.draft.layers[i].frame.x += dx as f64 * step;
                self.draft.layers[i].frame.y += dy as f64 * step;
                self.record(before, Some(egui::Id::unique("nudge")));
            }
        }
    }
    pub fn ui(&mut self, ui: &mut egui::Ui) -> Option<Template> {
        self.shortcuts(ui);
        let mut apply = false;
        ui.horizontal(|ui| {
            ui.heading(RichText::new("Layout studio").font(theme::editorial(29.0)));
            ui.label(RichText::new("MAKE IT YOURS").small().color(theme::MUTED));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                apply = ui
                    .add(
                        egui::Button::new(RichText::new("Apply to booth").color(theme::ON_ACCENT))
                            .fill(theme::ACCENT),
                    )
                    .clicked();
                if ui.button("Save .sbl…").clicked() {
                    self.save_dialog();
                }
                if ui.button("Open .sbl…").clicked() {
                    match file_dialog::choose(false, false) {
                        Ok(Some(p)) => self.import(&p),
                        Ok(None) => {}
                        Err(e) => self.message = e,
                    }
                }
            });
        });
        ui.horizontal_wrapped(|ui| {
            if ui.button("Templates").clicked() {
                self.tab = Tab::Templates;
            }
            ui.separator();
            if ui.button("+ Photo").clicked() {
                let shot = self.draft.shots_needed().min(7);
                self.add_layer(Layer::photo(
                    Frame::new(
                        60.0,
                        60.0,
                        self.draft.width as f64 * 0.6,
                        self.draft.width as f64 * 0.45,
                    ),
                    shot,
                ));
            }
            if ui.button("+ Text").clicked() {
                self.add_layer(Layer::text(
                    "Your text here",
                    Frame::new(
                        40.0,
                        70.0,
                        (self.draft.width as f64 - 80.0).max(40.0),
                        160.0,
                    ),
                ));
            }
            if ui.button("+ Image").clicked() {
                self.add_image(false);
            }
            ui.menu_button("+ Shape", |ui| {
                for shape in [
                    ShapeKind::Rectangle,
                    ShapeKind::Ellipse,
                    ShapeKind::Heart,
                    ShapeKind::Star,
                    ShapeKind::Diamond,
                ] {
                    if ui.button(format!("{shape:?}")).clicked() {
                        self.add_layer(Layer::shape(
                            shape,
                            Frame::new(60.0, 60.0, 180.0, 180.0),
                            [210, 154, 124, 255],
                        ));
                        ui.close();
                    }
                }
            });
            ui.separator();
            if ui
                .add_enabled(!self.undo.is_empty(), egui::Button::new("Undo"))
                .on_hover_text("Ctrl+Z")
                .clicked()
            {
                self.undo();
            }
            if ui
                .add_enabled(!self.redo.is_empty(), egui::Button::new("Redo"))
                .on_hover_text("Ctrl+Y")
                .clicked()
            {
                self.redo();
            }
        });
        if !self.message.is_empty() {
            ui.horizontal_wrapped(|ui| {
                ui.colored_label(theme::ACCENT, &self.message);
                if ui.small_button("Dismiss").clicked() {
                    self.message.clear();
                }
            });
        }
        ui.separator();
        let height = (ui.available_height() - 30.0).max(160.0);
        let left = 220.0;
        let right = 255.0;
        let canvas_width = (ui.available_width() - left - right - 44.0).max(160.0);
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(
                Vec2::new(left, height),
                egui::Layout::top_down(egui::Align::LEFT),
                |ui| {
                    ui.set_width(left);
                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                    ui.spacing_mut().button_padding = Vec2::new(7.0, 6.0);
                    ui.horizontal(|ui| {
                        ui.selectable_value(
                            &mut self.tab,
                            Tab::Layers,
                            RichText::new("Layers").small(),
                        );
                        ui.selectable_value(
                            &mut self.tab,
                            Tab::Templates,
                            RichText::new("Presets").small(),
                        );
                        ui.selectable_value(
                            &mut self.tab,
                            Tab::Canvas,
                            RichText::new("Canvas").small(),
                        );
                    });
                    ui.separator();
                    egui::ScrollArea::vertical()
                        .id_salt("studio_left")
                        .max_height(height - 45.0)
                        .show(ui, |ui| {
                            ui.set_width(left - 12.0);
                            match self.tab {
                                Tab::Layers => self.layers_ui(ui),
                                Tab::Templates => self.gallery_ui(ui),
                                Tab::Canvas => self.background_ui(ui),
                            }
                        });
                },
            );
            ui.separator();
            ui.allocate_ui_with_layout(
                Vec2::new(canvas_width, height),
                egui::Layout::top_down(egui::Align::LEFT),
                |ui| {
                    ui.set_width(canvas_width);
                    ui.horizontal(|ui| {
                        ui.small(format!("{} × {} px", self.draft.width, self.draft.height));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.small_button("Fit").clicked() {
                                self.zoom = 1.0;
                            }
                            ui.add(egui::Slider::new(&mut self.zoom, 0.5..=3.0).show_value(false))
                                .on_hover_text("Canvas zoom");
                        });
                    });
                    egui::ScrollArea::both()
                        .id_salt("studio_canvas")
                        .auto_shrink([false, false])
                        .max_height(height - 90.0)
                        .show(ui, |ui| {
                            let available =
                                (ui.available_size() - Vec2::splat(16.0)).max(Vec2::splat(20.0));
                            let fit = (available.x / self.draft.width as f32)
                                .min(available.y / self.draft.height as f32)
                                * self.zoom;
                            let margin =
                                ((available.x - self.draft.width as f32 * fit) / 2.0).max(0.0);
                            ui.horizontal_top(|ui| {
                                ui.add_space(margin);
                                self.canvas(ui);
                            });
                        });
                    ui.horizontal_wrapped(|ui| {
                        ui.checkbox(&mut self.snap_to_guides, "Snap");
                        ui.checkbox(&mut self.show_grid, "Grid");
                        ui.checkbox(&mut self.show_bounds, "Outlines");
                    });
                    ui.small(
                        "Drag to move · corners resize · circle rotates · Shift bypasses snap",
                    );
                },
            );
            ui.separator();
            ui.allocate_ui_with_layout(
                Vec2::new(right, height),
                egui::Layout::top_down(egui::Align::LEFT),
                |ui| {
                    ui.set_width(right);
                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                    egui::ScrollArea::vertical()
                        .id_salt("studio_inspector")
                        .max_height(height)
                        .show(ui, |ui| {
                            ui.set_width(right - 12.0);
                            ui.spacing_mut().button_padding = Vec2::new(8.0, 6.0);
                            self.inspector(ui);
                        });
                },
            );
        });
        if apply {
            match self.draft.validate() {
                Ok(()) => return Some(self.draft.clone()),
                Err(e) => self.message = e,
            }
        }
        None
    }
    fn layers_ui(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("LAYER STACK").small().color(theme::MUTED));
        ui.small("Front at top. Select any layer to edit.");
        let before = self.snapshot();
        let mut changed = false;
        for i in (0..self.draft.layers.len()).rev() {
            ui.push_id(i, |ui| {
                let l = &mut self.draft.layers[i];
                egui::Frame::new()
                    .fill(if self.selected == Some(i) {
                        theme::SURFACE_HOVER
                    } else {
                        theme::SURFACE
                    })
                    .corner_radius(6)
                    .inner_margin(6)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            changed |= ui
                                .checkbox(&mut l.visible, "")
                                .on_hover_text("Show / hide layer")
                                .changed();
                            let label = format!(
                                "{}  {}",
                                match l.kind {
                                    LayerKind::Text { .. } => "T",
                                    LayerKind::Photo { .. } => "P",
                                    LayerKind::Image { .. } => "I",
                                    LayerKind::Shape { .. } => "S",
                                },
                                l.name.chars().take(16).collect::<String>()
                            );
                            if ui
                                .selectable_label(self.selected == Some(i), label)
                                .clicked()
                            {
                                self.selected = Some(i);
                            }
                        });
                        ui.horizontal(|ui| {
                            ui.small(l.kind_name());
                            changed |= ui.checkbox(&mut l.locked, "Lock").changed();
                        });
                    });
            });
        }
        if changed {
            self.record(before, None);
        } else if self.selected != before.selected {
            self.last_edit = None;
        }
        ui.add_space(8.0);
        if let Some(i) = self.selected {
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(
                        i + 1 < self.draft.layers.len(),
                        egui::Button::new("Forward"),
                    )
                    .clicked()
                {
                    self.reorder(i + 1);
                }
                if ui
                    .add_enabled(i > 0, egui::Button::new("Backward"))
                    .clicked()
                {
                    self.reorder(i - 1);
                }
                if ui.button("To front").clicked() {
                    self.reorder(self.draft.layers.len() - 1);
                }
                if ui.button("To back").clicked() {
                    self.reorder(0);
                }
            });
        }
        ui.separator();
        ui.small("Ctrl+D duplicate · Delete removes\nArrows nudge · Shift+arrows 10 px\nCtrl+Z undo · Ctrl+Y redo · Ctrl+S save");
    }
    fn gallery_ui(&mut self, ui: &mut egui::Ui) {
        ui.label(
            RichText::new("START WITH A DESIGN")
                .small()
                .color(theme::MUTED),
        );
        ui.small("Every element is yours to edit. Undo restores your previous design.");
        ui.horizontal_wrapped(|ui| {
            if ui.small_button("Blank strip").clicked() {
                self.replace_draft(Template::builtin(false));
            }
            if ui.small_button("Double strip").clicked() {
                self.replace_draft(Template::builtin(true));
            }
            if ui.small_button("Blank grid").clicked() {
                self.replace_draft(Template::grid());
            }
        });
        for (i, (name, subtitle)) in presets::PRESETS.iter().enumerate() {
            if self.gallery[i].is_none() {
                if let Ok(image) = presets::build(i).render(&[], Some(220)) {
                    let rgba = image.to_rgba8();
                    self.gallery[i] = Some(ui.ctx().load_texture(
                        format!("preset-{i}"),
                        egui::ColorImage::from_rgba_unmultiplied(
                            [rgba.width() as usize, rgba.height() as usize],
                            rgba.as_raw(),
                        ),
                        egui::TextureOptions::LINEAR,
                    ));
                }
            }
            let mut choose = false;
            theme::card().inner_margin(10).show(ui, |ui| {
                if let Some(t) = &self.gallery[i] {
                    let fit = Vec2::new(155.0, 140.0);
                    let ratio = (fit.x / t.size()[0] as f32).min(fit.y / t.size()[1] as f32);
                    choose |= ui
                        .add(
                            egui::Image::new((
                                t.id(),
                                Vec2::new(t.size()[0] as f32 * ratio, t.size()[1] as f32 * ratio),
                            ))
                            .sense(Sense::click()),
                        )
                        .clicked();
                }
                ui.label(RichText::new(*name).strong());
                ui.small(*subtitle);
                choose |= ui.button("Use template").clicked();
            });
            ui.add_space(5.0);
            if choose {
                self.replace_draft(presets::build(i));
                self.tab = Tab::Layers;
            }
        }
    }
    fn background_ui(&mut self, ui: &mut egui::Ui) {
        let before = self.snapshot();
        let mut changed = false;
        ui.label(RichText::new("CANVAS").small().color(theme::MUTED));
        changed |= ui
            .add(
                egui::TextEdit::singleline(&mut self.draft.name)
                    .char_limit(100)
                    .desired_width(185.0),
            )
            .changed();
        changed |= ui
            .add(
                egui::DragValue::new(&mut self.draft.width)
                    .range(1..=8192)
                    .prefix("Width  ")
                    .suffix(" px"),
            )
            .changed();
        changed |= ui
            .add(
                egui::DragValue::new(&mut self.draft.height)
                    .range(1..=8192)
                    .prefix("Height  ")
                    .suffix(" px"),
            )
            .changed();
        ui.add_space(10.0);
        ui.label(RichText::new("BASE BACKGROUND").small().color(theme::MUTED));
        egui::ComboBox::from_id_salt("background_style")
            .selected_text(format!("{:?}", self.draft.background.style))
            .show_ui(ui, |ui| {
                for style in [
                    BackgroundStyle::Solid,
                    BackgroundStyle::Gradient,
                    BackgroundStyle::Radial,
                    BackgroundStyle::Dots,
                    BackgroundStyle::Checker,
                ] {
                    changed |= ui
                        .selectable_value(
                            &mut self.draft.background.style,
                            style,
                            format!("{style:?}"),
                        )
                        .changed();
                }
            });
        changed |= color_picker(ui, "Base color", &mut self.draft.background.color, false);
        if self.draft.background.style != BackgroundStyle::Solid {
            changed |= color_picker(
                ui,
                "Second color",
                &mut self.draft.background.secondary,
                false,
            );
        }
        if self.draft.background.style == BackgroundStyle::Gradient {
            changed |= ui
                .add(egui::Slider::new(&mut self.draft.background.angle, 0.0..=360.0).text("Angle"))
                .changed();
        }
        if matches!(
            self.draft.background.style,
            BackgroundStyle::Dots | BackgroundStyle::Checker
        ) {
            changed |= ui
                .add(
                    egui::DragValue::new(&mut self.draft.background.spacing)
                        .range(4.0..=1000.0)
                        .prefix("Pattern size  "),
                )
                .changed();
        }
        ui.add_space(8.0);
        ui.label("Color palettes");
        for (name, colors) in presets::PALETTES {
            ui.horizontal(|ui| {
                let r = ui.add_sized(
                    [106.0, 24.0],
                    egui::Button::new(RichText::new(name).small()),
                );
                if r.clicked() {
                    self.draft.background.color = colors[0];
                    self.draft.background.secondary = colors[1];
                    changed = true;
                }
                for color in colors {
                    let (rect, _) = ui.allocate_exact_size(Vec2::splat(13.0), Sense::hover());
                    ui.painter().rect_filled(
                        rect,
                        3,
                        Color32::from_rgba_unmultiplied(color[0], color[1], color[2], color[3]),
                    );
                }
            });
        }
        if changed {
            let group = ui.ctx().memory(|m| m.focused());
            self.record(before, group);
        }
        ui.add_space(8.0);
        if ui.button("Background image…").clicked() {
            self.add_image(true);
        }
        ui.small("Background images appear in Layers, where you can resize, replace, hide, or remove them.");
        ui.collapsing("Open a file path", |ui| {
            ui.text_edit_singleline(&mut self.import_path);
            if ui.button("Load .sbl").clicked() {
                self.import(&std::path::PathBuf::from(self.import_path.clone()));
            }
        });
    }
    fn inspector(&mut self, ui: &mut egui::Ui) {
        let Some(i) = self.selected else {
            ui.heading("Select an element");
            ui.label(
                "Click a layer or an element on the canvas. Use Canvas to change the background.",
            );
            return;
        };
        let before = self.snapshot();
        let mut changed = false;
        let l = &mut self.draft.layers[i];
        ui.label(
            RichText::new(format!("{} PROPERTIES", l.kind_name()))
                .small()
                .color(theme::MUTED),
        );
        changed |= ui
            .add(
                egui::TextEdit::singleline(&mut l.name)
                    .char_limit(100)
                    .desired_width(230.0),
            )
            .changed();
        ui.horizontal(|ui| {
            changed |= ui.checkbox(&mut l.visible, "Visible").changed();
            changed |= ui.checkbox(&mut l.locked, "Locked").changed();
        });
        ui.add_enabled_ui(!l.locked, |ui| {
            ui.separator();
            ui.label("Position & size");
            egui::Grid::new("layer_geometry")
                .num_columns(2)
                .spacing([8.0, 6.0])
                .show(ui, |ui| {
                    for (label, value, range) in [
                        ("X", &mut l.frame.x, -16384.0..=16384.0),
                        ("Y", &mut l.frame.y, -16384.0..=16384.0),
                        ("Width", &mut l.frame.width, 1.0..=8192.0),
                        ("Height", &mut l.frame.height, 1.0..=8192.0),
                        ("Rotation", &mut l.frame.rotation, -360.0..=360.0),
                    ] {
                        ui.label(label);
                        changed |= ui
                            .add(
                                egui::DragValue::new(value)
                                    .range(range)
                                    .speed(1.0)
                                    .suffix(if label == "Rotation" { "°" } else { " px" }),
                            )
                            .changed();
                        ui.end_row();
                    }
                });
            ui.separator();
            match &mut l.kind {
                LayerKind::Photo { shot, .. } => {
                    let mut n = *shot + 1;
                    if ui
                        .add(
                            egui::DragValue::new(&mut n)
                                .range(1..=8)
                                .prefix("Capture  "),
                        )
                        .changed()
                    {
                        *shot = n - 1;
                        changed = true;
                    }
                    ui.small(
                        "Reuse a capture number for duplicate prints. Photos use crop-to-fill.",
                    );
                }
                LayerKind::Text {
                    text,
                    family,
                    size,
                    color,
                    align,
                    bold,
                    italic,
                    spacing,
                    line_height,
                } => {
                    ui.label("Text content");
                    changed |= ui
                        .add(
                            egui::TextEdit::multiline(text)
                                .char_limit(1000)
                                .desired_width(230.0)
                                .desired_rows(3),
                        )
                        .changed();
                    egui::ComboBox::from_id_salt("text_family")
                        .selected_text(format!("{family:?}"))
                        .show_ui(ui, |ui| {
                            for f in [FontFamily::Sans, FontFamily::Serif, FontFamily::Mono] {
                                changed |=
                                    ui.selectable_value(family, f, format!("{f:?}")).changed();
                            }
                        });
                    changed |= ui
                        .add(
                            egui::DragValue::new(size)
                                .range(1.0..=1024.0)
                                .prefix("Font size  ")
                                .suffix(" px"),
                        )
                        .changed();
                    ui.horizontal(|ui| {
                        changed |= ui.checkbox(bold, "Bold").changed();
                        changed |= ui.checkbox(italic, "Italic").changed();
                    });
                    ui.horizontal(|ui| {
                        for a in [TextAlign::Left, TextAlign::Center, TextAlign::Right] {
                            changed |= ui.selectable_value(align, a, format!("{a:?}")).changed();
                        }
                    });
                    changed |= color_picker(ui, "Text color", color, true);
                    changed |= ui
                        .add(
                            egui::DragValue::new(spacing)
                                .range(-50.0..=200.0)
                                .speed(0.2)
                                .prefix("Tracking  "),
                        )
                        .changed();
                    changed |= ui
                        .add(
                            egui::DragValue::new(line_height)
                                .range(0.5..=3.0)
                                .speed(0.05)
                                .prefix("Line height  "),
                        )
                        .changed();
                    ui.small("Text wraps to the box. Increase its height for more lines.");
                }
                LayerKind::Image {
                    fit,
                    flip_x,
                    flip_y,
                    ..
                } => {
                    ui.label("Image fitting");
                    egui::ComboBox::from_id_salt("image_fit")
                        .selected_text(format!("{fit:?}"))
                        .show_ui(ui, |ui| {
                            for f in [ImageFit::Contain, ImageFit::Cover, ImageFit::Stretch] {
                                changed |= ui.selectable_value(fit, f, format!("{f:?}")).changed();
                            }
                        });
                    ui.horizontal(|ui| {
                        changed |= ui.checkbox(flip_x, "Flip X").changed();
                        changed |= ui.checkbox(flip_y, "Flip Y").changed();
                    });
                }
                LayerKind::Shape { shape, color } => {
                    egui::ComboBox::from_id_salt("shape_kind")
                        .selected_text(format!("{shape:?}"))
                        .show_ui(ui, |ui| {
                            for f in [
                                ShapeKind::Rectangle,
                                ShapeKind::Ellipse,
                                ShapeKind::Heart,
                                ShapeKind::Star,
                                ShapeKind::Diamond,
                            ] {
                                changed |=
                                    ui.selectable_value(shape, f, format!("{f:?}")).changed();
                            }
                        });
                    changed |= color_picker(ui, "Fill", color, true);
                }
            }
            if !l.is_photo() {
                ui.separator();
                changed |= ui
                    .add(egui::Slider::new(&mut l.opacity, 0.0..=1.0).text("Opacity"))
                    .changed();
                if !matches!(l.kind, LayerKind::Text { .. }) {
                    changed |= ui
                        .add(
                            egui::DragValue::new(&mut l.rounding)
                                .range(0.0..=4096.0)
                                .prefix("Corner radius  "),
                        )
                        .changed();
                }
                changed |= ui
                    .add(
                        egui::DragValue::new(&mut l.border)
                            .range(0.0..=1024.0)
                            .prefix("Border  ")
                            .suffix(" px"),
                    )
                    .changed();
                if l.border > 0.0 {
                    changed |= color_picker(ui, "Border color", &mut l.border_color, true);
                }
            }
        });
        if changed {
            let group = ui.ctx().memory(|m| m.focused());
            self.record(before, group);
        }
        if matches!(self.draft.layers[i].kind, LayerKind::Image { .. })
            && !self.draft.layers[i].locked
            && ui.button("Replace image…").clicked()
        {
            match file_dialog::choose(false, true) {
                Ok(Some(p)) => match template::load_image(&p) {
                    Ok(asset) => {
                        let before = self.snapshot();
                        if let LayerKind::Image { asset: a, .. } = &mut self.draft.layers[i].kind {
                            *a = asset;
                        }
                        if let Err(e) = self.draft.validate() {
                            self.draft = before.draft;
                            self.message = e;
                        } else {
                            self.record(before, None);
                        }
                    }
                    Err(e) => self.message = e,
                },
                Ok(None) => {}
                Err(e) => self.message = e,
            }
        }
        ui.add_space(8.0);
        ui.separator();
        ui.label("Align to canvas");
        ui.add_enabled_ui(!self.draft.layers[i].locked, |ui| {
            ui.horizontal(|ui| {
                for (a, label) in ["Left", "Center", "Right"].iter().enumerate() {
                    if ui.button(*label).clicked() {
                        self.align(0, a);
                    }
                }
            });
            ui.horizontal(|ui| {
                for (a, label) in ["Top", "Middle", "Bottom"].iter().enumerate() {
                    if ui.button(*label).clicked() {
                        self.align(1, a);
                    }
                }
            });
            ui.horizontal(|ui| {
                if ui.button("Duplicate").clicked() {
                    self.duplicate();
                }
                if ui
                    .add_enabled(self.can_delete(), egui::Button::new("Delete"))
                    .clicked()
                {
                    self.delete();
                }
            });
        });
    }
    fn canvas(&mut self, ui: &mut egui::Ui) {
        if self.dirty {
            match self.draft.render(&[], Some(1100)) {
                Ok(image) => {
                    let rgba = image.to_rgba8();
                    let pixels = egui::ColorImage::from_rgba_unmultiplied(
                        [rgba.width() as usize, rgba.height() as usize],
                        rgba.as_raw(),
                    );
                    match &mut self.texture {
                        Some(t) => t.set(pixels, egui::TextureOptions::LINEAR),
                        None => {
                            self.texture = Some(ui.ctx().load_texture(
                                "layout-studio",
                                pixels,
                                egui::TextureOptions::LINEAR,
                            ))
                        }
                    }
                    self.dirty = false;
                }
                Err(e) => {
                    ui.colored_label(Color32::LIGHT_RED, &e);
                    ui.small("Correct the canvas or layer values in the inspector.");
                    return;
                }
            }
        }
        let available = (ui.available_size() - Vec2::splat(16.0)).max(Vec2::splat(20.0));
        let scale = (available.x / self.draft.width as f32)
            .min(available.y / self.draft.height as f32)
            .max(0.001)
            * self.zoom;
        let size = Vec2::new(
            self.draft.width as f32 * scale,
            self.draft.height as f32 * scale,
        );
        let (rect, response) = ui.allocate_exact_size(size, Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        if let Some(t) = &self.texture {
            painter.image(
                t.id(),
                rect,
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        let screen = |p: (f64, f64)| {
            rect.min + Vec2::new((p.0 * scale as f64) as f32, (p.1 * scale as f64) as f32)
        };
        if self.show_grid {
            let step = (50.0 * scale).max(8.0);
            let mut x = rect.left();
            while x < rect.right() {
                painter.line_segment(
                    [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
                    Stroke::new(0.5, Color32::from_black_alpha(35)),
                );
                x += step;
            }
            let mut y = rect.top();
            while y < rect.bottom() {
                painter.line_segment(
                    [Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)],
                    Stroke::new(0.5, Color32::from_black_alpha(35)),
                );
                y += step;
            }
        }
        if response.clicked() || response.drag_started() {
            let pointer = if response.drag_started() {
                ui.input(|i| i.pointer.press_origin())
            } else {
                response.interact_pointer_pos()
            };
            self.drag_start = None;
            if let Some(pointer) = pointer {
                let mut picked = None;
                if self.show_bounds {
                    if let Some(i) = self.selected {
                        let l = &self.draft.layers[i];
                        if l.visible && !l.locked {
                            let corners = l.frame.corners().map(screen);
                            for (k, c) in corners.iter().enumerate() {
                                if c.distance(pointer) < 10.0 {
                                    picked = Some((i, DragMode::Resize(k)));
                                }
                            }
                            let rotation = rotation_handle(corners, rect);
                            if rotation.distance(pointer) < 10.0 {
                                picked = Some((i, DragMode::Rotate));
                            }
                        }
                    }
                }
                if picked.is_none() {
                    let x = ((pointer.x - rect.min.x) / scale) as f64;
                    let y = ((pointer.y - rect.min.y) / scale) as f64;
                    picked = self
                        .draft
                        .layers
                        .iter()
                        .enumerate()
                        .rev()
                        .find(|(_, l)| l.visible && !l.locked && l.frame.contains(x, y))
                        .map(|(i, _)| (i, DragMode::Move));
                }
                let selection = picked.map(|(i, _)| i);
                if selection != self.selected {
                    self.last_edit = None;
                }
                self.selected = selection;
                if let Some((i, mode)) = picked {
                    if response.drag_started() {
                        self.record(self.snapshot(), None);
                        self.drag_start = Some(Drag {
                            pointer,
                            frame: self.draft.layers[i].frame,
                            mode,
                        });
                    }
                }
            }
        }
        if response.dragged() || response.drag_stopped() {
            if let (Some(drag), Some(pointer), Some(i)) = (
                &self.drag_start,
                response.interact_pointer_pos(),
                self.selected,
            ) {
                let mut frame = drag.frame;
                let dx = ((pointer.x - drag.pointer.x) / scale) as f64;
                let dy = ((pointer.y - drag.pointer.y) / scale) as f64;
                match drag.mode {
                    DragMode::Move => {
                        let aligned = alignment::align_drag(
                            &self.draft,
                            i,
                            (frame.x + dx, frame.y + dy),
                            scale as f64,
                            self.snap_to_guides && !ui.input(|i| i.modifiers.shift),
                        );
                        frame.x = aligned.x;
                        frame.y = aligned.y;
                        self.guides = aligned.guides;
                    }
                    DragMode::Resize(corner) => {
                        let (s, c) = frame.rotation.to_radians().sin_cos();
                        let (lx, ly) = (c * dx + s * dy, -s * dx + c * dy);
                        let left = corner == 0 || corner == 3;
                        let top = corner == 0 || corner == 1;
                        let old = frame;
                        frame.width =
                            (frame.width + if left { -lx } else { lx }).clamp(1.0, 8192.0);
                        frame.height =
                            (frame.height + if top { -ly } else { ly }).clamp(1.0, 8192.0);
                        if ui.input(|i| i.modifiers.shift) {
                            frame.height =
                                (frame.width * old.height / old.width).clamp(1.0, 8192.0);
                        }
                        let ox = if left { old.width - frame.width } else { 0.0 };
                        let oy = if top { old.height - frame.height } else { 0.0 };
                        frame.x += c * ox - s * oy;
                        frame.y += s * ox + c * oy;
                    }
                    DragMode::Rotate => {
                        let origin = screen((frame.x, frame.y));
                        let a = (drag.pointer.y - origin.y).atan2(drag.pointer.x - origin.x);
                        let b = (pointer.y - origin.y).atan2(pointer.x - origin.x);
                        frame.rotation += (b - a).to_degrees() as f64;
                        if ui.input(|i| i.modifiers.shift) {
                            frame.rotation = (frame.rotation / 15.0).round() * 15.0;
                        }
                    }
                }
                if frame.validate().is_ok() {
                    self.draft.layers[i].frame = frame;
                    self.dirty = true;
                }
            }
        }
        if response.drag_stopped() {
            self.drag_start = None;
            self.guides = Guides::default();
        }
        if self.show_bounds {
            for (i, l) in self.draft.layers.iter().enumerate() {
                if !l.visible {
                    continue;
                }
                let corners = l.frame.corners().map(screen);
                let selected = self.selected == Some(i);
                if selected || l.is_photo() {
                    painter.add(egui::Shape::closed_line(
                        corners.to_vec(),
                        Stroke::new(
                            if selected { 2.0 } else { 0.7 },
                            if selected {
                                theme::ACCENT
                            } else {
                                Color32::from_white_alpha(100)
                            },
                        ),
                    ));
                }
                if let LayerKind::Photo { shot, .. } = l.kind {
                    painter.text(
                        corners[0] + Vec2::splat(7.0),
                        egui::Align2::LEFT_TOP,
                        format!("{}", shot + 1),
                        egui::FontId::proportional(17.0),
                        Color32::WHITE,
                    );
                }
                if selected && !l.locked {
                    for corner in corners {
                        painter.rect_filled(
                            Rect::from_center_size(corner, Vec2::splat(8.0)),
                            1,
                            theme::TEXT,
                        );
                    }
                    let top = (corners[0] + corners[1].to_vec2()) / 2.0;
                    let rotation = rotation_handle(corners, rect);
                    painter.line_segment([top, rotation], Stroke::new(1.0, theme::ACCENT));
                    painter.circle_filled(rotation, 5.0, theme::ACCENT);
                }
            }
        }
        if let Some(i) = self.selected {
            let b = self.draft.layers[i].frame.bounds();
            for x in &self.guides.vertical {
                let px = screen((*x, 0.0)).x;
                painter.extend(egui::Shape::dashed_line(
                    &[Pos2::new(px, rect.top()), Pos2::new(px, rect.bottom())],
                    Stroke::new(1.5, theme::ACCENT),
                    6.0,
                    4.0,
                ));
                for y in b.y_anchors() {
                    painter.circle_filled(screen((*x, y)), 3.0, theme::ACCENT);
                }
            }
            for y in &self.guides.horizontal {
                let py = screen((0.0, *y)).y;
                painter.extend(egui::Shape::dashed_line(
                    &[Pos2::new(rect.left(), py), Pos2::new(rect.right(), py)],
                    Stroke::new(1.5, theme::ACCENT),
                    6.0,
                    4.0,
                ));
                for x in b.x_anchors() {
                    painter.circle_filled(screen((x, *y)), 3.0, theme::ACCENT);
                }
            }
        }
    }
}
fn rotation_handle(corners: [Pos2; 4], canvas: Rect) -> Pos2 {
    let top = corners[0] + (corners[1] - corners[0]) * 0.5;
    let direction = (corners[1] - corners[0]).normalized();
    (top + Vec2::new(direction.y, -direction.x) * 24.0)
        .clamp(canvas.min + Vec2::splat(7.0), canvas.max - Vec2::splat(7.0))
}
fn color_picker(ui: &mut egui::Ui, label: &str, color: &mut Color, alpha: bool) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label(label);
        let mut c = Color32::from_rgba_unmultiplied(color[0], color[1], color[2], color[3]);
        let response = egui::color_picker::color_edit_button_srgba(
            ui,
            &mut c,
            if alpha {
                egui::color_picker::Alpha::OnlyBlend
            } else {
                egui::color_picker::Alpha::Opaque
            },
        );
        if response.changed() {
            *color = c.to_srgba_unmultiplied();
            changed = true;
        }
    });
    ui.horizontal_wrapped(|ui| {
        for (_, palette) in presets::PALETTES.iter().take(4) {
            for value in [palette[1], palette[3]] {
                if ui
                    .add_sized(
                        [18.0, 18.0],
                        egui::Button::new("").fill(Color32::from_rgb(value[0], value[1], value[2])),
                    )
                    .clicked()
                {
                    *color = value;
                    changed = true;
                }
            }
        }
    });
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn studio_controls_fit_normal_and_small_windows() {
        for size in [Vec2::new(1280.0, 700.0), Vec2::new(760.0, 380.0)] {
            let ctx = egui::Context::default();
            theme::install(&ctx);
            let mut editor = Editor::new(presets::build(0));
            editor.selected = Some(editor.draft.layers.len() - 1);
            for tab in [Tab::Layers, Tab::Templates, Tab::Canvas] {
                editor.tab = tab;
                let _ = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                        ..Default::default()
                    },
                    |ui| {
                        egui::CentralPanel::default().show(ui, |ui| {
                            editor.ui(ui);
                            assert!(
                                ui.min_rect().right() <= size.x + 1.0,
                                "studio overflow: {} > {}",
                                ui.min_rect().right(),
                                size.x
                            );
                        });
                    },
                );
            }
        }
    }
    #[test]
    fn history_restores_layers_and_presets_without_losing_images() {
        let mut e = Editor::new(Template::builtin(false));
        e.add_layer(Layer::text("hello", Frame::new(40.0, 50.0, 300.0, 100.0)));
        let count = e.draft.layers.len();
        e.replace_draft(presets::build(3));
        assert_ne!(e.draft.layers.len(), count);
        e.undo();
        assert_eq!(e.draft.layers.len(), count);
        assert!(
            matches!(&e.draft.layers.last().unwrap().kind,LayerKind::Text{text,..} if text=="hello")
        );
        e.undo();
        assert_eq!(e.draft.layers.len(), 4);
        e.redo();
        assert_eq!(e.draft.layers.len(), count);
    }
    #[test]
    fn locked_layers_and_last_photo_are_protected() {
        let mut e = Editor::new(Template::builtin(false));
        e.draft.layers.truncate(1);
        assert!(!e.can_delete());
        e.add_layer(Layer::text("hello", Frame::new(0.0, 0.0, 300.0, 100.0)));
        e.draft.layers[1].locked = true;
        e.duplicate();
        e.delete();
        e.reorder(0);
        assert_eq!(e.draft.layers.len(), 2);
        assert_eq!(e.selected, Some(1));
    }
    #[test]
    fn pointer_drag_includes_initial_motion_shows_guides_and_shift_bypasses() {
        let ctx = egui::Context::default();
        let mut t = Template::builtin(false);
        t.layers[0].frame.x = 90.0;
        let mut e = Editor::new(t);
        let mut time = 0.0;
        let mut frame = |e: &mut Editor, mut events: Vec<egui::Event>, shift| {
            time += 0.05;
            events.insert(
                0,
                egui::Event::ModifiersChanged(egui::Modifiers {
                    shift,
                    ..Default::default()
                }),
            );
            let _ = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::splat(800.0))),
                    time: Some(time),
                    events,
                    ..Default::default()
                },
                |ui| {
                    egui::CentralPanel::default().show(ui, |ui| e.canvas(ui));
                },
            );
        };
        let button = |pos, pressed, shift| egui::Event::PointerButton {
            pos,
            pressed,
            button: egui::PointerButton::Primary,
            modifiers: egui::Modifiers {
                shift,
                ..Default::default()
            },
        };
        frame(&mut e, vec![], false);
        let scale = 768.0 / 1800.0;
        let origin = Pos2::new(100.0, 60.0);
        let aligned = Pos2::new(100.0 + (32.0 - 90.0) * scale, 60.0);
        frame(
            &mut e,
            vec![
                egui::Event::PointerMoved(origin),
                button(origin, true, false),
            ],
            false,
        );
        frame(&mut e, vec![egui::Event::PointerMoved(aligned)], false);
        assert!((e.draft.layers[0].frame.x - 30.0).abs() < 0.001);
        assert!(e.guides.vertical.contains(&30.0));
        let free = Pos2::new(100.0 + (40.0 - 90.0) * scale, 60.0);
        frame(&mut e, vec![egui::Event::PointerMoved(free)], true);
        assert!((e.draft.layers[0].frame.x - 40.0).abs() < 0.001);
        assert!(e.guides.vertical.is_empty());
        frame(&mut e, vec![button(free, false, true)], true);
        assert!(e.drag_start.is_none());
        e.undo();
        assert_eq!(e.draft.layers[0].frame.x, 90.0);
        e.draft.layers[0].frame.x = 30.0;
        e.dirty = true;
        frame(&mut e, vec![], false);
        let old = e.draft.layers[0].frame;
        let blank = Pos2::new(10.0, 300.0);
        let blank_end = Pos2::new(20.0, 310.0);
        frame(
            &mut e,
            vec![egui::Event::PointerMoved(blank), button(blank, true, false)],
            false,
        );
        frame(&mut e, vec![egui::Event::PointerMoved(blank_end)], false);
        frame(&mut e, vec![button(blank_end, false, false)], false);
        assert_eq!(
            e.draft.layers[0].frame, old,
            "background drags must not move the selection"
        );
        e.selected = Some(0);
        frame(&mut e, vec![], false);
        let corner = Pos2::new(
            8.0 + (old.x + old.width) as f32 * scale,
            8.0 + (old.y + old.height) as f32 * scale,
        );
        let resize = corner + Vec2::new(25.0, 20.0);
        frame(
            &mut e,
            vec![
                egui::Event::PointerMoved(corner),
                button(corner, true, false),
            ],
            false,
        );
        frame(&mut e, vec![egui::Event::PointerMoved(resize)], false);
        frame(&mut e, vec![button(resize, false, false)], false);
        assert!(
            e.draft.layers[0].frame.width > old.width + 40.0,
            "corner must resize width"
        );
        assert!(
            e.draft.layers[0].frame.height > old.height + 30.0,
            "corner must resize height"
        );
        e.undo();
        assert_eq!(e.draft.layers[0].frame, old);
        frame(&mut e, vec![], false);
        let screen = |p: (f64, f64)| Pos2::new(8.0 + p.0 as f32 * scale, 8.0 + p.1 as f32 * scale);
        let rect = Rect::from_min_size(
            Pos2::new(8.0, 8.0),
            Vec2::new(600.0 * scale, 1800.0 * scale),
        );
        let handle = rotation_handle(old.corners().map(screen), rect);
        let rotate = handle + Vec2::new(20.0, 45.0);
        frame(
            &mut e,
            vec![
                egui::Event::PointerMoved(handle),
                button(handle, true, false),
            ],
            false,
        );
        frame(&mut e, vec![egui::Event::PointerMoved(rotate)], false);
        frame(&mut e, vec![button(rotate, false, false)], false);
        assert!(
            e.draft.layers[0].frame.rotation.abs() > 10.0,
            "round handle must rotate"
        );
    }
}
