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

pub fn camera_mark(painter: &egui::Painter, center: egui::Pos2, radius: f32, color: Color32) {
    painter.circle_stroke(center, radius, Stroke::new(1.2, color));
    painter.circle_stroke(center, radius * 0.42, Stroke::new(1.2, color));
    for index in 0..6 {
        let angle = index as f32 * std::f32::consts::TAU / 6.0;
        painter.line_segment(
            [
                center + Vec2::angled(angle) * radius * 0.42,
                center + Vec2::angled(angle + 0.5) * radius,
            ],
            Stroke::new(1.0, color),
        );
    }
}

/// A translucent welcome mark painted only over the idle live preview.
pub fn paint_brand(painter: &egui::Painter, rect: egui::Rect) {
    if rect.width() < 130.0 || rect.height() < 90.0 {
        return;
    }
    let size = (rect.width() * 0.075)
        .min(rect.height() * 0.18)
        .clamp(18.0, 44.0);
    let center = rect.center();
    let accent = Color32::from_rgba_unmultiplied(ACCENT.r(), ACCENT.g(), ACCENT.b(), 130);
    camera_mark(
        painter,
        center - Vec2::new(0.0, size * 1.2),
        size * 0.36,
        accent,
    );
    painter.text(
        center,
        egui::Align2::CENTER_CENTER,
        "Radiate",
        editorial(size),
        Color32::from_rgba_unmultiplied(TEXT.r(), TEXT.g(), TEXT.b(), 135),
    );
    painter.text(
        center + Vec2::new(0.0, size * 0.82),
        egui::Align2::CENTER_CENTER,
        "PHOTOBOOTH",
        button_font((size * 0.24).max(8.0)),
        accent,
    );
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
    Library,
    Studio,
    Quit,
}

/// All host controls live in this popout, including fullscreen escape.
pub fn operator_menu(
    ui: &mut egui::Ui,
    idle: bool,
    studio: bool,
    fullscreen: bool,
) -> HeaderAction {
    let mut action = HeaderAction::None;
    ui.menu_button(RichText::new("Menu").size(13.0), |ui| {
        ui.set_min_width(180.0);
        if ui
            .button(if fullscreen {
                "Exit fullscreen"
            } else {
                "Go fullscreen"
            })
            .clicked()
        {
            action = HeaderAction::Fullscreen;
            ui.close();
        }
        if ui
            .add_enabled(idle, egui::Button::new("Settings"))
            .clicked()
        {
            action = HeaderAction::Settings;
            ui.close();
        }
        if ui
            .add_enabled(idle, egui::Button::new("Layout library"))
            .clicked()
        {
            action = HeaderAction::Library;
            ui.close();
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
            ui.close();
        }
        ui.separator();
        if ui.button("Quit").clicked() {
            action = HeaderAction::Quit;
            ui.close();
        }
    })
    .response
    .on_hover_text("Booth controls");
    action
}
