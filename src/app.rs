//! eframe UI: preview, countdown overlay, capture, and review screens.

use crate::booth::{Booth, BoothEvent, Phase};
use crate::camera::{CameraClient, CaptureResult};
use crate::layout::{self, LayoutKind, LayoutOptions};
use crate::library_ui::{LibraryAction, LibraryView};
use crate::settings::{default_settings_path, Settings};
use crate::storage;
use crate::ui_theme::{self as theme, HeaderAction};
use crate::{
    editor::Editor,
    printing::{self, PrintOutcome},
    template::Template,
};
use eframe::egui;
use egui::load::SizedTexture;
use egui::{Color32, RichText, TextureHandle, Vec2};
use image::DynamicImage;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

/// Upload one captured shot as a filmstrip/review thumbnail. The camera frame
/// is already 640×480, so `thumbnail` is a cheap no-op in the common case.
fn shot_texture(ctx: &egui::Context, index: usize, shot: &DynamicImage) -> TextureHandle {
    let rgba = shot.thumbnail(640, 480).to_rgba8();
    ctx.load_texture(
        format!("session-shot-{index}"),
        egui::ColorImage::from_rgba_unmultiplied(
            [rgba.width() as usize, rgba.height() as usize],
            rgba.as_raw(),
        ),
        egui::TextureOptions::LINEAR,
    )
}

#[derive(Clone, Copy)]
enum PhotoReorder {
    Move,
    Swap,
}

#[derive(Clone, Copy)]
struct DraggedPhoto {
    index: usize,
}

/// Live video covers the viewport with a centered crop; captured photos fit in full.
/// Cropping UVs rather than changing the image proportions keeps faces undistorted.
fn preview_placement(area: egui::Rect, source: Vec2, live: bool) -> (egui::Rect, egui::Rect) {
    if live {
        let scale = (area.width() / source.x).max(area.height() / source.y);
        let visible = (area.size() / (source * scale)).min(Vec2::splat(1.0));
        let crop = (Vec2::splat(1.0) - visible) * 0.5;
        (
            area,
            egui::Rect::from_min_max(
                egui::pos2(1.0 - crop.x, crop.y),
                egui::pos2(crop.x, 1.0 - crop.y),
            ),
        )
    } else {
        let size = source * (area.width() / source.x).min(area.height() / source.y);
        (
            egui::Rect::from_center_size(area.center(), size),
            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
        )
    }
}

pub struct SparkboothApp {
    settings: Settings,
    settings_path: PathBuf,
    show_settings: bool,
    show_library: bool,
    library: LibraryView,
    library_folder_input: String,
    camera: CameraClient,
    booth: Booth,
    shots: Vec<DynamicImage>,
    preview_texture: Option<TextureHandle>,
    preview_id: u64,
    review_texture: Option<TextureHandle>,
    capture_rx: Option<mpsc::Receiver<CaptureResult>>,
    capture_error: Option<String>,
    status: String,
    camera_note: Option<String>,
    camera_names: Vec<String>,
    camera_active: Option<String>,
    last_strip: Option<PathBuf>,
    template: Option<Template>,
    template_error: Option<String>,
    editor: Editor,
    show_editor: bool,
    session_template: Option<Template>,
    print_image: Option<DynamicImage>,
    shot_textures: Vec<TextureHandle>,
    review_selection: usize,
    selected_capture: Option<usize>,
    last_capture: Option<usize>,
    print_feedback: Option<String>,
    print_copies: u16,
}

impl SparkboothApp {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        simulate: bool,
        settings_path: Option<PathBuf>,
    ) -> Self {
        Self::from_context(&cc.egui_ctx, simulate, settings_path)
    }

    fn from_context(ctx: &egui::Context, simulate: bool, settings_path: Option<PathBuf>) -> Self {
        let settings_path = settings_path.unwrap_or_else(default_settings_path);
        let mut status = String::new();
        let settings = match Settings::load(&settings_path) {
            Ok(s) => s,
            Err(e) => {
                status = format!("settings not loaded ({e}); using defaults");
                Settings::default()
            }
        };
        let camera = CameraClient::start(settings.camera_index, simulate || settings.simulate);
        let (template, template_error) = match &settings.template_path {
            Some(path) => match Template::load(path) {
                Ok(t) => (Some(t), None),
                Err(e) => (
                    None,
                    Some(format!(
                        "Could not load selected layout: {e}. Choose a layout in Layout studio."
                    )),
                ),
            },
            None => (None, None),
        };
        let booth = Booth::new(
            template
                .as_ref()
                .map(Template::shots_needed)
                .unwrap_or(settings.shots),
            settings.countdown_seconds,
        );
        let editor = Editor::new(template.clone().unwrap_or_else(|| Template::builtin(false)));
        theme::install(ctx);
        Self {
            library_folder_input: settings.layout_library_dir.to_string_lossy().into_owned(),
            settings,
            settings_path,
            show_settings: false,
            show_library: false,
            library: LibraryView::default(),
            camera,
            booth,
            shots: Vec::new(),
            preview_texture: None,
            preview_id: 0,
            review_texture: None,
            capture_rx: None,
            capture_error: None,
            status,
            camera_note: None,
            camera_names: Vec::new(),
            camera_active: None,
            last_strip: None,
            template,
            template_error,
            editor,
            show_editor: false,
            session_template: None,
            print_image: None,
            shot_textures: Vec::new(),
            review_selection: 0,
            selected_capture: None,
            last_capture: None,
            print_feedback: None,
            print_copies: 1,
        }
    }

    fn set_fullscreen(&mut self, ctx: &egui::Context, enabled: bool) {
        if enabled {
            self.show_settings = false;
            self.show_editor = false;
            self.show_library = false;
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(enabled));
    }

    fn reload_booth_if_idle(&mut self) {
        if self.booth.phase() == Phase::Preview && self.shots.is_empty() {
            self.booth = Booth::new(
                self.template
                    .as_ref()
                    .map(Template::shots_needed)
                    .unwrap_or(self.settings.shots),
                self.settings.countdown_seconds,
            );
        }
    }

    fn save_settings(&mut self) -> bool {
        let saved = match self.settings.save(&self.settings_path) {
            Ok(()) => {
                self.status = format!("settings saved to {}", self.settings_path.display());
                true
            }
            Err(e) => {
                self.status = format!("could not save settings: {e}");
                false
            }
        };
        self.reload_booth_if_idle();
        saved
    }

    /// Re-open the camera with the current settings, without restarting.
    fn reconnect_camera(&mut self) {
        self.save_settings();
        self.camera
            .reconnect(self.settings.camera_index, self.settings.simulate);
        self.camera_note = None;
        self.status = "reconnecting camera…".to_string();
    }

    fn update_camera_texture(&mut self, ctx: &egui::Context) {
        // The worker already downscales; hand over an `Arc` so this is a
        // pointer clone, not a frame copy.
        let (id, frame, note, names, active) = match self.camera.latest.lock() {
            Ok(state) => (
                state.id,
                state.image.clone(),
                state.note.clone(),
                state.cameras.clone(),
                state.active.clone(),
            ),
            Err(_) => return,
        };
        self.camera_note = note;
        if names != self.camera_names {
            self.camera_names = names;
        }
        if active != self.camera_active {
            self.camera_active = active;
        }
        if id == self.preview_id {
            return;
        }
        if let Some(rgb) = frame {
            let image = egui::ColorImage::from_rgb(
                [rgb.width() as usize, rgb.height() as usize],
                rgb.as_raw(),
            );
            match &mut self.preview_texture {
                Some(tex) => tex.set(image, egui::TextureOptions::LINEAR),
                None => {
                    self.preview_texture = Some(ctx.load_texture(
                        "camera-preview",
                        image,
                        egui::TextureOptions::LINEAR,
                    ))
                }
            }
            self.preview_id = id;
        }
    }

    fn drain_capture(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.capture_rx else {
            return;
        };
        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => {
                Err("Camera worker disconnected. Please ask the host to restart the camera.".into())
            }
        };
        self.capture_rx = None;
        match result {
            Ok(rgba) => self.accept_capture(ctx, DynamicImage::ImageRgba8(rgba)),
            Err(error) => {
                self.capture_error = Some(error);
                self.booth.capture_failed();
            }
        }
    }

    /// Apply a completed capture to its original destination. Images and their
    /// thumbnail projections are changed together, only after capture succeeds.
    fn accept_capture(&mut self, ctx: &egui::Context, photo: DynamicImage) {
        let index = self.booth.capture_index() as usize;
        if self.booth.phase() != Phase::Shooting || index > self.shots.len() {
            return;
        }
        let texture = shot_texture(ctx, index, &photo);
        if index == self.shots.len() {
            self.shots.push(photo);
            self.shot_textures.push(texture);
        } else {
            self.shots[index] = photo;
            self.shot_textures[index] = texture;
        }
        self.selected_capture = None;
        self.last_capture = Some(index);
        if self.booth.deliver_shot() == BoothEvent::StripComplete {
            self.compose_and_save(ctx);
        }
    }

    fn request_retake(&mut self, index: usize) {
        if index < self.shots.len() && self.booth.request_retake(index as u32) {
            self.selected_capture = Some(index);
            self.review_selection = index + 1;
            self.capture_error = None;
        }
    }

    /// The photo and its thumbnail have one order; composition and printing
    /// consume that same order. Moving is only allowed on the review screen.
    fn reorder_photos(
        &mut self,
        ctx: &egui::Context,
        operation: PhotoReorder,
        from: usize,
        to: usize,
    ) -> bool {
        if self.booth.phase() != Phase::Review
            || from == to
            || from >= self.shots.len()
            || to >= self.shots.len()
        {
            return false;
        }
        match operation {
            PhotoReorder::Move => {
                let photo = self.shots.remove(from);
                self.shots.insert(to, photo);
                let texture = self.shot_textures.remove(from);
                self.shot_textures.insert(to, texture);
            }
            PhotoReorder::Swap => {
                self.shots.swap(from, to);
                self.shot_textures.swap(from, to);
            }
        }
        self.last_capture = None;
        self.compose_and_save(ctx);
        self.review_selection = to + 1;
        true
    }

    fn layout_options(&self) -> LayoutOptions {
        LayoutOptions {
            cell: (640, 480),
            margin: 36,
            gap: 14,
            background: if self.settings.light_border {
                [255, 255, 255, 255]
            } else {
                [16, 16, 18, 255]
            },
        }
    }

    fn compose_and_save(&mut self, ctx: &egui::Context) {
        self.last_strip = None;
        self.print_image = None;
        self.review_texture = None;
        self.review_selection = 0;
        self.print_feedback = None;
        let opts = self.layout_options();
        let composed = match &self.session_template {
            Some(template) => template.render(&self.shots, None),
            None => {
                layout::compose(&self.shots, self.settings.layout, &opts).map_err(|e| e.to_string())
            }
        };
        match composed {
            Ok(strip) => {
                match storage::save_strip(&strip, &self.settings.output_dir) {
                    Ok(path) => {
                        self.last_strip = Some(path.clone());
                        self.status = format!("saved {}", path.display());
                    }
                    Err(e) => self.status = format!("could not save strip: {e}"),
                }
                if self.settings.save_individual_shots {
                    for (index, shot) in self.shots.iter().enumerate() {
                        if let Err(e) =
                            storage::save_shot(shot, &self.settings.output_dir, index as u32)
                        {
                            self.status = format!("could not save shot {}: {e}", index + 1);
                        }
                    }
                }
                let rgba = strip.to_rgba8();
                let image = egui::ColorImage::from_rgba_unmultiplied(
                    [rgba.width() as usize, rgba.height() as usize],
                    rgba.as_raw(),
                );
                self.review_texture =
                    Some(ctx.load_texture("review-strip", image, egui::TextureOptions::LINEAR));
                self.print_image = Some(strip);
                debug_assert_eq!(
                    self.shot_textures.len(),
                    self.shots.len(),
                    "one filmstrip thumbnail per captured shot"
                );
            }
            Err(e) => {
                self.status = format!("could not compose strip: {e}");
                self.capture_error = Some(format!(
                    "Could not prepare the layout: {e}. Your photos are still here."
                ));
            }
        }
    }

    fn reset_session(&mut self) {
        self.booth.finish();
        self.shots.clear();
        self.review_texture = None;
        self.capture_rx = None;
        self.capture_error = None;
        self.print_image = None;
        self.shot_textures.clear();
        self.review_selection = 0;
        self.selected_capture = None;
        self.last_capture = None;
        self.print_feedback = None;
        self.print_copies = 1;
        self.last_strip = None;
        self.session_template = None;
        self.status = "Ready for the next session.".into();
        self.reload_booth_if_idle();
    }

    fn request_picture(&mut self) {
        if self.booth.phase() == Phase::Preview {
            self.session_template = self.template.clone();
            self.last_strip = None;
            self.show_settings = false;
            self.show_editor = false;
            self.shots.clear();
            self.shot_textures.clear();
        }
        self.capture_error = None;
        if let Some(index) = self.selected_capture {
            self.request_retake(index);
        } else {
            self.booth.request_shot();
        }
    }

    /// Keep the guest action centered independently of the small host popout.
    fn top_controls(&mut self, ui: &mut egui::Ui, fullscreen: bool) -> HeaderAction {
        let width = ui.available_width();
        let (row, _) = ui.allocate_exact_size(Vec2::new(width, 46.0), egui::Sense::hover());
        let menu_rect = egui::Rect::from_min_size(
            egui::pos2(row.right() - 64.0, row.top()),
            Vec2::new(64.0, 46.0),
        );
        let mut menu_ui = ui.new_child(
            egui::UiBuilder::new()
                .id_salt("host-popout")
                .max_rect(menu_rect)
                .layout(egui::Layout::top_down(egui::Align::Center)),
        );
        let action = theme::operator_menu(
            &mut menu_ui,
            self.booth.phase() == Phase::Preview,
            self.show_editor,
            fullscreen,
        );
        if self.booth.phase() != Phase::Preview && !self.show_editor && !self.show_library {
            let mut cancel_ui = ui.new_child(
                egui::UiBuilder::new()
                    .id_salt("cancel-set")
                    .max_rect(egui::Rect::from_min_size(row.min, Vec2::new(130.0, 46.0)))
                    .layout(egui::Layout::top_down(egui::Align::Min)),
            );
            if cancel_ui
                .add(egui::Button::new("Cancel set").min_size(Vec2::new(120.0, 46.0)))
                .on_hover_text("Start over with a new set of photos")
                .clicked()
            {
                self.reset_session();
            }
        }
        if self.booth.phase() != Phase::Review && !self.show_editor && !self.show_library {
            let label = match self.booth.phase() {
                Phase::Countdown => "Get ready...".to_owned(),
                Phase::Shooting => "Taking photo...".to_owned(),
                _ => self
                    .selected_capture
                    .map(|index| format!("Retake photo {}", index + 1))
                    .unwrap_or_else(|| "Take photo".to_owned()),
            };
            let ready = self.booth.ready_for_picture()
                && self.preview_texture.is_some()
                && self.template_error.is_none();
            let size = Vec2::new((width - 144.0).clamp(120.0, 210.0), 46.0);
            let take_rect = egui::Rect::from_center_size(row.center(), size);
            let mut take_ui = ui.new_child(
                egui::UiBuilder::new()
                    .id_salt("take-photo")
                    .max_rect(take_rect)
                    .layout(egui::Layout::top_down(egui::Align::Center)),
            );
            let response = theme::primary_button(&mut take_ui, &label, size, ready);
            theme::camera_mark(
                take_ui.painter(),
                egui::pos2(response.rect.left() + 21.0, response.rect.center().y),
                8.0,
                if ready {
                    theme::ON_ACCENT
                } else {
                    theme::ON_ACCENT.gamma_multiply(0.5)
                },
            );
            if response.clicked() {
                self.request_picture();
            }
        }
        action
    }

    /// Portrait kiosk: the capture action stays above the full-width live preview,
    /// with session photos below it. Compact windows use the same hierarchy.
    fn booth_page(&mut self, ui: &mut egui::Ui) {
        let available_height = ui.available_height();
        let compact = available_height < 720.0;
        if self.booth.retaking_index().is_some()
            || self.selected_capture.is_some()
            || self.capture_error.is_some()
            || self.template_error.is_some()
        {
            egui::Panel::top("booth_context")
                .show_separator_line(false)
                .frame(egui::Frame::new().inner_margin(egui::Margin::symmetric(20, 4)))
                .show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        if self.booth.retaking_index().is_some() {
                            if ui
                                .add_enabled(
                                    self.booth.phase() == Phase::Countdown,
                                    egui::Button::new("Keep original photo").corner_radius(20),
                                )
                                .clicked()
                            {
                                self.booth.cancel_retake();
                                self.capture_error = None;
                            }
                        } else if self.selected_capture.is_some()
                            && self.booth.ready_for_picture()
                            && ui.button("Back to live camera").clicked()
                        {
                            self.selected_capture = None;
                        }
                        if let Some(error) =
                            self.capture_error.as_ref().or(self.template_error.as_ref())
                        {
                            ui.label(RichText::new(error).size(13.0).color(theme::ACCENT));
                        }
                    });
                });
        }

        let slots = self.booth.shots_wanted().max(1) as usize;
        egui::Panel::bottom("booth_filmstrip")
            .frame(egui::Frame::new().inner_margin(egui::Margin::symmetric(20, 10)))
            .show(ui, |ui| {
                let width = ui.available_width();
                let columns = slots.min(if width < 1000.0 && !compact { 4 } else { 8 });
                let gap = if width < 700.0 { 10.0 } else { 14.0 };
                let rows = slots.div_ceil(columns);
                let filmstrip_share = if rows > 1 { 0.32 } else { 0.24 };
                let max_cell_h =
                    ((available_height * filmstrip_share - 54.0) / rows as f32).max(54.0);
                let mut cell_w = ((width - gap * (columns - 1) as f32) / columns as f32).min(220.0);
                let footer = if cell_w < 100.0 { 30.0 } else { 40.0 };
                cell_w = cell_w.min(((max_cell_h - footer) / 0.75 + 12.0).max(40.0));
                let image_h = (cell_w - 12.0).max(1.0) * 0.75;
                let cell_h = image_h + if cell_w < 100.0 { 30.0 } else { 40.0 };
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(format!(
                                "{} / {} captured",
                                self.booth.shots_taken(),
                                slots
                            ))
                            .size(12.0)
                            .color(theme::MUTED),
                        );
                        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            ui.add(
                                egui::Label::new(theme::eyebrow(&self.settings.filmstrip_heading))
                                    .truncate(),
                            )
                            .on_hover_text(&self.settings.filmstrip_heading);
                        });
                    });
                });
                ui.add_space(3.0);
                let progress_width = (width - gap * (slots - 1) as f32) / slots as f32;
                let (progress, _) =
                    ui.allocate_exact_size(Vec2::new(width, 3.0), egui::Sense::hover());
                for index in 0..slots {
                    let segment = egui::Rect::from_min_size(
                        progress.min + Vec2::new(index as f32 * (progress_width + gap), 0.0),
                        Vec2::new(progress_width, 2.0),
                    );
                    ui.painter().rect_filled(
                        segment,
                        1.0,
                        if index < self.shot_textures.len() {
                            theme::ACCENT
                        } else {
                            theme::BORDER
                        },
                    );
                }
                ui.add_space(9.0);
                for first in (0..slots).step_by(columns) {
                    let count = (slots - first).min(columns);
                    let total = count as f32 * cell_w + (count - 1) as f32 * gap;
                    let (row, _) =
                        ui.allocate_exact_size(Vec2::new(width, cell_h), egui::Sense::hover());
                    let start = row.center().x - total / 2.0;
                    for column in 0..count {
                        let cell = egui::Rect::from_min_size(
                            egui::pos2(start + column as f32 * (cell_w + gap), row.top()),
                            Vec2::new(cell_w, cell_h),
                        );
                        let index = first + column;
                        if self.paint_filmstrip_cell(ui, cell, index) {
                            self.selected_capture = Some(index);
                        }
                    }
                }
            });

        egui::CentralPanel::default()
            .frame(egui::Frame::new())
            .show(ui, |ui| {
                let (area, _) = ui.allocate_exact_size(ui.available_size(), egui::Sense::hover());
                self.paint_preview(ui, area);
            });
    }

    /// Fill the live viewport without stretching; selected captured shots remain fully visible.
    fn paint_preview(&self, ui: &egui::Ui, area: egui::Rect) {
        if area.width() < 20.0 || area.height() < 20.0 {
            return;
        }
        let painter = ui.painter_at(area);
        let selected = self
            .selected_capture
            .filter(|_| self.booth.ready_for_picture());
        let texture = selected
            .and_then(|index| self.shot_textures.get(index))
            .or(self.preview_texture.as_ref());
        let Some(texture) = texture else {
            painter.rect_filled(area, 18.0, theme::SURFACE);
            painter.text(
                area.center() - Vec2::new(0.0, 16.0),
                egui::Align2::CENTER_CENTER,
                "Getting the camera ready",
                theme::editorial(26.0),
                theme::TEXT,
            );
            painter.text(
                area.center() + Vec2::new(0.0, 20.0),
                egui::Align2::CENTER_CENTER,
                "Your live preview will appear here.",
                egui::FontId::proportional(14.0),
                theme::MUTED,
            );
            return;
        };
        let ts = texture.size_vec2();
        let bounds = if selected.is_some() {
            egui::Rect::from_min_size(
                area.min + Vec2::splat(6.0),
                (area.size() - Vec2::new(12.0, 36.0)).max(Vec2::splat(1.0)),
            )
        } else {
            area
        };
        let (image_rect, uv) = preview_placement(bounds, ts, selected.is_none());
        let card = image_rect.expand(5.0);
        if selected.is_some() {
            painter.rect_filled(card, 18.0, theme::SURFACE);
            painter.rect_stroke(
                card,
                18.0,
                egui::Stroke::new(1.0, theme::BORDER),
                egui::StrokeKind::Inside,
            );
        }
        egui::Image::from_texture(SizedTexture::new(texture.id(), image_rect.size()))
            .uv(uv)
            .corner_radius(if selected.is_some() { 14 } else { 0 })
            .paint_at(ui, image_rect);

        if selected.is_none() && image_rect.width() > 260.0 && image_rect.height() > 160.0 {
            let badge = egui::Rect::from_min_size(
                image_rect.min + Vec2::splat(14.0),
                Vec2::new(80.0, 28.0),
            );
            painter.rect_filled(badge, 14.0, Color32::from_black_alpha(185));
            painter.circle_filled(badge.min + Vec2::new(15.0, 14.0), 3.0, theme::LIVE);
            painter.text(
                badge.min + Vec2::new(28.0, 14.0),
                egui::Align2::LEFT_CENTER,
                "LIVE",
                theme::button_font(10.0),
                theme::TEXT,
            );
        }
        if let Some(index) = selected {
            painter.text(
                egui::pos2(image_rect.center().x, card.bottom() + 16.0),
                egui::Align2::CENTER_CENTER,
                format!("PHOTO {}  /  SELECT RETAKE TO TRY AGAIN", index + 1),
                egui::FontId::proportional(10.0),
                theme::MUTED,
            );
        }
        if self.booth.phase() == Phase::Preview && selected.is_none() {
            theme::paint_brand(&painter, image_rect);
        }
        if self.booth.phase() == Phase::Countdown {
            let radius = (image_rect.height() * 0.18).clamp(35.0, 90.0);
            let center = image_rect.center();
            // No filled disk: guests can see their pose through the countdown.
            painter.circle_stroke(
                center,
                radius,
                egui::Stroke::new(1.0, Color32::from_white_alpha(70)),
            );
            let fraction = (self.booth.remaining_ms() as f32
                / (self.settings.countdown_seconds.max(1) as f32 * 1000.0))
                .clamp(0.0, 1.0);
            let points = (0..=64)
                .map(|i| {
                    let angle = -std::f32::consts::FRAC_PI_2
                        + std::f32::consts::TAU * fraction * i as f32 / 64.0;
                    center + Vec2::angled(angle) * (radius - 4.0)
                })
                .collect();
            painter.add(egui::Shape::line(
                points,
                egui::Stroke::new(2.0, theme::ACCENT),
            ));
            painter.text(
                center - Vec2::new(0.0, radius * 0.08),
                egui::Align2::CENTER_CENTER,
                self.booth.remaining_seconds().to_string(),
                theme::editorial(radius * 1.12),
                Color32::from_white_alpha(205),
            );
            if radius >= 55.0 {
                painter.text(
                    center + Vec2::new(0.0, radius * 0.55),
                    egui::Align2::CENTER_CENTER,
                    "HOLD THAT POSE",
                    theme::button_font(9.0),
                    theme::ACCENT,
                );
            }
        }
    }

    /// Captured frames become little paper prints. Empty slots keep their place
    /// and clearly identify the next shot without implying it was captured.
    fn paint_filmstrip_cell(&self, ui: &egui::Ui, cell: egui::Rect, index: usize) -> bool {
        let painter = ui.painter();
        let texture = self.shot_textures.get(index);
        let latest = self.last_capture == Some(index);
        let selected = self.selected_capture == Some(index);
        let enabled = texture.is_some() && self.booth.ready_for_picture();
        let response = ui.interact(
            cell,
            ui.scope_id().with(("captured-photo", index)),
            if enabled {
                egui::Sense::click()
            } else {
                egui::Sense::hover()
            },
        );
        if texture.is_some() {
            response.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::accesskit::Role::Button,
                    enabled,
                    format!("View photo {}", index + 1),
                )
            });
        }
        let next = index == self.shot_textures.len();
        let narrow = cell.width() < 100.0;
        let footer = if narrow { 24.0 } else { 34.0 };
        painter.rect_filled(
            cell,
            10.0,
            if texture.is_some() {
                theme::TEXT
            } else {
                theme::SURFACE
            },
        );
        painter.rect_stroke(
            cell,
            10.0,
            egui::Stroke::new(
                if selected {
                    3.0
                } else if latest {
                    2.0
                } else {
                    1.0
                },
                if selected || latest || next {
                    theme::ACCENT
                } else {
                    theme::BORDER
                },
            ),
            egui::StrokeKind::Inside,
        );
        let image_area = egui::Rect::from_min_max(
            cell.min + Vec2::splat(6.0),
            cell.max - Vec2::new(6.0, footer),
        );
        let ink = if texture.is_some() {
            theme::ON_ACCENT
        } else {
            theme::MUTED
        };
        if let Some(texture) = texture {
            painter.rect_filled(image_area, 5.0, Color32::BLACK);
            let ts = texture.size_vec2();
            let size = ts * (image_area.width() / ts.x).min(image_area.height() / ts.y);
            egui::Image::from_texture(SizedTexture::new(texture.id(), size))
                .corner_radius(4)
                .paint_at(ui, egui::Rect::from_center_size(image_area.center(), size));
        } else {
            painter.text(
                image_area.center(),
                egui::Align2::CENTER_CENTER,
                format!("{:02}", index + 1),
                theme::editorial(if narrow { 22.0 } else { 32.0 }),
                if next { theme::ACCENT } else { theme::BORDER },
            );
        }
        let baseline = cell.bottom() - footer / 2.0;
        painter.text(
            egui::pos2(cell.left() + 12.0, baseline),
            egui::Align2::LEFT_CENTER,
            format!("{:02}", index + 1),
            theme::button_font(10.0),
            ink,
        );
        if !narrow {
            let label = if selected {
                "SELECTED"
            } else if latest {
                "JUST TAKEN"
            } else if texture.is_some() {
                "CAPTURED"
            } else if next {
                "UP NEXT"
            } else {
                ""
            };
            painter.text(
                egui::pos2(cell.right() - 12.0, baseline),
                egui::Align2::RIGHT_CENTER,
                label,
                egui::FontId::proportional(9.0),
                ink,
            );
        }
        response
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .clicked()
    }

    fn apply_template(&mut self, template: Template) {
        let path = self.settings_path.with_file_name("active-layout.sbl");
        match template.save(&path) {
            Ok(()) => {
                self.settings.template_path = Some(path);
                self.template = Some(template);
                self.template_error = None;
                if self.save_settings() {
                    self.show_editor = false;
                    self.show_library = false;
                    self.status = "Layout applied. Ready for your next session.".into();
                } else {
                    self.editor.message = self.status.clone();
                    self.library.message = self.status.clone();
                }
            }
            Err(e) => {
                self.editor.message = format!("Could not apply layout: {e}");
                self.library.message = self.editor.message.clone();
            }
        }
    }

    fn review_page(&mut self, ui: &mut egui::Ui, fullscreen: bool) {
        // The scroll content has unbounded height; size the preview from the viewport.
        let viewport_height = ui.available_height();
        egui::ScrollArea::vertical()
            .id_salt("review_page")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.label(theme::eyebrow("THAT'S A KEEPER"));
                let mut start_again = false;
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("Your photos, your memories.")
                            .font(theme::editorial(if ui.available_width() < 800.0 {
                                30.0
                            } else {
                                38.0
                            }))
                            .color(theme::TEXT),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        start_again = ui
                            .add(
                                egui::Button::new("Start again")
                                    .min_size(Vec2::new(125.0, 42.0))
                                    .corner_radius(24),
                            )
                            .clicked();
                    });
                });
                if start_again {
                    self.reset_session();
                    return;
                }
                if let Some(error) = &self.capture_error {
                    ui.label(RichText::new(error).color(theme::ACCENT));
                }
                ui.label(
                    RichText::new(
                        "Drag photos on the template to arrange them, then print your collection.",
                    )
                    .size(14.0)
                    .color(theme::MUTED),
                );
                ui.add_space(20.0);
                let width = ui.available_width();
                if width < 950.0 {
                    let preview_height = (viewport_height * 0.48).clamp(180.0, 520.0);
                    self.review_preview(ui, width, preview_height);
                    ui.add_space(18.0);
                    self.review_actions(ui, width, fullscreen);
                } else {
                    let preview_width = width - 340.0;
                    let preview_height = (viewport_height - 220.0).max(180.0);
                    ui.horizontal_top(|ui| {
                        ui.allocate_ui_with_layout(
                            Vec2::new(preview_width, 0.0),
                            egui::Layout::top_down(egui::Align::Center),
                            |ui| {
                                self.review_preview(ui, preview_width, preview_height);
                            },
                        );
                        ui.add_space(14.0);
                        ui.allocate_ui_with_layout(
                            Vec2::new(310.0, 0.0),
                            egui::Layout::top_down(egui::Align::LEFT),
                            |ui| {
                                self.review_actions(ui, 310.0, fullscreen);
                            },
                        );
                    });
                }
            });
    }

    fn review_preview(&mut self, ui: &mut egui::Ui, width: f32, height: f32) {
        let controls_top = ui.cursor().top();
        self.review_photo_controls(ui);
        ui.add_space(12.0);
        let controls_height = ui.cursor().top() - controls_top;
        let height = (height - controls_height).max(140.0);
        let (area, _) = ui.allocate_exact_size(Vec2::new(width, height), egui::Sense::hover());
        let texture = if self.review_selection == 0 {
            self.review_texture.as_ref()
        } else {
            self.shot_textures.get(self.review_selection - 1)
        };
        ui.painter().rect_filled(area, 18.0, theme::SURFACE);
        ui.painter().rect_stroke(
            area,
            18.0,
            egui::Stroke::new(1.0, theme::BORDER),
            egui::StrokeKind::Inside,
        );
        if let Some(texture) = texture {
            let ts = texture.size_vec2();
            let available = area.shrink(16.0).size();
            let size = ts * (available.x / ts.x).min(available.y / ts.y);
            let image_rect = egui::Rect::from_center_size(area.center(), size);
            egui::Image::from_texture(SizedTexture::new(texture.id(), size))
                .corner_radius(6)
                .paint_at(ui, image_rect);
            if self.review_selection == 0 {
                self.interact_template(ui, image_rect);
            }
        } else {
            ui.painter().text(
                area.center(),
                egui::Align2::CENTER_CENTER,
                "The photo layout could not be prepared.",
                egui::FontId::proportional(15.0),
                theme::MUTED,
            );
        }
        ui.add_space(12.0);
        egui::ScrollArea::horizontal()
            .id_salt("review_thumbnails")
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new("ALL").font(theme::button_font(14.0)),
                                )
                                .min_size(Vec2::new(82.0, 60.0))
                                .selected(self.review_selection == 0)
                                .stroke(egui::Stroke::new(
                                    1.0,
                                    if self.review_selection == 0 {
                                        theme::ACCENT
                                    } else {
                                        theme::BORDER
                                    },
                                ))
                                .corner_radius(10),
                            )
                            .clicked()
                        {
                            self.review_selection = 0;
                        }
                        ui.label(RichText::new("Full layout").size(11.0).color(theme::MUTED));
                    });
                    for (index, texture) in self.shot_textures.iter().enumerate() {
                        ui.vertical(|ui| {
                            let response = ui.add(
                                egui::Button::image(
                                    egui::Image::from_texture(SizedTexture::new(
                                        texture.id(),
                                        Vec2::new(72.0, 54.0),
                                    ))
                                    .corner_radius(5)
                                    .alt_text(format!("View photo {}", index + 1)),
                                )
                                .selected(self.review_selection == index + 1)
                                .stroke(egui::Stroke::new(
                                    1.0,
                                    if self.review_selection == index + 1 {
                                        theme::ACCENT
                                    } else {
                                        theme::BORDER
                                    },
                                ))
                                .corner_radius(10),
                            );
                            if response.clicked() {
                                self.review_selection = index + 1;
                            }
                            ui.label(
                                RichText::new(format!("Photo {:02}", index + 1))
                                    .size(11.0)
                                    .color(theme::MUTED),
                            );
                        });
                    }
                });
            });
    }

    fn review_regions(&self) -> Vec<(usize, crate::design::Frame)> {
        if let Some(template) = &self.session_template {
            template
                .photo_regions()
                .filter(|(index, _)| *index < self.shots.len())
                .collect()
        } else {
            layout::photo_cells(
                self.shots.len(),
                self.settings.layout,
                &self.layout_options(),
            )
            .into_iter()
            .map(|cell| {
                (
                    cell.shot,
                    crate::design::Frame::new(
                        cell.x as f64,
                        cell.y as f64,
                        cell.width as f64,
                        cell.height as f64,
                    ),
                )
            })
            .collect()
        }
    }

    fn template_photo_at(&self, rect: egui::Rect, pointer: egui::Pos2) -> Option<usize> {
        if !rect.contains(pointer) {
            return None;
        }
        let image = self.print_image.as_ref()?;
        let x = ((pointer.x - rect.left()) / rect.width()) as f64 * image.width() as f64;
        let y = ((pointer.y - rect.top()) / rect.height()) as f64 * image.height() as f64;
        self.review_regions()
            .into_iter()
            .rev()
            .find_map(|(index, frame)| frame.contains(x, y).then_some(index))
    }

    /// One drag surface uses exact rotated photo regions, rather than competing
    /// bounding-box widgets. Esc and drops outside a photo leave the order intact.
    fn interact_template(&mut self, ui: &mut egui::Ui, rect: egui::Rect) {
        let response = ui.interact(
            rect,
            ui.scope_id().with("arrange-template"),
            egui::Sense::click_and_drag(),
        );
        response.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::accesskit::Role::Image,
                true,
                "Photo template: drag a photo onto another photo to swap their positions",
            )
        });
        if self.booth.phase() != Phase::Review {
            return;
        }
        if response.drag_started() {
            if let Some(index) = ui
                .input(|input| input.pointer.press_origin())
                .and_then(|point| self.template_photo_at(rect, point))
            {
                egui::DragAndDrop::set_payload(ui.ctx(), DraggedPhoto { index });
            }
        }
        let pointer = ui.input(|input| input.pointer.interact_pos());
        let target = pointer.and_then(|point| self.template_photo_at(rect, point));
        let payload = egui::DragAndDrop::payload::<DraggedPhoto>(ui.ctx());
        if response.hovered() && target.is_some() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
        }
        if let Some(drag) = payload.as_ref() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
            let image = self.print_image.as_ref().unwrap();
            let size = Vec2::new(image.width() as f32, image.height() as f32);
            let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
            for (index, frame) in self.review_regions() {
                if Some(index) == target || index == drag.index {
                    let points = frame.corners().map(|(x, y)| {
                        rect.min + Vec2::new(x as f32 / size.x, y as f32 / size.y) * rect.size()
                    });
                    painter.add(egui::Shape::closed_line(
                        points.to_vec(),
                        egui::Stroke::new(
                            if Some(index) == target { 3.0 } else { 1.0 },
                            theme::ACCENT,
                        ),
                    ));
                }
            }
            if let Some(point) = pointer.filter(|point| rect.contains(*point)) {
                if let Some(texture) = self.shot_textures.get(drag.index) {
                    let ghost = egui::Rect::from_min_size(
                        point + Vec2::new(14.0, 14.0),
                        Vec2::new(88.0, 66.0),
                    );
                    painter.rect_filled(ghost.expand(4.0), 6.0, Color32::from_black_alpha(180));
                    painter.image(
                        texture.id(),
                        ghost,
                        egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                        Color32::from_white_alpha(200),
                    );
                }
            }
        }
        if let Some(drag) = response.dnd_release_payload::<DraggedPhoto>() {
            if let Some(target) = target {
                if self.reorder_photos(ui.ctx(), PhotoReorder::Swap, drag.index, target) {
                    self.review_selection = 0;
                }
            }
        }
    }

    fn review_photo_controls(&mut self, ui: &mut egui::Ui) {
        ui.add_space(12.0);
        if let Some(index) = self.review_selection.checked_sub(1) {
            let count = self.shots.len();
            let mut destination = None;
            let mut retake = false;
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new(format!("Photo {} of {}", index + 1, count)).color(theme::MUTED),
                );
                if ui
                    .add_enabled(index > 0, egui::Button::new("Move earlier"))
                    .clicked()
                {
                    destination = Some(index - 1);
                }
                if ui
                    .add_enabled(index + 1 < count, egui::Button::new("Move later"))
                    .clicked()
                {
                    destination = Some(index + 1);
                }
                retake = ui.button(format!("Retake photo {}", index + 1)).clicked();
            });
            if let Some(to) = destination {
                self.reorder_photos(ui.ctx(), PhotoReorder::Move, index, to);
            }
            if retake {
                self.request_retake(index);
            }
        } else {
            ui.label(
                RichText::new("Drag photos onto each other on the template to swap them. Tap a thumbnail to retake.")
                    .size(12.0)
                    .color(theme::MUTED),
            );
        }
    }

    fn review_actions(&mut self, ui: &mut egui::Ui, width: f32, fullscreen: bool) {
        let inner_width = (width - 40.0).max(100.0);
        ui.set_width(width);
        let previous_copies = self.print_copies;
        ui.horizontal(|ui| {
            ui.label(RichText::new("Copies").size(16.0));
            if ui
                .add_enabled(
                    self.print_copies > 1,
                    egui::Button::new("-").min_size(Vec2::splat(42.0)),
                )
                .on_hover_text("Fewer copies")
                .clicked()
            {
                self.print_copies -= 1;
            }
            ui.add_sized(
                [64.0, 42.0],
                egui::DragValue::new(&mut self.print_copies)
                    .range(1..=printing::MAX_COPIES)
                    .speed(1),
            );
            if ui
                .add_enabled(
                    self.print_copies < printing::MAX_COPIES,
                    egui::Button::new("+").min_size(Vec2::splat(42.0)),
                )
                .on_hover_text("More copies")
                .clicked()
            {
                self.print_copies += 1;
            }
        });
        if previous_copies != self.print_copies {
            self.print_feedback = None;
        }
        ui.add_space(12.0);
        let label = if self.print_copies == 1 {
            "Print 1 copy".to_owned()
        } else {
            format!("Print {} copies", self.print_copies)
        };
        if theme::primary_button(
            ui,
            &label,
            Vec2::new(width.min(380.0), 54.0),
            self.print_image.is_some() && self.booth.phase() == Phase::Review,
        )
        .on_hover_text("Confirm your printer and paper in Windows")
        .clicked()
        {
            if let Some(image) = &self.print_image {
                self.print_feedback = match printing::print_image(image, self.print_copies, None) {
                    Ok(PrintOutcome::Submitted { copies, .. }) => {
                        self.print_copies = copies;
                        Some(format!(
                            "{} {} sent to the printer.",
                            copies,
                            if copies == 1 { "copy" } else { "copies" }
                        ))
                    }
                    Ok(PrintOutcome::Cancelled) => None,
                    Err(error) => Some(format!("Printing failed: {error}")),
                };
            }
        }
        if let Some(feedback) = &self.print_feedback {
            ui.add_space(8.0);
            ui.label(feedback);
        }
        ui.add_space(12.0);
        theme::card().show(ui, |ui| {
            ui.set_width(inner_width);
            ui.horizontal(|ui| {
                ui.label(RichText::new("Send photos").font(theme::editorial(21.0)));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(theme::eyebrow("COMING SOON").size(8.0).color(theme::MUTED));
                });
            });
            ui.label(
                RichText::new("Email and text delivery are on the way.")
                    .size(13.0)
                    .color(theme::MUTED),
            );
        });
        if !fullscreen {
            if let Some(path) = &self.last_strip {
                ui.add_space(12.0);
                ui.label(
                    RichText::new(format!("Saved to {}", path.display()))
                        .size(11.0)
                        .color(theme::MUTED),
                );
            }
        }
    }
}

impl eframe::App for SparkboothApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        self.library.poll(&ctx);

        // Advance the booth clock; capture on demand.
        let dt_ms = (ctx.input(|i| i.stable_dt) * 1000.0) as u64;
        if let BoothEvent::ShotDue { .. } = self.booth.update(dt_ms) {
            match self.camera.request_capture() {
                Some(rx) => self.capture_rx = Some(rx),
                None => {
                    self.capture_error=Some("Camera worker is unavailable. Please ask the host to reconnect the camera.".into());
                    self.booth.capture_failed();
                }
            }
        }
        self.drain_capture(&ctx);
        // The preview is hidden in Review, so don't upload frames for it.
        if self.booth.phase() != Phase::Review {
            self.update_camera_texture(&ctx);
        }

        // Repaint only as often as the current screen needs. egui still wakes
        // immediately for input, so buttons and sliders stay responsive.
        let repaint_after = match self.booth.phase() {
            Phase::Preview | Phase::Ready | Phase::Countdown => Duration::from_millis(33),
            Phase::Shooting => Duration::from_millis(50),
            Phase::Review => Duration::from_millis(250),
        };
        ctx.request_repaint_after(repaint_after);

        let fullscreen = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
        let keyboard_toggle = ctx.input(|i| {
            if i.key_pressed(egui::Key::F11) {
                Some(!fullscreen)
            } else if fullscreen && i.key_pressed(egui::Key::Escape) {
                Some(false)
            } else {
                None
            }
        });
        if let Some(enabled) = keyboard_toggle {
            self.set_fullscreen(&ctx, enabled);
        }

        let mut action = HeaderAction::None;
        egui::Panel::top("booth_controls")
            .show_separator_line(false)
            .frame(
                egui::Frame::new()
                    .fill(theme::BACKGROUND)
                    .inner_margin(egui::Margin::symmetric(20, 8)),
            )
            .show(ui, |ui| {
                action = self.top_controls(ui, fullscreen);
            });
        match action {
            HeaderAction::Fullscreen => self.set_fullscreen(&ctx, !fullscreen),
            HeaderAction::Settings => {
                if fullscreen {
                    self.set_fullscreen(&ctx, false);
                }
                self.show_settings = !self.show_settings;
                self.show_editor = false;
                self.show_library = false;
            }
            HeaderAction::Library => {
                if fullscreen {
                    self.set_fullscreen(&ctx, false);
                }
                self.show_library = true;
                self.show_editor = false;
                self.show_settings = false;
                self.library.open(
                    &self.settings.layout_library_dir,
                    &self.settings.layout_catalog_url,
                );
            }
            HeaderAction::Studio => {
                if fullscreen {
                    self.set_fullscreen(&ctx, false);
                }
                self.show_editor = !self.show_editor;
                self.show_settings = false;
                self.show_library = false;
            }
            HeaderAction::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            HeaderAction::None => {}
        }

        if !fullscreen
            && self.show_settings
            && !self.show_editor
            && self.booth.phase() == Phase::Preview
        {
            egui::Panel::right("settings_panel")
                .default_size(320.0)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.heading("Booth settings");
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("Close").on_hover_text("Close settings").clicked() {
                                self.show_settings = false;
                            }
                        });
                    });
                    egui::ScrollArea::vertical()
                        .id_salt("operator_settings_scroll")
                        .show(ui, |ui| {
                            // Operator diagnostics stay available here without a permanent footer.
                            if let Some(name) = &self.camera_active {
                                ui.small(format!("Connected: {name}"));
                            }
                            if let Some(note) = &self.camera_note {
                                ui.small(note);
                            }
                            if !self.status.is_empty() {
                                ui.small(&self.status);
                            }
                            ui.add_space(8.0);
                            ui.label("Photo strip heading");
                            if ui
                                .text_edit_singleline(&mut self.settings.filmstrip_heading)
                                .lost_focus()
                            {
                                self.save_settings();
                            }
                            ui.small("Shown above the captured photos. Leave blank to hide it.");
                            ui.add_space(8.0);
                            if let Some(t) = &self.template {
                                ui.label(format!("{} · {} captures", t.name, t.shots_needed()));
                                ui.small("Capture count follows the applied template.");
                            }
                            if self.settings.template_path.is_some()
                                && ui.button("Use basic strip / grid instead").clicked()
                            {
                                self.settings.template_path = None;
                                self.template = None;
                                self.template_error = None;
                                self.save_settings();
                            }
                            ui.horizontal(|ui| {
                                ui.label("Shots");
                                if ui
                                    .add_enabled(
                                        self.template.is_none(),
                                        egui::Slider::new(&mut self.settings.shots, 1..=8),
                                    )
                                    .changed()
                                {
                                    self.save_settings();
                                }
                            });
                            ui.horizontal(|ui| {
                                ui.label("Countdown (s)");
                                if ui
                                    .add(egui::Slider::new(
                                        &mut self.settings.countdown_seconds,
                                        1..=10,
                                    ))
                                    .changed()
                                {
                                    self.save_settings();
                                }
                            });
                            ui.add_enabled_ui(self.template.is_none(), |ui| {
                                ui.horizontal(|ui| {
                                    ui.label("Layout");
                                    egui::ComboBox::from_id_salt("layout")
                                        .selected_text(match self.settings.layout {
                                            LayoutKind::Strip => "Strip",
                                            LayoutKind::Grid2x2 => "Grid 2x2",
                                        })
                                        .show_ui(ui, |ui| {
                                            if ui
                                                .selectable_value(
                                                    &mut self.settings.layout,
                                                    LayoutKind::Strip,
                                                    "Strip",
                                                )
                                                .changed()
                                            {
                                                self.save_settings();
                                            }
                                            if ui
                                                .selectable_value(
                                                    &mut self.settings.layout,
                                                    LayoutKind::Grid2x2,
                                                    "Grid 2x2",
                                                )
                                                .changed()
                                            {
                                                self.save_settings();
                                            }
                                        });
                                });
                                if ui
                                    .checkbox(&mut self.settings.light_border, "Light border")
                                    .changed()
                                {
                                    self.save_settings();
                                }
                            });
                            if ui
                                .checkbox(
                                    &mut self.settings.save_individual_shots,
                                    "Save source shots",
                                )
                                .changed()
                            {
                                self.save_settings();
                            }
                            if ui
                                .checkbox(&mut self.settings.simulate, "Simulated camera")
                                .changed()
                            {
                                self.reconnect_camera();
                            }
                            ui.horizontal(|ui| {
                                ui.label("Camera");
                                if self.camera_names.is_empty() {
                                    ui.label(RichText::new("none detected").weak());
                                    let response = ui.add(
                                        egui::DragValue::new(&mut self.settings.camera_index)
                                            .speed(1)
                                            .prefix("index "),
                                    );
                                    if response.changed() {
                                        self.reconnect_camera();
                                    }
                                } else {
                                    let selected = self
                                        .camera_names
                                        .get(self.settings.camera_index as usize)
                                        .cloned()
                                        .unwrap_or_else(|| {
                                            format!("Camera {}", self.settings.camera_index)
                                        });
                                    egui::ComboBox::from_id_salt("camera_choice")
                                        .selected_text(selected)
                                        .show_ui(ui, |ui| {
                                            let names = self.camera_names.clone();
                                            for (position, name) in names.iter().enumerate() {
                                                let index = position as u32;
                                                if ui
                                                    .selectable_value(
                                                        &mut self.settings.camera_index,
                                                        index,
                                                        format!("{name} (#{index})"),
                                                    )
                                                    .changed()
                                                {
                                                    self.reconnect_camera();
                                                }
                                            }
                                        });
                                }
                            });
                            if ui.button("Reconnect camera").clicked() {
                                self.reconnect_camera();
                            }
                            ui.label("Output folder");
                            let mut dir = self.settings.output_dir.to_string_lossy().into_owned();
                            if ui.text_edit_singleline(&mut dir).lost_focus() {
                                self.settings.output_dir = PathBuf::from(dir);
                                self.save_settings();
                            }
                            ui.separator();
                            ui.heading("Layout library");
                            ui.add_enabled_ui(!self.library.busy(), |ui| {
                                ui.label("Cloud catalog URL");
                                if ui
                                    .add(
                                        egui::TextEdit::singleline(
                                            &mut self.settings.layout_catalog_url,
                                        )
                                        .hint_text("https://…/catalog.json")
                                        .desired_width(f32::INFINITY),
                                    )
                                    .lost_focus()
                                {
                                    self.save_settings();
                                }
                                ui.small("Optional. Add it when your server is ready.");
                                ui.label("Download folder");
                                ui.add(
                                    egui::TextEdit::singleline(&mut self.library_folder_input)
                                        .desired_width(f32::INFINITY),
                                );
                                if ui.button("Save library connection").clicked() {
                                    if self.library_folder_input.trim().is_empty() {
                                        self.status =
                                            "Choose a folder for downloaded layouts.".into();
                                    } else {
                                        self.settings.layout_library_dir =
                                            PathBuf::from(&self.library_folder_input);
                                        self.save_settings();
                                    }
                                }
                            });
                            ui.small(
                                "Cloud selections download here as .sbl files for offline use.",
                            );
                            if ui.button("Open layout library").clicked() {
                                self.library.open(
                                    &self.settings.layout_library_dir,
                                    &self.settings.layout_catalog_url,
                                );
                                self.show_library = true;
                                self.show_settings = false;
                            }
                            ui.separator();
                            ui.monospace(self.settings_path.display().to_string());
                        });
                });
        }

        if !fullscreen && self.show_library {
            let mut selected = None;
            egui::CentralPanel::default()
                .frame(egui::Frame::new().fill(theme::BACKGROUND).inner_margin(20))
                .show(ui, |ui| {
                    selected = self.library.ui(ui);
                });
            match selected {
                Some(LibraryAction::Apply(template)) => self.apply_template(template),
                Some(LibraryAction::Studio(template)) => {
                    self.editor = Editor::new(template);
                    self.show_editor = true;
                    self.show_library = false;
                }
                Some(LibraryAction::Back) => self.show_library = false,
                None => {}
            }
            return;
        }

        if !fullscreen && self.show_editor {
            let mut applied = None;
            egui::CentralPanel::default().show(ui, |ui| {
                applied = self.editor.ui(ui);
            });
            if let Some(template) = applied {
                self.apply_template(template);
            }
            return;
        }

        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(theme::BACKGROUND).inner_margin(
                if self.booth.phase() == Phase::Review {
                    if fullscreen {
                        24
                    } else {
                        20
                    }
                } else {
                    0
                },
            ))
            .show(ui, |ui| match self.booth.phase() {
                Phase::Review => self.review_page(ui, fullscreen),
                _ => self.booth_page(ui),
            });
    }
}

#[cfg(test)]
mod session_edit_tests {
    use super::*;
    use image::{GenericImageView, Rgba, RgbaImage};

    fn app(name: &str) -> (egui::Context, SparkboothApp, PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "radiate-session-edit-{}-{name}",
            std::process::id()
        ));
        let ctx = egui::Context::default();
        let mut app = SparkboothApp::from_context(&ctx, true, Some(dir.join("settings.toml")));
        app.settings.output_dir = dir.clone();
        app.settings.shots = 3;
        app.settings.countdown_seconds = 1;
        app.reload_booth_if_idle();
        (ctx, app, dir)
    }

    fn capture(app: &mut SparkboothApp, ctx: &egui::Context, color: [u8; 4]) {
        app.request_picture();
        assert!(matches!(app.booth.update(1000), BoothEvent::ShotDue { .. }));
        app.accept_capture(
            ctx,
            DynamicImage::ImageRgba8(RgbaImage::from_pixel(16, 12, Rgba(color))),
        );
    }

    fn shot_colors(app: &SparkboothApp) -> Vec<[u8; 4]> {
        app.shots
            .iter()
            .map(|photo| photo.get_pixel(0, 0).0)
            .collect()
    }

    #[test]
    fn preview_fills_wide_and_portrait_viewports_without_stretching() {
        for viewport in [
            Vec2::new(1280.0, 500.0),
            Vec2::new(760.0, 1000.0),
            Vec2::new(640.0, 480.0),
        ] {
            for source in [Vec2::new(640.0, 480.0), Vec2::new(1920.0, 1080.0)] {
                let area = egui::Rect::from_min_size(egui::pos2(10.0, 70.0), viewport);
                let (image, uv) = preview_placement(area, source, true);
                assert_eq!(
                    image, area,
                    "live video must fill both edges of its viewport"
                );
                assert!(uv.min.x > uv.max.x, "live video remains mirrored");
                for point in [uv.min, uv.max] {
                    assert!((0.0..=1.0).contains(&point.x));
                    assert!((0.0..=1.0).contains(&point.y));
                }
                assert!((uv.center().x - 0.5).abs() < 0.00001);
                assert!((uv.center().y - 0.5).abs() < 0.00001);
                let horizontal_scale = source.x * uv.width().abs() / image.width();
                let vertical_scale = source.y * uv.height() / image.height();
                assert!(
                    (horizontal_scale - vertical_scale).abs() < 0.00001,
                    "faces must keep their proportions"
                );
                let (photo, uv) = preview_placement(area, source, false);
                assert_eq!(uv.min, egui::Pos2::ZERO);
                assert_eq!(
                    uv.max,
                    egui::pos2(1.0, 1.0),
                    "captured photos remain fully visible"
                );
                assert!(area.contains_rect(photo));
                assert!((photo.width() / photo.height() - source.x / source.y).abs() < 0.00001);
            }
        }
    }

    #[test]
    fn cancelling_a_set_resets_every_stage_and_detaches_a_pending_capture() {
        for stage in [
            "first-countdown",
            "ready",
            "countdown",
            "shooting",
            "retake",
            "review",
        ] {
            let (ctx, mut app, dir) = app(&format!("cancel-{stage}"));
            let old = [220, 20, 20, 255];
            if stage != "first-countdown" {
                capture(&mut app, &ctx, old);
            }
            match stage {
                "first-countdown" | "countdown" | "shooting" => app.request_picture(),
                "retake" => app.request_retake(0),
                "review" => {
                    capture(&mut app, &ctx, old);
                    capture(&mut app, &ctx, old);
                }
                _ => {}
            }
            let pending = if stage == "shooting" {
                app.booth.update(1000);
                let (sender, receiver) = mpsc::channel();
                app.capture_rx = Some(receiver);
                Some(sender)
            } else {
                None
            };
            let saved = app
                .last_strip
                .as_ref()
                .map(|path| (path.clone(), std::fs::read(path).unwrap()));
            app.capture_error = Some("An earlier capture failed".into());
            app.print_copies = 3;
            app.reset_session();
            assert_eq!(app.booth.phase(), Phase::Preview, "{stage}");
            assert_eq!(app.booth.shots_taken(), 0);
            assert_eq!(app.booth.retaking_index(), None);
            assert!(app.shots.is_empty());
            assert!(app.shot_textures.is_empty());
            assert!(app.capture_rx.is_none());
            assert!(app.capture_error.is_none());
            assert!(app.review_texture.is_none());
            assert!(app.print_image.is_none());
            assert!(app.last_strip.is_none());
            assert!(app.session_template.is_none());
            assert_eq!(app.selected_capture, None);
            assert_eq!(app.last_capture, None);
            assert_eq!(app.print_copies, 1);
            if let Some(sender) = pending {
                assert!(
                    sender
                        .send(Ok(RgbaImage::from_pixel(16, 12, Rgba(old))))
                        .is_err(),
                    "a late frame from the cancelled set must be disconnected"
                );
            }
            if let Some((path, bytes)) = saved {
                assert_eq!(
                    std::fs::read(path).unwrap(),
                    bytes,
                    "cancelling must preserve already saved files"
                );
            }
            let new = [20, 220, 20, 255];
            capture(&mut app, &ctx, new);
            app.drain_capture(&ctx);
            assert_eq!(
                shot_colors(&app),
                vec![new],
                "{stage} must start with an empty set"
            );
            drop(app);
            if dir.exists() {
                std::fs::remove_dir_all(dir).unwrap();
            }
        }
    }

    fn assert_print_order(app: &SparkboothApp, expected: &[[u8; 4]]) {
        let print = app.print_image.as_ref().expect("printable layout");
        let saved = image::open(app.last_strip.as_ref().expect("saved layout")).unwrap();
        for (index, color) in expected.iter().enumerate() {
            let y = 46 + index as u32 * 494;
            assert_eq!(print.get_pixel(46, y).0, *color);
            let actual = saved.get_pixel(46, y).0;
            for channel in 0..3 {
                assert!(
                    actual[channel].abs_diff(color[channel]) <= 4,
                    "saved layout order differs from print"
                );
            }
        }
    }

    #[test]
    fn retake_mid_session_preserves_other_photos_on_cancel_failure_and_success() {
        let (ctx, mut app, dir) = app("partial");
        let red = [220, 20, 20, 255];
        let green = [20, 220, 20, 255];
        capture(&mut app, &ctx, red);
        capture(&mut app, &ctx, green);
        let textures: Vec<_> = app.shot_textures.iter().map(TextureHandle::id).collect();
        app.request_retake(0);
        assert!(app.booth.cancel_retake());
        assert_eq!(shot_colors(&app), vec![red, green]);
        app.request_retake(0);
        app.booth.update(1000);
        let (tx, rx) = mpsc::channel();
        app.capture_rx = Some(rx);
        tx.send(Err("camera disconnected".into())).unwrap();
        app.drain_capture(&ctx);
        assert_eq!(app.booth.phase(), Phase::Ready);
        assert_eq!(shot_colors(&app), vec![red, green]);
        assert_eq!(
            app.shot_textures
                .iter()
                .map(TextureHandle::id)
                .collect::<Vec<_>>(),
            textures
        );
        assert!(app
            .capture_error
            .as_deref()
            .unwrap()
            .contains("disconnected"));
        app.selected_capture = Some(0);
        let blue = [20, 20, 220, 255];
        capture(&mut app, &ctx, blue);
        assert_eq!(shot_colors(&app), vec![blue, green]);
        assert_eq!(app.shot_textures[1].id(), textures[1]);
        assert_eq!(app.booth.shots_taken(), 2);
        assert!(!app.reorder_photos(&ctx, PhotoReorder::Move, 0, 1));
        capture(&mut app, &ctx, red);
        assert_eq!(app.booth.phase(), Phase::Review);
        assert_print_order(&app, &[blue, green, red]);
        app.print_copies = 3;
        app.reset_session();
        assert_eq!(app.print_copies, 1);
        assert!(app.shots.is_empty());
        assert!(app.shot_textures.is_empty());
        assert_eq!(app.selected_capture, None);
        assert_eq!(app.last_capture, None);
        drop(app);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn reorder_then_retake_updates_the_same_slot_in_thumbnails_saved_layout_and_print() {
        let (ctx, mut app, dir) = app("review");
        let colors = [[220, 20, 20, 255], [20, 220, 20, 255], [20, 20, 220, 255]];
        for color in colors {
            capture(&mut app, &ctx, color);
        }
        let original_path = app.last_strip.clone().unwrap();
        let original_bytes = std::fs::read(&original_path).unwrap();
        let textures: Vec<_> = app.shot_textures.iter().map(TextureHandle::id).collect();
        app.print_copies = 3;
        assert!(app.reorder_photos(&ctx, PhotoReorder::Move, 0, 2));
        assert_eq!(app.print_copies, 3);
        assert_eq!(shot_colors(&app), vec![colors[1], colors[2], colors[0]]);
        assert_eq!(
            app.shot_textures
                .iter()
                .map(TextureHandle::id)
                .collect::<Vec<_>>(),
            vec![textures[1], textures[2], textures[0]]
        );
        assert_eq!(app.review_selection, 3);
        assert_print_order(&app, &[colors[1], colors[2], colors[0]]);
        assert!(app.reorder_photos(&ctx, PhotoReorder::Move, 2, 0));
        assert_print_order(&app, &colors);
        assert!(!app.reorder_photos(&ctx, PhotoReorder::Move, 0, 3));
        assert!(!app.reorder_photos(&ctx, PhotoReorder::Move, 3, 0));
        assert!(!app.reorder_photos(&ctx, PhotoReorder::Move, 1, 1));
        app.request_retake(1);
        assert!(app.booth.cancel_retake());
        assert_eq!(app.booth.phase(), Phase::Review);
        assert_print_order(&app, &colors);
        app.request_retake(1);
        app.booth.update(1000);
        let (tx, rx) = mpsc::channel();
        app.capture_rx = Some(rx);
        tx.send(Err("capture failed".into())).unwrap();
        app.drain_capture(&ctx);
        assert_eq!(app.booth.phase(), Phase::Review);
        assert_print_order(&app, &colors);
        app.request_retake(1);
        assert!(!app.reorder_photos(&ctx, PhotoReorder::Move, 0, 1));
        app.booth.update(1000);
        let yellow = [220, 220, 20, 255];
        app.accept_capture(
            &ctx,
            DynamicImage::ImageRgba8(RgbaImage::from_pixel(16, 12, Rgba(yellow))),
        );
        assert_eq!(app.booth.phase(), Phase::Review);
        assert_eq!(app.booth.shots_taken(), 3);
        assert_eq!(shot_colors(&app), vec![colors[0], yellow, colors[2]]);
        assert_eq!(app.shot_textures[0].id(), textures[0]);
        assert_eq!(app.shot_textures[2].id(), textures[2]);
        assert_print_order(&app, &[colors[0], yellow, colors[2]]);
        assert_eq!(std::fs::read(original_path).unwrap(), original_bytes);
        drop(app);
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn dragging_on_template_swaps_photos_and_ignores_cancelled_or_invalid_drops() {
        let (ctx, mut app, dir) = app("drag");
        let colors = [[220, 20, 20, 255], [20, 220, 20, 255], [20, 20, 220, 255]];
        for color in colors {
            capture(&mut app, &ctx, color);
        }
        let rect = egui::Rect::from_min_size(egui::pos2(80.0, 40.0), Vec2::new(356.0, 770.0));
        let center = |index: usize| {
            egui::pos2(
                rect.center().x,
                rect.top() + (276.0 + index as f32 * 494.0) / 2.0,
            )
        };
        let first = center(0);
        let last = center(2);
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let mut time = 0.0;
        let mut frame = |app: &mut SparkboothApp, events| {
            time += 0.05;
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        Vec2::new(1024.0, 1000.0),
                    )),
                    events,
                    time: Some(time),
                    ..Default::default()
                },
                |ui| {
                    egui::CentralPanel::default().show(ui, |ui| app.interact_template(ui, rect));
                },
            );
            // This headless check intentionally does not send textures to a GPU.
            output.textures_delta.clear();
        };
        frame(&mut app, vec![]);
        frame(
            &mut app,
            vec![egui::Event::PointerMoved(first), button(first, true)],
        );
        frame(
            &mut app,
            vec![egui::Event::PointerMoved(first + Vec2::new(20.0, 0.0))],
        );
        assert!(egui::DragAndDrop::has_payload_of_type::<DraggedPhoto>(&ctx));
        assert_eq!(shot_colors(&app), colors);
        frame(
            &mut app,
            vec![egui::Event::PointerMoved(last), button(last, false)],
        );
        assert_eq!(shot_colors(&app), [colors[2], colors[1], colors[0]]);
        assert_print_order(&app, &[colors[2], colors[1], colors[0]]);
        assert_eq!(app.review_selection, 0);
        let saved = app.last_strip.clone();
        // Releasing over the frame's margin must not reorder anything.
        let margin = rect.min + Vec2::splat(2.0);
        frame(
            &mut app,
            vec![egui::Event::PointerMoved(first), button(first, true)],
        );
        frame(
            &mut app,
            vec![egui::Event::PointerMoved(first + Vec2::new(20.0, 0.0))],
        );
        frame(
            &mut app,
            vec![egui::Event::PointerMoved(margin), button(margin, false)],
        );
        assert_eq!(app.last_strip, saved);
        // Escape cancels a drag even if the pointer is subsequently released on a photo.
        frame(
            &mut app,
            vec![egui::Event::PointerMoved(first), button(first, true)],
        );
        frame(
            &mut app,
            vec![egui::Event::PointerMoved(first + Vec2::new(20.0, 0.0))],
        );
        frame(
            &mut app,
            vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        frame(
            &mut app,
            vec![egui::Event::PointerMoved(last), button(last, false)],
        );
        assert_eq!(app.last_strip, saved);
        assert_eq!(shot_colors(&app), [colors[2], colors[1], colors[0]]);
        assert!(!egui::DragAndDrop::has_payload_of_type::<DraggedPhoto>(
            &ctx
        ));
        drop(app);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn template_targets_follow_letterboxing_rotation_visibility_and_repeated_slots() {
        use crate::design::{Frame, Layer};
        let (ctx, mut app, dir) = app("regions");
        let colors = [[220, 20, 20, 255], [20, 220, 20, 255], [20, 20, 220, 255]];
        for color in colors {
            capture(&mut app, &ctx, color);
        }
        let mut template = Template::builtin(false);
        template.width = 300;
        template.height = 200;
        let rotated = Frame {
            rotation: 45.0,
            ..Frame::new(100.0, 20.0, 60.0, 40.0)
        };
        let mut hidden = Layer::photo(Frame::new(0.0, 0.0, 300.0, 200.0), 2);
        hidden.visible = false;
        template.layers = vec![
            Layer::photo(Frame::new(10.0, 10.0, 60.0, 50.0), 0),
            Layer::photo(Frame::new(10.0, 100.0, 60.0, 50.0), 0),
            Layer::photo(rotated, 1),
            hidden,
        ];
        app.session_template = Some(template);
        app.compose_and_save(&ctx);
        let rect = egui::Rect::from_min_size(egui::pos2(80.0, 120.0), Vec2::new(600.0, 400.0));
        let point = |x: f32, y: f32| rect.min + Vec2::new(x, y) * 2.0;
        assert_eq!(app.template_photo_at(rect, point(40.0, 35.0)), Some(0));
        assert_eq!(app.template_photo_at(rect, point(40.0, 125.0)), Some(0));
        assert_eq!(app.template_photo_at(rect, point(107.0, 55.0)), Some(1));
        // A point in the rotated bounding box, but outside the actual photo.
        assert_eq!(app.template_photo_at(rect, point(75.0, 22.0)), None);
        assert_eq!(
            app.template_photo_at(rect, rect.min - Vec2::splat(1.0)),
            None
        );
        assert!(app.reorder_photos(&ctx, PhotoReorder::Swap, 0, 1));
        let image = app.print_image.as_ref().unwrap();
        assert_eq!(image.get_pixel(40, 35).0, colors[1]);
        assert_eq!(image.get_pixel(40, 125).0, colors[1]);
        assert_eq!(image.get_pixel(107, 55).0, colors[0]);
        drop(app);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
