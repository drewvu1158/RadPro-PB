//! Layout studio edits a draft; applying is explicit and never mutates imports.
use crate::{
    alignment::{self, Guides},
    file_dialog,
    template::Template,
    ui_theme as theme,
};
use eframe::egui::{self, Color32, Pos2, Rect, RichText, Sense, Stroke, TextureHandle, Vec2};

pub struct Editor {
    pub draft: Template,
    selected: usize,
    texture: Option<TextureHandle>,
    dirty: bool,
    pub message: String,
    pub import_path: String,
    drag_start: Option<(Pos2, f64, f64)>,
    snap_to_guides: bool,
    guides: Guides,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_drags_include_initial_motion_and_background_drags_do_not_move_boxes() {
        let ctx = egui::Context::default();
        let mut editor = Editor::new(Template::builtin(false));
        let mut time = 0.0;
        let mut frame = |editor: &mut Editor, events| {
            time += 0.05;
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::splat(800.0))),
                time: Some(time),
                events,
                ..Default::default()
            };
            let _ = ctx.run_ui(input, |ui| {
                egui::CentralPanel::default().show(ui, |ui| editor.canvas(ui));
            });
        };
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        frame(&mut editor, vec![]);
        let origin = Pos2::new(60.0, 60.0);
        let end = Pos2::new(73.0, 70.0);
        frame(
            &mut editor,
            vec![egui::Event::PointerMoved(origin), button(origin, true)],
        );
        frame(&mut editor, vec![egui::Event::PointerMoved(end)]);
        frame(&mut editor, vec![button(end, false)]);
        assert!(
            editor.draft.slots[0].x > 50.0,
            "must retain the initial 13-point drag"
        );
        assert!(editor.draft.slots[0].y > 45.0);
        let previous = (editor.draft.slots[0].x, editor.draft.slots[0].y);
        let blank = Pos2::new(10.0, 50.0);
        let blank_end = Pos2::new(20.0, 65.0);
        frame(
            &mut editor,
            vec![egui::Event::PointerMoved(blank), button(blank, true)],
        );
        frame(&mut editor, vec![egui::Event::PointerMoved(blank_end)]);
        frame(&mut editor, vec![button(blank_end, false)]);
        assert_eq!((editor.draft.slots[0].x, editor.draft.slots[0].y), previous);
    }

    #[test]
    fn pointer_drag_shows_guides_and_shift_releases_the_snap() {
        let ctx = egui::Context::default();
        let mut draft = Template::builtin(false);
        draft.slots[0].x = 90.0;
        let mut editor = Editor::new(draft);
        let mut time = 0.0;
        let mut frame = |editor: &mut Editor, mut events: Vec<egui::Event>, shift| {
            time += 0.05;
            events.insert(
                0,
                egui::Event::ModifiersChanged(egui::Modifiers {
                    shift,
                    ..Default::default()
                }),
            );
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::splat(800.0))),
                time: Some(time),
                events,
                ..Default::default()
            };
            let _ = ctx.run_ui(input, |ui| {
                egui::CentralPanel::default().show(ui, |ui| editor.canvas(ui));
            });
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
        frame(&mut editor, vec![], false);
        // In this 800-point panel the 1800px canvas has a 768-point drawing area.
        let scale = 768.0 / 1800.0;
        let origin = Pos2::new(100.0, 60.0);
        let aligned = Pos2::new(100.0 + (32.0 - 90.0) * scale, 60.0);
        frame(
            &mut editor,
            vec![
                egui::Event::PointerMoved(origin),
                button(origin, true, false),
            ],
            false,
        );
        frame(&mut editor, vec![egui::Event::PointerMoved(aligned)], false);
        assert!((editor.draft.slots[0].x - 30.0).abs() < 0.001);
        assert!(editor.guides.vertical.contains(&30.0));
        assert!(editor.guides.vertical.contains(&300.0));
        let free = Pos2::new(100.0 + (40.0 - 90.0) * scale, 60.0);
        frame(&mut editor, vec![egui::Event::PointerMoved(free)], true);
        assert!((editor.draft.slots[0].x - 40.0).abs() < 0.001);
        assert!(editor.guides.vertical.is_empty());
        frame(&mut editor, vec![button(free, false, true)], true);
        assert!(editor.guides.vertical.is_empty());
        assert!(editor.guides.horizontal.is_empty());
        assert!(editor.drag_start.is_none());
    }
}

impl Editor {
    pub fn new(draft: Template) -> Self {
        Self {
            draft,
            selected: 0,
            texture: None,
            dirty: true,
            message: String::new(),
            import_path: String::new(),
            drag_start: None,
            snap_to_guides: true,
            guides: Guides::default(),
        }
    }

    fn replace_draft(&mut self, draft: Template) {
        self.draft = draft;
        self.selected = 0;
        self.dirty = true;
        self.drag_start = None;
        self.guides = Guides::default();
        self.message.clear();
    }

    pub fn import(&mut self, path: &std::path::Path) {
        match Template::load(path) {
            Ok(template) => {
                self.replace_draft(template);
                self.import_path = path.display().to_string();
                self.message =
                    "Imported a copy. Apply to use it in the booth; the source file is unchanged."
                        .into();
            }
            Err(e) => self.message = format!("Import failed: {e}"),
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) -> Option<Template> {
        let mut apply = false;
        ui.heading(RichText::new("Layout studio").size(28.0));
        ui.label("Import a Sparkbooth .sbl, arrange your photos, and make it yours.");
        ui.add_space(12.0);
        ui.horizontal_wrapped(|ui| {
            if ui.button("Import .sbl…").clicked() {
                match file_dialog::choose(false, false) {
                    Ok(Some(path)) => self.import(&path),
                    Ok(None) => {}
                    Err(e) => self.message = e,
                }
            }
            if ui.button("New 2 × 6 strip").clicked() {
                self.replace_draft(Template::builtin(false));
            }
            if ui.button("New double strip").clicked() {
                self.replace_draft(Template::builtin(true));
            }
            if ui.button("New 4 × 6 grid").clicked() {
                self.replace_draft(Template::grid());
            }
            if ui.button("Export .sbl…").clicked() {
                match file_dialog::choose(true, false) {
                    Ok(Some(path)) => {
                        self.message = match self.draft.save(&path) {
                            Ok(()) => format!("Exported {}", path.display()),
                            Err(e) => e,
                        }
                    }
                    Ok(None) => {}
                    Err(e) => self.message = e,
                }
            }
            apply = ui
                .add(
                    egui::Button::new(RichText::new("Apply to booth").color(theme::ON_ACCENT))
                        .fill(theme::ACCENT),
                )
                .clicked();
        });
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.snap_to_guides, "Snap to guides")
                .on_hover_text("Align edges and centers with other photo boxes and the canvas.");
            ui.small("Hold Shift while dragging to move freely.");
        });
        ui.collapsing("Import by file path", |ui| {
            ui.horizontal(|ui| {
                ui.text_edit_singleline(&mut self.import_path);
                if ui.button("Load file").clicked() {
                    let path = self.import_path.clone();
                    self.import(std::path::Path::new(&path));
                }
            });
        });
        if !self.message.is_empty() {
            ui.label(RichText::new(&self.message).color(theme::ACCENT));
        }
        ui.add_space(8.0);
        ui.separator();

        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(Vec2::new(260.0, (ui.available_height()-12.0).max(100.0)), egui::Layout::top_down(egui::Align::LEFT), |ui| {
                egui::ScrollArea::vertical().id_salt("layout_properties").show(ui, |ui| {
                    ui.set_max_width(250.0);
                    ui.label(RichText::new("CANVAS").strong().color(theme::MUTED));
                    ui.text_edit_singleline(&mut self.draft.name);
                    self.dirty |= ui.add(egui::DragValue::new(&mut self.draft.width).range(1..=8192).prefix("Width  ").suffix(" px")).changed();
                    self.dirty |= ui.add(egui::DragValue::new(&mut self.draft.height).range(1..=8192).prefix("Height  ").suffix(" px")).changed();
                    ui.label(format!("{} captures · {} placements", self.draft.shots_needed(), self.draft.slots.len()));
                    ui.add_space(12.0);
                    for overlay in [false, true] {
                        ui.label(RichText::new(if overlay { "OVERLAY" } else { "BACKGROUND" }).strong().color(theme::MUTED));
                        ui.horizontal(|ui| {
                            if ui.button(if overlay { "Choose overlay…" } else { "Choose background…" }).clicked() {
                                match file_dialog::choose(false, true) {
                                    Ok(Some(path)) => match self.draft.set_art(overlay, Some(&path)) { Ok(()) => self.dirty = true, Err(e) => self.message = e },
                                    Ok(None) => {}, Err(e) => self.message = e,
                                }
                            }
                            if ui.button("Clear").clicked() {
                                match self.draft.set_art(overlay, None) { Ok(()) => self.dirty = true, Err(e) => self.message = e }
                            }
                        });
                    }
                    ui.add_space(12.0);
                    ui.label(RichText::new("PHOTO PLACEMENTS").strong().color(theme::MUTED));
                    egui::ComboBox::from_id_salt("placement")
                        .selected_text(format!("Box {}", self.selected + 1))
                        .show_ui(ui, |ui| { for (i,p) in self.draft.slots.iter().enumerate() { ui.selectable_value(&mut self.selected, i, format!("Box {} · shot {}", i + 1, p.photo + 1)); } });
                    if let Some(p) = self.draft.slots.get_mut(self.selected) {
                        let mut shot = p.photo + 1;
                        if ui.add(egui::DragValue::new(&mut shot).range(1..=8).prefix("Shot  ")).changed() { p.photo = shot - 1; self.dirty = true; }
                        ui.small("Use the same shot in multiple boxes for duplicate strips.");
                        for (label, value) in [("X  ",&mut p.x), ("Y  ",&mut p.y), ("Width  ",&mut p.width), ("Height  ",&mut p.height), ("Rotation  ",&mut p.rotation)] {
                            self.dirty |= ui.add(egui::DragValue::new(value).speed(1.0).prefix(label)).changed();
                        }
                    }
                    ui.horizontal_wrapped(|ui| {
                        if ui.add_enabled(self.draft.slots.len() < 64, egui::Button::new("Duplicate box")).clicked() {
                            let mut p = self.draft.slots[self.selected].clone(); p.x += 24.0; p.y += 24.0;
                            self.draft.slots.push(p); self.selected = self.draft.slots.len() - 1; self.dirty = true;
                        }
                        if ui.add_enabled(self.draft.slots.len() > 1, egui::Button::new("Remove box")).clicked() {
                            self.draft.slots.remove(self.selected); self.selected = self.selected.min(self.draft.slots.len()-1); self.dirty = true;
                        }
                        if ui.add_enabled(self.selected > 0, egui::Button::new("Layer down")).clicked() {
                            self.draft.slots.swap(self.selected, self.selected - 1); self.selected -= 1; self.dirty = true;
                        }
                        if ui.add_enabled(self.selected + 1 < self.draft.slots.len(), egui::Button::new("Layer up")).clicked() {
                            self.draft.slots.swap(self.selected, self.selected + 1); self.selected += 1; self.dirty = true;
                        }
                    });
                    ui.add_space(12.0);
                    ui.small("Drag a photo box on the canvas to move it. Change its size or angle above. Artwork fills the canvas. Import supports v2 photo boxes and embedded backgrounds/overlays; separate text/image layers require flattening first.");
                });
            });
            ui.separator();
            ui.vertical(|ui| { self.canvas(ui); });
        });
        if apply {
            match self.draft.validate() {
                Ok(()) => return Some(self.draft.clone()),
                Err(e) => self.message = e,
            }
        }
        None
    }

    fn canvas(&mut self, ui: &mut egui::Ui) {
        if self.dirty {
            match self.draft.render(&[], Some(1000)) {
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
                    ui.colored_label(Color32::LIGHT_RED, e);
                    return;
                }
            }
        }
        let available = ui.available_size() - Vec2::splat(16.0);
        let scale = (available.x / self.draft.width as f32)
            .min(available.y / self.draft.height as f32)
            .max(0.001);
        let size = Vec2::new(
            self.draft.width as f32 * scale,
            self.draft.height as f32 * scale,
        );
        let (rect, response) = ui.allocate_exact_size(size, Sense::click_and_drag());
        if let Some(t) = &self.texture {
            ui.painter().image(
                t.id(),
                rect,
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        if response.clicked() || response.drag_started() {
            let pointer = if response.drag_started() {
                ui.input(|i| i.pointer.press_origin())
            } else {
                response.interact_pointer_pos()
            };
            self.drag_start = None;
            if let Some(pointer) = pointer {
                if let Some((index, _)) =
                    self.draft.slots.iter().enumerate().rev().find(|(_, p)| {
                        p.contains(
                            ((pointer.x - rect.min.x) / scale) as f64,
                            ((pointer.y - rect.min.y) / scale) as f64,
                        )
                    })
                {
                    self.selected = index;
                    if response.drag_started() {
                        let slot = &self.draft.slots[index];
                        self.drag_start = Some((pointer, slot.x, slot.y));
                    }
                }
            }
        }
        if response.dragged() || response.drag_stopped() {
            if let (Some((origin, x, y)), Some(pointer)) =
                (self.drag_start, response.interact_pointer_pos())
            {
                let requested = (
                    x + ((pointer.x - origin.x) / scale) as f64,
                    y + ((pointer.y - origin.y) / scale) as f64,
                );
                let snapped = alignment::align_drag(
                    &self.draft,
                    self.selected,
                    requested,
                    scale as f64,
                    self.snap_to_guides && !ui.input(|i| i.modifiers.shift),
                );
                let slot = &mut self.draft.slots[self.selected];
                slot.x = snapped.x;
                slot.y = snapped.y;
                self.guides = snapped.guides;
                self.dirty = true;
            }
        }
        if response.drag_stopped() {
            self.drag_start = None;
            self.guides = Guides::default();
        }
        for (i, p) in self.draft.slots.iter().enumerate() {
            let corners: Vec<_> = p
                .corners()
                .iter()
                .map(|(x, y)| {
                    rect.min + Vec2::new((*x * scale as f64) as f32, (*y * scale as f64) as f32)
                })
                .collect();
            let color = if i == self.selected {
                theme::ACCENT
            } else {
                theme::MUTED
            };
            ui.painter().add(egui::Shape::closed_line(
                corners.clone(),
                Stroke::new(if i == self.selected { 2.5 } else { 1.0 }, color),
            ));
            ui.painter().text(
                corners[0] + Vec2::splat(6.0),
                egui::Align2::LEFT_TOP,
                format!("{}", p.photo + 1),
                egui::FontId::proportional(20.0),
                Color32::WHITE,
            );
        }
        let painter = ui.painter_at(rect);
        let bounds = self.draft.slots[self.selected].bounds();
        for x in &self.guides.vertical {
            let px = rect.min.x + (*x * scale as f64) as f32;
            painter.extend(egui::Shape::dashed_line(
                &[Pos2::new(px, rect.min.y), Pos2::new(px, rect.max.y)],
                Stroke::new(1.5, theme::ACCENT),
                6.0,
                4.0,
            ));
            for y in bounds.y_anchors() {
                painter.circle_filled(
                    Pos2::new(px, rect.min.y + (y * scale as f64) as f32),
                    3.0,
                    theme::ACCENT,
                );
            }
        }
        for y in &self.guides.horizontal {
            let py = rect.min.y + (*y * scale as f64) as f32;
            painter.extend(egui::Shape::dashed_line(
                &[Pos2::new(rect.min.x, py), Pos2::new(rect.max.x, py)],
                Stroke::new(1.5, theme::ACCENT),
                6.0,
                4.0,
            ));
            for x in bounds.x_anchors() {
                painter.circle_filled(
                    Pos2::new(rect.min.x + (x * scale as f64) as f32, py),
                    3.0,
                    theme::ACCENT,
                );
            }
        }
    }
}
