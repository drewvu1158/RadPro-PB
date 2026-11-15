//! The shared dark palette, typography and navigation for Radiate Photobooth.
use eframe::egui::{self, Color32, RichText, Stroke, Vec2};

pub const BACKGROUND: Color32 = Color32::BLACK;
pub const SURFACE: Color32 = Color32::from_rgb(18, 18, 19);
pub const SURFACE_HOVER: Color32 = Color32::from_rgb(33, 32, 35);
pub const TEXT: Color32 = Color32::from_rgb(247, 243, 237);
pub const MUTED: Color32 = Color32::from_rgb(166, 163, 160);
pub const ACCENT: Color32 = Color32::from_rgb(229, 181, 149);
pub const ON_ACCENT: Color32 = Color32::from_rgb(36, 28, 23);
pub const BORDER: Color32 = Color32::from_rgb(48, 45, 43);
pub const LIVE: Color32 = Color32::from_rgb(166, 195, 158);

pub fn editorial(size: f32) -> egui::FontId {
    egui::FontId::new(size, egui::FontFamily::Name("editorial".into()))
}

pub fn button_font(size: f32) -> egui::FontId {
    egui::FontId::new(size, egui::FontFamily::Name("interface_medium".into()))
}

pub fn install(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    // Load installed Windows faces locally; bundled fonts remain the fallback.
    let font_dir = std::env::var_os("WINDIR")
        .map(std::path::PathBuf::from)
        .map(|dir| dir.join("Fonts"));
    if let Some(bytes) = font_dir
        .as_ref()
        .and_then(|dir| std::fs::read(dir.join("segoeui.ttf")).ok())
    {
        fonts
            .font_data
            .insert("interface".into(), egui::FontData::from_owned(bytes).into());
        fonts
            .families
            .get_mut(&egui::FontFamily::Proportional)
            .unwrap()
            .insert(0, "interface".into());
    }
    let mut medium_fonts = fonts.families[&egui::FontFamily::Proportional].clone();
    if let Some(bytes) = font_dir
        .as_ref()
        .and_then(|dir| std::fs::read(dir.join("segoeuib.ttf")).ok())
    {
        fonts.font_data.insert(
            "interface_bold".into(),
            egui::FontData::from_owned(bytes).into(),
        );
        medium_fonts.insert(0, "interface_bold".into());
    }
    fonts.families.insert(
        egui::FontFamily::Name("interface_medium".into()),
        medium_fonts,
    );
    let mut heading_fonts = fonts.families[&egui::FontFamily::Proportional].clone();
    // System fonts are read locally and are never copied into the app package.
    if let Some(bytes) = font_dir
        .as_ref()
        .and_then(|dir| std::fs::read(dir.join("georgia.ttf")).ok())
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
    v.window_corner_radius = 16.into();
    v.menu_corner_radius = 8.into();
    for widget in [
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        widget.corner_radius = 10.into();
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
    style.spacing.button_padding = Vec2::new(16.0, 11.0);
    style.spacing.interact_size.y = 32.0;
    style
        .text_styles
        .insert(egui::TextStyle::Body, egui::FontId::proportional(16.0));
    style
        .text_styles
        .insert(egui::TextStyle::Button, button_font(15.0));
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
        .corner_radius(14)
        .inner_margin(20)
}

/// Quiet, letterspaced labels contrast with the editorial headings.
pub fn eyebrow(text: impl Into<String>) -> RichText {
    RichText::new(text)
        .font(button_font(10.0))
        .extra_letter_spacing(1.8)
        .color(ACCENT)
}

pub fn brand(ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(32.0), egui::Sense::hover());
        let painter = ui.painter();
        painter.circle_stroke(rect.center(), 12.0, Stroke::new(1.0, ACCENT));
        painter.circle_stroke(rect.center(), 5.0, Stroke::new(1.0, ACCENT));
        for index in 0..6 {
            let angle = index as f32 * std::f32::consts::TAU / 6.0;
            let direction = Vec2::angled(angle);
            painter.line_segment(
                [
                    rect.center() + direction * 5.0,
                    rect.center() + Vec2::angled(angle + 0.5) * 12.0,
                ],
                Stroke::new(1.0, ACCENT),
            );
        }
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            ui.label(RichText::new("Radiate").font(editorial(23.0)).color(TEXT));
            ui.label(eyebrow("PHOTO BOOTH").size(8.0));
        });
    });
}

pub fn primary_button(ui: &mut egui::Ui, text: &str, size: Vec2, enabled: bool) -> egui::Response {
    ui.scope(|ui| {
        let widgets = &mut ui.visuals_mut().widgets;
        widgets.inactive.weak_bg_fill = ACCENT;
        widgets.hovered.weak_bg_fill = Color32::from_rgb(245, 204, 174);
        widgets.active.weak_bg_fill = Color32::from_rgb(209, 157, 123);
        for widget in [
            &mut widgets.inactive,
            &mut widgets.hovered,
            &mut widgets.active,
        ] {
            widget.bg_stroke = Stroke::NONE;
            widget.fg_stroke = Stroke::new(1.0, ON_ACCENT);
        }
        ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
            ui.add_enabled(
                enabled,
                egui::Button::new(
                    RichText::new(text)
                        .font(button_font(if size.y >= 60.0 { 21.0 } else { 16.0 }))
                        .color(ON_ACCENT),
                )
                .min_size(size)
                .corner_radius(30),
            )
        })
        .inner
    })
    .inner
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum HeaderAction {
    None,
    Fullscreen,
    Settings,
    Studio,
    Quit,
}

/// Operator header for the windowed booth. Guest fullscreen hides it; the
/// booth page itself carries the take and exit-fullscreen controls.
pub fn header(ui: &mut egui::Ui, idle: bool, studio: bool) -> HeaderAction {
    let mut action = HeaderAction::None;
    // Keep operator actions readable at portrait widths; the guest wordmark
    // lives in the page masthead.
    let compact = ui.available_width() < 980.0;
    ui.horizontal(|ui| {
        ui.style_mut()
            .text_styles
            .insert(egui::TextStyle::Button, egui::FontId::proportional(13.0));
        ui.spacing_mut().button_padding = Vec2::new(12.0, 9.0);
        let (r, _) = ui.allocate_exact_size(Vec2::splat(8.0), egui::Sense::hover());
        ui.painter().circle_filled(r.center(), 3.0, ACCENT);
        if !compact {
            ui.label(eyebrow("HOST CONTROLS").color(MUTED));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // Right-to-left: Quit ends up at the far right.
            if ui.button("Quit").clicked() {
                action = HeaderAction::Quit;
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
            if ui
                .add_enabled(idle, egui::Button::new("Settings"))
                .clicked()
            {
                action = HeaderAction::Settings;
            }
            if ui.button("Go fullscreen").clicked() {
                action = HeaderAction::Fullscreen;
            }
        });
    });
    action
}
