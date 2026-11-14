//! The shared dark palette, typography and navigation for Radiate Photobooth.
use eframe::egui::{self, Color32, RichText, Stroke, Vec2};

pub const BACKGROUND: Color32 = Color32::from_rgb(29, 28, 26);
pub const SURFACE: Color32 = Color32::from_rgb(39, 38, 35);
pub const SURFACE_HOVER: Color32 = Color32::from_rgb(53, 50, 46);
pub const TEXT: Color32 = Color32::from_rgb(236, 231, 222);
pub const MUTED: Color32 = Color32::from_rgb(169, 162, 152);
pub const ACCENT: Color32 = Color32::from_rgb(210, 154, 124);
pub const ON_ACCENT: Color32 = Color32::from_rgb(36, 28, 23);
pub const BORDER: Color32 = Color32::from_rgb(68, 64, 58);

pub fn editorial(size: f32) -> egui::FontId {
    egui::FontId::new(size, egui::FontFamily::Name("editorial".into()))
}

pub fn install(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let mut heading_fonts = fonts.families[&egui::FontFamily::Proportional].clone();
    // Use the host's serif font when available, with bundled-font fallback.
    // The Windows font is read locally and is never copied into the app package.
    if let Some(bytes) = std::env::var_os("WINDIR")
        .map(std::path::PathBuf::from)
        .and_then(|dir| std::fs::read(dir.join("Fonts/georgia.ttf")).ok())
    {
        fonts.font_data.insert(
            "system_serif".into(),
            egui::FontData::from_owned(bytes).into(),
        );
        heading_fonts.insert(0, "system_serif".into());
    }
    fonts
        .families
        .insert(egui::FontFamily::Name("editorial".into()), heading_fonts);
    ctx.set_fonts(fonts);
    ctx.set_theme(egui::Theme::Dark);
    let mut style = (*ctx.style_of(egui::Theme::Dark)).clone();
    let mut v = egui::Visuals::dark();
    v.override_text_color = Some(TEXT);
    v.panel_fill = BACKGROUND;
    v.window_fill = SURFACE;
    v.extreme_bg_color = BACKGROUND;
    v.faint_bg_color = SURFACE_HOVER;
    v.hyperlink_color = ACCENT;
    v.selection.bg_fill = SURFACE_HOVER;
    v.selection.stroke = Stroke::new(1.0, TEXT);
    v.window_corner_radius = 10.into();
    v.menu_corner_radius = 8.into();
    for widget in [
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        widget.corner_radius = 7.into();
        widget.bg_stroke = Stroke::new(1.0, BORDER);
        widget.fg_stroke = Stroke::new(1.0, TEXT);
    }
    v.widgets.inactive.bg_fill = SURFACE;
    v.widgets.inactive.weak_bg_fill = SURFACE;
    v.widgets.hovered.bg_fill = SURFACE_HOVER;
    v.widgets.hovered.weak_bg_fill = SURFACE_HOVER;
    v.widgets.active.bg_fill = SURFACE_HOVER;
    v.widgets.active.weak_bg_fill = SURFACE_HOVER;
    v.widgets.open.bg_fill = SURFACE_HOVER;
    v.widgets.open.weak_bg_fill = SURFACE_HOVER;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, MUTED);
    style.visuals = v;
    style.spacing.item_spacing = Vec2::new(10.0, 8.0);
    style.spacing.button_padding = Vec2::new(14.0, 10.0);
    style.spacing.interact_size.y = 32.0;
    style
        .text_styles
        .insert(egui::TextStyle::Body, egui::FontId::proportional(16.0));
    style
        .text_styles
        .insert(egui::TextStyle::Button, egui::FontId::proportional(15.0));
    style
        .text_styles
        .insert(egui::TextStyle::Heading, editorial(27.0));
    style
        .text_styles
        .insert(egui::TextStyle::Small, egui::FontId::proportional(12.0));
    ctx.set_style_of(egui::Theme::Dark, style);
}

pub fn card() -> egui::Frame {
    egui::Frame::new()
        .fill(SURFACE)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(10)
        .inner_margin(18)
}

pub fn pill(ui: &mut egui::Ui, text: &str, fill: Color32) {
    egui::Frame::new()
        .fill(fill)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(6)
        .inner_margin(egui::Margin::symmetric(10, 5))
        .show(ui, |ui| {
            ui.label(RichText::new(text).size(12.0).color(if fill == ACCENT {
                ON_ACCENT
            } else {
                TEXT
            }));
        });
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum HeaderAction {
    None,
    Fullscreen,
    ExitFullscreen,
    Settings,
    Studio,
    Quit,
}

/// Guest fullscreen contains one operator control; guest actions stay in the page.
pub fn header(ui: &mut egui::Ui, fullscreen: bool, idle: bool, studio: bool) -> HeaderAction {
    let mut action = HeaderAction::None;
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(Vec2::splat(8.0), egui::Sense::hover());
        ui.painter().circle_filled(r.center(), 3.0, ACCENT);
        ui.label(
            RichText::new("Radiate Photobooth")
                .font(editorial(22.0))
                .color(TEXT),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if fullscreen {
                if ui.button("Exit fullscreen").clicked() {
                    action = HeaderAction::ExitFullscreen;
                }
            } else {
                if ui.button("Go fullscreen").clicked() {
                    action = HeaderAction::Fullscreen;
                }
                if ui
                    .add_enabled(idle, egui::Button::new("Settings"))
                    .clicked()
                {
                    action = HeaderAction::Settings;
                }
                if ui
                    .add_enabled(
                        idle,
                        egui::Button::new(if studio {
                            "Back to booth"
                        } else {
                            "Layout studio"
                        }),
                    )
                    .clicked()
                {
                    action = HeaderAction::Studio;
                }
                if ui.button("Quit").clicked() {
                    action = HeaderAction::Quit;
                }
            }
        });
    });
    action
}
