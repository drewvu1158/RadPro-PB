//! eframe UI: preview, countdown overlay, capture, and review screens.

use crate::booth::{Booth, BoothEvent, Phase};
use crate::camera::{CameraClient, CaptureResult};
use crate::layout::{self, LayoutKind, LayoutOptions};
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

pub struct SparkboothApp {
    settings: Settings,
    settings_path: PathBuf,
    show_settings: bool,
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
    delivery_method: bool,
    delivery_address: String,
    print_feedback: Option<String>,
}

impl SparkboothApp {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        simulate: bool,
        settings_path: Option<PathBuf>,
    ) -> Self {
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
        theme::install(&cc.egui_ctx);
        Self {
            settings,
            settings_path,
            show_settings: false,
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
            delivery_method: false,
            delivery_address: String::new(),
            print_feedback: None,
        }
    }

    fn set_fullscreen(&mut self, ctx: &egui::Context, enabled: bool) {
        if enabled {
            self.show_settings = false;
            self.show_editor = false;
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
            Ok(rgba) => {
                self.shots.push(DynamicImage::ImageRgba8(rgba));
                if self.booth.deliver_shot() == BoothEvent::StripComplete {
                    self.compose_and_save(ctx);
                }
            }
            Err(error) => {
                self.capture_error = Some(error);
                self.booth.capture_failed();
            }
        }
    }

    fn compose_and_save(&mut self, ctx: &egui::Context) {
        self.last_strip = None;
        self.print_image = None;
        self.review_selection = 0;
        self.print_feedback = None;
        let opts = LayoutOptions {
            cell: (640, 480),
            margin: 36,
            gap: 14,
            background: if self.settings.light_border {
                [255, 255, 255, 255]
            } else {
                [16, 16, 18, 255]
            },
        };
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
                self.shot_textures = self
                    .shots
                    .iter()
                    .enumerate()
                    .map(|(i, shot)| {
                        let rgba = shot.thumbnail(640, 480).to_rgba8();
                        ctx.load_texture(
                            format!("review-shot-{i}"),
                            egui::ColorImage::from_rgba_unmultiplied(
                                [rgba.width() as usize, rgba.height() as usize],
                                rgba.as_raw(),
                            ),
                            egui::TextureOptions::LINEAR,
                        )
                    })
                    .collect();
            }
            Err(e) => {
                self.status = format!("could not compose strip: {e}");
                self.booth.retake();
                self.shots.clear();
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
        self.delivery_address.clear();
        self.print_feedback = None;
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
        }
        self.capture_error = None;
        self.booth.request_shot();
    }

    /// Fit the whole camera frame, mirror only the preview, and leave a single
    /// guest control. Saved shots retain their original camera orientation.
    fn booth_page(&mut self, ui: &mut egui::Ui) {
        let available = ui.available_size();
        let picture_area = Vec2::new(available.x, (available.y - 100.0).max(60.0));
        let (area, _) = ui.allocate_exact_size(picture_area, egui::Sense::hover());
        let painter = ui.painter_at(area);
        if let Some(texture) = &self.preview_texture {
            let ts = texture.size_vec2();
            let size = ts * (area.width() / ts.x).min(area.height() / ts.y);
            let image_rect = egui::Rect::from_center_size(area.center(), size);
            painter.image(
                texture.id(),
                image_rect,
                egui::Rect::from_min_max(egui::pos2(1.0, 0.0), egui::pos2(0.0, 1.0)),
                Color32::WHITE,
            );
            if self.booth.phase() == Phase::Countdown {
                let radius = (image_rect.height() * 0.16).clamp(40.0, 100.0);
                painter.circle_filled(
                    image_rect.center(),
                    radius,
                    theme::BACKGROUND.gamma_multiply(0.9),
                );
                painter.text(
                    image_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    self.booth.remaining_seconds().to_string(),
                    theme::editorial(radius * 1.2),
                    theme::TEXT,
                );
            }
        } else {
            painter.text(
                area.center(),
                egui::Align2::CENTER_CENTER,
                "Connecting to the camera...",
                egui::FontId::proportional(20.0),
                theme::MUTED,
            );
        }
        ui.add_space(10.0);
        ui.vertical_centered(|ui| {
            let ready = self.booth.ready_for_picture()
                && self.preview_texture.is_some()
                && self.template_error.is_none();
            if ui
                .add_enabled(
                    ready,
                    egui::Button::new(
                        RichText::new("Take picture")
                            .size(22.0)
                            .color(theme::ON_ACCENT),
                    )
                    .fill(theme::ACCENT)
                    .corner_radius(8)
                    .min_size(Vec2::new(280.0, 56.0)),
                )
                .clicked()
            {
                self.request_picture();
            }
            if let Some(error) = self.capture_error.as_ref().or(self.template_error.as_ref()) {
                ui.add(
                    egui::Label::new(RichText::new(error).size(12.0).color(theme::ACCENT))
                        .truncate(),
                )
                .on_hover_text(error);
            } else {
                let state = match self.booth.phase() {
                    Phase::Ready => " / ready when you are",
                    Phase::Shooting => " / taking picture...",
                    _ => "",
                };
                ui.label(
                    RichText::new(format!(
                        "Photo {} of {}{}",
                        (self.booth.shots_taken() + 1).min(self.booth.shots_wanted()),
                        self.booth.shots_wanted(),
                        state
                    ))
                    .size(12.0)
                    .color(theme::MUTED),
                );
            }
        });
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
                    self.status = "Layout applied. Ready for your next session.".into();
                } else {
                    self.editor.message = self.status.clone();
                }
            }
            Err(e) => self.editor.message = format!("Could not apply layout: {e}"),
        }
    }

    fn review_page(&mut self, ui: &mut egui::Ui, fullscreen: bool) {
        let width = ui.available_width().min(1500.0);
        let height = ui.available_height();
        ui.vertical_centered(|ui| {
            ui.allocate_ui_with_layout(
                Vec2::new(width, height),
                egui::Layout::top_down(egui::Align::LEFT),
                |ui| self.review_content(ui, fullscreen),
            );
        });
    }

    fn review_content(&mut self, ui: &mut egui::Ui, fullscreen: bool) {
        ui.label(
            RichText::new("REVIEW")
                .size(12.0)
                .strong()
                .color(theme::ACCENT),
        );
        let mut next_guest = false;
        ui.horizontal(|ui| {
            ui.heading(
                RichText::new("Your photos.")
                    .font(theme::editorial(36.0))
                    .color(theme::TEXT),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                next_guest = ui
                    .add_sized(
                        [250.0, 42.0],
                        egui::Button::new(RichText::new("Start again").size(16.0))
                            .fill(theme::SURFACE_HOVER)
                            .corner_radius(8),
                    )
                    .clicked();
            });
        });
        if next_guest {
            self.reset_session();
            return;
        }
        ui.label(
            RichText::new("View your photos or print the complete layout.").color(theme::MUTED),
        );
        ui.add_space(14.0);
        ui.horizontal_top(|ui| {
            let preview_width = (ui.available_width() - 350.0).max(200.0);
            ui.allocate_ui_with_layout(Vec2::new(preview_width,(ui.available_height()-8.0).max(150.0)),egui::Layout::top_down(egui::Align::Center),|ui| {
                theme::card().inner_margin(12).show(ui,|ui| {
                    let tex = if self.review_selection==0 { self.review_texture.as_ref() } else { self.shot_textures.get(self.review_selection-1) };
                    if let Some(tex) = tex {
                        let available = Vec2::new((preview_width-24.0).max(80.0),(ui.available_height()-142.0).max(80.0));
                        let ts=tex.size_vec2(); let scale=(available.x/ts.x).min(available.y/ts.y);
                        ui.add(egui::Image::from_texture(SizedTexture::new(tex.id(),ts*scale)).corner_radius(8));
                    }
                });
                ui.add_space(12.0);
                egui::ScrollArea::horizontal().id_salt("review_thumbnails").max_height(100.0).show(ui,|ui| {
                    ui.horizontal(|ui| {
                        if ui.selectable_label(self.review_selection==0,"Complete layout").clicked() { self.review_selection=0; }
                        for (i,t) in self.shot_textures.iter().enumerate() {
                            ui.vertical(|ui| {
                                if ui.add(egui::Image::from_texture(SizedTexture::new(t.id(),Vec2::new(72.0,54.0))).corner_radius(8).sense(egui::Sense::click())).clicked() { self.review_selection=i+1; }
                                if ui.selectable_label(self.review_selection==i+1,format!("Photo {}",i+1)).clicked() { self.review_selection=i+1; }
                            });
                        }
                    });
                });
            });
            ui.add_space(12.0);
            ui.allocate_ui_with_layout(Vec2::new(320.0,ui.available_height()),egui::Layout::top_down(egui::Align::LEFT),|ui| {
                egui::ScrollArea::vertical().id_salt("review_actions").show(ui,|ui| {
                    ui.set_max_width(310.0);
                    theme::card().fill(theme::SURFACE).show(ui,|ui| {
                        ui.set_width(270.0);
                                                ui.heading("Print photos");
                        ui.label("Choose your printer, paper size, and copies.");
                        ui.add_space(8.0);
                        if ui.add_enabled(self.print_image.is_some(),egui::Button::new(RichText::new("Print layout").size(19.0).color(theme::ON_ACCENT)).min_size(Vec2::new(265.0,56.0)).fill(theme::ACCENT).corner_radius(8)).clicked() {
                            if let Some(image)=&self.print_image {
                                self.print_feedback=Some(match printing::print_image(image,None) {
                                    Ok(PrintOutcome::Submitted{job_id}) => format!("Print job {job_id} sent to the printer"),
                                    Ok(PrintOutcome::Cancelled) => "Printing cancelled. Your photos are still here.".into(),
                                    Err(e) => format!("Printing failed: {e}"),
                                });
                            }
                        }
                        if let Some(feedback)=&self.print_feedback { ui.add_space(6.0); ui.label(feedback); }
                    });
                    ui.add_space(12.0);
                    theme::card().show(ui,|ui| {
                        ui.set_width(270.0);
                        theme::pill(ui,"COMING SOON",theme::SURFACE_HOVER);
                        ui.heading("Send photos");
                        ui.label("Email and text delivery will be available soon.");
                        ui.add_space(6.0);
                        ui.horizontal(|ui| { ui.selectable_value(&mut self.delivery_method,false,"Email"); ui.selectable_value(&mut self.delivery_method,true,"Text message"); });
                        ui.add_enabled_ui(false,|ui| {
                            ui.add(egui::TextEdit::singleline(&mut self.delivery_address).desired_width(255.0).hint_text(if self.delivery_method {"Phone number"} else {"Email address"}));
                            ui.add_sized([265.0,40.0],egui::Button::new("Send photos"));
                        });
                    });
                    ui.add_space(12.0);
                    if !fullscreen {
                        if let Some(path)=&self.last_strip { ui.add_space(8.0); ui.small(format!("Saved to {}",path.display())); }
                    }
                });
            });
        });
    }
}

impl eframe::App for SparkboothApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

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
        if !self.booth.capture_active() {
            egui::Panel::top("booth_header")
                .frame(
                    egui::Frame::new()
                        .fill(theme::BACKGROUND)
                        .inner_margin(egui::Margin::symmetric(20, 14)),
                )
                .show(ui, |ui| {
                    action = theme::header(
                        ui,
                        fullscreen,
                        self.booth.phase() == Phase::Preview,
                        self.show_editor,
                    );
                });
        }
        match action {
            HeaderAction::Fullscreen => self.set_fullscreen(&ctx, true),
            HeaderAction::ExitFullscreen => self.set_fullscreen(&ctx, false),
            HeaderAction::Settings => {
                self.show_settings = !self.show_settings;
                self.show_editor = false;
            }
            HeaderAction::Studio => {
                self.show_editor = !self.show_editor;
                self.show_settings = false;
            }
            HeaderAction::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            HeaderAction::None => {}
        }
        if !fullscreen && !self.booth.capture_active() {
            egui::Panel::bottom("operator_status")
                .frame(
                    egui::Frame::new()
                        .fill(theme::BACKGROUND)
                        .inner_margin(egui::Margin::symmetric(20, 6)),
                )
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        if let Some(name) = &self.camera_active {
                            ui.label(RichText::new(name).size(11.0).color(theme::MUTED));
                            ui.separator();
                        }
                        let text = self.camera_note.as_deref().unwrap_or(&self.status);
                        ui.add(
                            egui::Label::new(RichText::new(text).size(11.0).color(theme::MUTED))
                                .truncate(),
                        )
                        .on_hover_text(text);
                    });
                });
        }

        if !fullscreen
            && self.show_settings
            && !self.show_editor
            && self.booth.phase() == Phase::Preview
        {
            egui::Panel::right("settings_panel")
                .default_size(320.0)
                .show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("operator_settings_scroll")
                        .show(ui, |ui| {
                            ui.heading("Booth settings");
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
                            ui.monospace(self.settings_path.display().to_string());
                        });
                });
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
                if self.booth.capture_active() {
                    8
                } else if fullscreen {
                    24
                } else {
                    20
                },
            ))
            .show(ui, |ui| match self.booth.phase() {
                Phase::Review => self.review_page(ui, fullscreen),
                _ => self.booth_page(ui),
            });
    }
}
