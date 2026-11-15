//! Native library browser. Network/decoding work runs outside the UI thread.

use crate::{library, presets, template::Template, ui_theme as theme};
use eframe::egui::{self, RichText, TextureHandle, Vec2};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread::JoinHandle;

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Section {
    #[default]
    BuiltIn,
    Downloaded,
    Cloud,
}

pub enum LibraryAction {
    Apply(Template),
    Studio(Template),
    Back,
}

struct Selection {
    template: Template,
    texture: TextureHandle,
    path: Option<PathBuf>,
}

enum Loaded {
    Catalog(Vec<library::CloudLayout>),
    Template(Template, image::RgbaImage, Option<PathBuf>),
}

struct Job {
    receiver: mpsc::Receiver<Result<Loaded, String>>,
    handle: JoinHandle<()>,
}

#[derive(Default)]
pub struct LibraryView {
    section: Section,
    folder: PathBuf,
    catalog_url: String,
    catalog: Vec<library::CloudLayout>,
    catalog_loaded: bool,
    local: Vec<library::LocalLayout>,
    selected: Option<Selection>,
    job: Option<Job>,
    pub message: String,
}

fn preview(template: Template, path: Option<PathBuf>) -> Result<Loaded, String> {
    let image = template.render(&[], Some(720))?.to_rgba8();
    Ok(Loaded::Template(template, image, path))
}

impl LibraryView {
    pub fn busy(&self) -> bool {
        self.job.is_some()
    }

    pub fn open(&mut self, folder: &Path, catalog_url: &str) {
        if self.busy() {
            return;
        }
        if self.folder != folder {
            self.folder = folder.to_owned();
            self.selected = None;
        }
        if self.catalog_url != catalog_url.trim() {
            self.catalog_url = catalog_url.trim().to_owned();
            self.catalog.clear();
            self.catalog_loaded = false;
        }
        self.refresh_local();
    }

    fn refresh_local(&mut self) {
        match library::local_layouts(&self.folder) {
            Ok(local) => self.local = local,
            Err(e) => {
                self.local.clear();
                self.message = e;
            }
        }
    }

    fn start(
        &mut self,
        ctx: &egui::Context,
        message: &str,
        work: impl FnOnce() -> Result<Loaded, String> + Send + 'static,
    ) {
        if self.busy() {
            return;
        }
        let (sender, receiver) = mpsc::channel();
        let ctx = ctx.clone();
        match std::thread::Builder::new()
            .name("layout-library".into())
            .spawn(move || {
                let _ = sender.send(work());
                ctx.request_repaint();
            }) {
            Ok(handle) => {
                self.job = Some(Job { receiver, handle });
                self.message = message.into();
            }
            Err(e) => self.message = format!("Could not start the library worker: {e}"),
        }
    }

    fn fetch_catalog(&mut self, ctx: &egui::Context) {
        let url = self.catalog_url.clone();
        self.start(ctx, "Loading the cloud library…", move || {
            library::fetch_catalog(&url).map(Loaded::Catalog)
        });
    }

    pub fn poll(&mut self, ctx: &egui::Context) {
        let Some(job) = &self.job else { return };
        let result = match job.receiver.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => {
                Err("The library worker stopped. Please try again.".into())
            }
        };
        let job = self.job.take().unwrap();
        let _ = job.handle.join();
        match result {
            Ok(Loaded::Catalog(entries)) => {
                self.message = format!(
                    "{} {} available. Select one to download it.",
                    entries.len(),
                    if entries.len() == 1 {
                        "layout"
                    } else {
                        "layouts"
                    }
                );
                self.catalog = entries;
                self.catalog_loaded = true;
            }
            Ok(Loaded::Template(template, image, path)) => {
                let texture = ctx.load_texture(
                    "library-preview",
                    egui::ColorImage::from_rgba_unmultiplied(
                        [image.width() as usize, image.height() as usize],
                        image.as_raw(),
                    ),
                    egui::TextureOptions::LINEAR,
                );
                self.message = match &path {
                    Some(path) => format!("Available offline: {}", path.display()),
                    None => {
                        "Preview ready. Apply this layout or make it your own in Layout studio."
                            .into()
                    }
                };
                self.selected = Some(Selection {
                    template,
                    texture,
                    path,
                });
                self.refresh_local();
            }
            Err(e) => self.message = e,
        }
    }

    fn list(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let busy = self.busy();
        ui.add_enabled_ui(!busy, |ui| match self.section {
            Section::BuiltIn => {
                ui.small("Original designs, ready to customize.");
                for (index, (name, description)) in presets::PRESETS.iter().enumerate() {
                    if ui.add_sized([ui.available_width(), 40.0], egui::Button::new(*name)).clicked() {
                        self.selected = None;
                        self.start(&ctx, "Preparing the preview…", move || preview(presets::build(index), None));
                    }
                    ui.label(RichText::new(*description).size(12.0).color(theme::MUTED));
                    ui.add_space(4.0);
                }
            }
            Section::Downloaded => {
                if ui.button("Refresh folder").clicked() {
                    self.refresh_local();
                }
                ui.small("Saved on this computer. No connection needed.");
                if self.local.is_empty() {
                    ui.label("No downloaded layouts yet. Select a design in Cloud to save it here.");
                }
                let mut chosen = None;
                for layout in &self.local {
                    if ui.add_sized([ui.available_width(), 40.0], egui::Button::new(&layout.name)).clicked() {
                        chosen = Some(layout.path.clone());
                    }
                }
                if let Some(path) = chosen {
                    self.selected = None;
                    self.start(&ctx, "Opening the saved layout…", move || preview(Template::load(&path)?, Some(path)));
                }
            }
            Section::Cloud => {
                if self.catalog_url.is_empty() {
                    ui.label("Your cloud library is ready to connect.");
                    ui.small("Add your catalog URL in Settings when your server is ready. Built-in and downloaded layouts are available now.");
                    return;
                }
                if ui.button("Refresh cloud library").clicked() {
                    self.fetch_catalog(&ctx);
                }
                ui.small("Select a layout to download it into your library folder.");
                if self.catalog_loaded && self.catalog.is_empty() {
                    ui.label("This catalog has no layouts yet.");
                }
                let mut chosen = None;
                for layout in &self.catalog {
                    if ui.add_sized([ui.available_width(), 40.0], egui::Button::new(&layout.name)).clicked() {
                        chosen = Some(layout.clone());
                    }
                    if !layout.description.is_empty() {
                        ui.label(RichText::new(&layout.description).size(12.0).color(theme::MUTED));
                    }
                    ui.add_space(4.0);
                }
                if let Some(layout) = chosen {
                    let folder = self.folder.clone();
                    self.selected = None;
                    self.start(&ctx, "Downloading and saving the layout…", move || {
                        let (template, path) = library::download(&layout, &folder)?;
                        preview(template, Some(path))
                    });
                }
            }
        });
    }

    fn selection(&self, ui: &mut egui::Ui, max_height: f32) -> Option<LibraryAction> {
        let Some(selected) = &self.selected else {
            theme::card().show(ui, |ui| {
                ui.heading("Find your next look.");
                ui.label("Choose a layout to see its preview here.");
            });
            return None;
        };
        ui.heading(&selected.template.name);
        ui.small(format!(
            "{} photos · {} × {}",
            selected.template.shots_needed(),
            selected.template.width,
            selected.template.height
        ));
        let size = selected.texture.size_vec2();
        let scale = (ui.available_width() / size.x)
            .min(max_height.max(80.0) / size.y)
            .min(1.0);
        ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
            ui.image((selected.texture.id(), size * scale));
        });
        let mut action = None;
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(
                    !self.busy(),
                    egui::Button::new(
                        RichText::new("Apply to booth")
                            .font(theme::button_font(16.0))
                            .color(theme::ON_ACCENT),
                    )
                    .fill(theme::ACCENT)
                    .corner_radius(30)
                    .min_size(Vec2::new(170.0, 42.0)),
                )
                .clicked()
            {
                action = Some(LibraryAction::Apply(selected.template.clone()));
            }
            if ui
                .add_enabled(!self.busy(), egui::Button::new("Open in Layout studio"))
                .clicked()
            {
                action = Some(LibraryAction::Studio(selected.template.clone()));
            }
        });
        if let Some(path) = &selected.path {
            ui.label(
                RichText::new(path.display().to_string())
                    .size(12.0)
                    .color(theme::MUTED),
            );
        }
        action
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) -> Option<LibraryAction> {
        let mut action = None;
        ui.horizontal(|ui| {
            ui.heading("Layout library");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Back to booth").clicked() {
                    action = Some(LibraryAction::Back);
                }
            });
        });
        ui.horizontal_wrapped(|ui| {
            let previous = self.section;
            ui.selectable_value(&mut self.section, Section::BuiltIn, "Built-in");
            ui.selectable_value(&mut self.section, Section::Downloaded, "Downloaded");
            ui.selectable_value(&mut self.section, Section::Cloud, "Cloud");
            if previous != self.section
                && self.section == Section::Cloud
                && !self.catalog_loaded
                && !self.catalog_url.is_empty()
                && !self.busy()
            {
                self.fetch_catalog(ui.ctx());
            }
        });
        if self.busy() {
            ui.spinner();
        }
        if !self.message.is_empty() {
            ui.label(RichText::new(&self.message).size(13.0).color(theme::ACCENT));
        }
        ui.add_space(8.0);
        let height = ui.available_height();
        if ui.available_width() >= 850.0 {
            let list_width = (ui.available_width() * 0.35).min(380.0);
            ui.horizontal_top(|ui| {
                ui.allocate_ui_with_layout(
                    Vec2::new(list_width, height),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        egui::ScrollArea::vertical()
                            .id_salt("library-list")
                            .show(ui, |ui| self.list(ui));
                    },
                );
                ui.separator();
                ui.allocate_ui_with_layout(
                    Vec2::new(ui.available_width(), height),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        egui::ScrollArea::vertical()
                            .id_salt("library-selection")
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                if let Some(selected) = self.selection(ui, height - 155.0) {
                                    action = Some(selected);
                                }
                            });
                    },
                );
            });
        } else {
            egui::ScrollArea::vertical()
                .id_salt("library-portrait")
                .show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("library-list")
                        .max_height((height * 0.36).max(120.0))
                        .show(ui, |ui| self.list(ui));
                    ui.separator();
                    if let Some(selected) = self.selection(ui, height * 0.46) {
                        action = Some(selected);
                    }
                });
        }
        action
    }
}

impl Drop for LibraryView {
    fn drop(&mut self) {
        // Keep ownership through finalization; a network operation has a 15 s total timeout.
        if let Some(job) = self.job.take() {
            let _ = job.handle.join();
        }
    }
}
