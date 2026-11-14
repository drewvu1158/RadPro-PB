//! Editable design primitives and the shared CPU compositor used by preview,
//! booth output, and the flattened Sparkbooth fallback artwork.
use ab_glyph::{point, Font, FontArc, PxScale, ScaleFont};
use image::{DynamicImage, Pixel, Rgba, RgbaImage};
use std::sync::{Arc, OnceLock};

pub const MAX_PIXELS: u64 = 16_000_000;
pub type Color = [u8; 4];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub rotation: f64,
}

#[derive(Clone, Copy, Debug)]
pub struct Bounds {
    pub left: f64,
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
}
impl Bounds {
    pub fn x_anchors(self) -> [f64; 3] {
        [self.left, (self.left + self.right) / 2.0, self.right]
    }
    pub fn y_anchors(self) -> [f64; 3] {
        [self.top, (self.top + self.bottom) / 2.0, self.bottom]
    }
    pub fn translated(self, x: f64, y: f64) -> Self {
        Self {
            left: self.left + x,
            right: self.right + x,
            top: self.top + y,
            bottom: self.bottom + y,
        }
    }
}
impl Frame {
    pub fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self {
            x,
            y,
            width,
            height,
            rotation: 0.0,
        }
    }
    pub fn corners(self) -> [(f64, f64); 4] {
        let (s, c) = self.rotation.to_radians().sin_cos();
        [
            (0.0, 0.0),
            (self.width, 0.0),
            (self.width, self.height),
            (0.0, self.height),
        ]
        .map(|(x, y)| (self.x + c * x - s * y, self.y + s * x + c * y))
    }
    pub fn bounds(self) -> Bounds {
        let c = self.corners();
        Bounds {
            left: c.iter().map(|p| p.0).fold(f64::INFINITY, f64::min),
            right: c.iter().map(|p| p.0).fold(f64::NEG_INFINITY, f64::max),
            top: c.iter().map(|p| p.1).fold(f64::INFINITY, f64::min),
            bottom: c.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max),
        }
    }
    pub fn local(self, x: f64, y: f64) -> (f64, f64) {
        let (s, c) = self.rotation.to_radians().sin_cos();
        let (dx, dy) = (x - self.x, y - self.y);
        (c * dx + s * dy, -s * dx + c * dy)
    }
    pub fn contains(self, x: f64, y: f64) -> bool {
        let (x, y) = self.local(x, y);
        x >= 0.0 && y >= 0.0 && x < self.width && y < self.height
    }
    pub fn validate(self) -> Result<(), String> {
        if ![self.x, self.y, self.width, self.height, self.rotation]
            .iter()
            .all(|v| v.is_finite())
            || self.x.abs() > 16384.0
            || self.y.abs() > 16384.0
            || self.width < 1.0
            || self.height < 1.0
            || self.width > 8192.0
            || self.height > 8192.0
            || self.width * self.height > MAX_PIXELS as f64
        {
            return Err("Layers need finite positions and sizes of 1–8192 px within the 16 megapixel budget".into());
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackgroundStyle {
    Solid,
    Gradient,
    Radial,
    Dots,
    Checker,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Background {
    pub color: Color,
    pub secondary: Color,
    pub style: BackgroundStyle,
    pub angle: f64,
    pub spacing: f64,
}
impl Default for Background {
    fn default() -> Self {
        Self {
            color: [255; 4],
            secondary: [226, 215, 203, 255],
            style: BackgroundStyle::Solid,
            angle: 90.0,
            spacing: 60.0,
        }
    }
}
impl Background {
    pub fn render(&self, w: u32, h: u32, scale: f64) -> RgbaImage {
        let (s, c) = self.angle.to_radians().sin_cos();
        let extent = (c.abs() * w as f64 + s.abs() * h as f64).max(1.0);
        RgbaImage::from_fn(w, h, |x, y| {
            let t = match self.style {
                BackgroundStyle::Solid => 0.0,
                BackgroundStyle::Gradient => {
                    (((x as f64 - w as f64 / 2.0) * c + (y as f64 - h as f64 / 2.0) * s) / extent
                        + 0.5)
                        .clamp(0.0, 1.0)
                }
                BackgroundStyle::Radial => (((x as f64 / w as f64 - 0.5).powi(2)
                    + (y as f64 / h as f64 - 0.5).powi(2))
                .sqrt()
                    * 1.42)
                    .min(1.0),
                BackgroundStyle::Dots => {
                    let d = (self.spacing * scale).max(2.0);
                    let dx = (x as f64 % d - d / 2.0) / d;
                    let dy = (y as f64 % d - d / 2.0) / d;
                    if dx * dx + dy * dy < 0.016 {
                        1.0
                    } else {
                        0.0
                    }
                }
                BackgroundStyle::Checker => {
                    let d = (self.spacing * scale).max(2.0);
                    (((x as f64 / d) as u32 + (y as f64 / d) as u32) % 2) as f64
                }
            };
            let mut color = [255; 4];
            for (i, out) in color.iter_mut().enumerate().take(3) {
                *out =
                    (self.color[i] as f64 * (1.0 - t) + self.secondary[i] as f64 * t).round() as u8;
            }
            Rgba(color)
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FontFamily {
    Sans,
    Serif,
    Mono,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextAlign {
    Left,
    Center,
    Right,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageFit {
    Cover,
    Contain,
    Stretch,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShapeKind {
    Rectangle,
    Ellipse,
    Heart,
    Star,
    Diamond,
}
#[derive(Clone)]
pub struct ImageAsset {
    pub pixels: Arc<RgbaImage>,
    pub bytes: Arc<[u8]>,
}
#[derive(Clone)]
pub enum LayerKind {
    Photo {
        shot: u32,
        source: crate::amf::Object,
    },
    Text {
        text: String,
        family: FontFamily,
        size: f64,
        color: Color,
        align: TextAlign,
        bold: bool,
        italic: bool,
        spacing: f64,
        line_height: f64,
    },
    Image {
        asset: ImageAsset,
        fit: ImageFit,
        flip_x: bool,
        flip_y: bool,
    },
    Shape {
        shape: ShapeKind,
        color: Color,
    },
}
#[derive(Clone)]
pub struct Layer {
    pub name: String,
    pub frame: Frame,
    pub visible: bool,
    pub locked: bool,
    pub opacity: f64,
    pub rounding: f64,
    pub border: f64,
    pub border_color: Color,
    pub kind: LayerKind,
}
impl Layer {
    pub fn new(name: impl Into<String>, frame: Frame, kind: LayerKind) -> Self {
        Self {
            name: name.into(),
            frame,
            visible: true,
            locked: false,
            opacity: 1.0,
            rounding: 0.0,
            border: 0.0,
            border_color: [255; 4],
            kind,
        }
    }
    pub fn photo(frame: Frame, shot: u32) -> Self {
        Self::new(
            format!("Photo {}", shot + 1),
            frame,
            LayerKind::Photo {
                shot,
                source: Default::default(),
            },
        )
    }
    pub fn text(text: impl Into<String>, frame: Frame) -> Self {
        let text = text.into();
        Self::new(
            "Text",
            frame,
            LayerKind::Text {
                text,
                family: FontFamily::Sans,
                size: 60.0,
                color: [37, 33, 29, 255],
                align: TextAlign::Center,
                bold: false,
                italic: false,
                spacing: 0.0,
                line_height: 1.2,
            },
        )
    }
    pub fn shape(shape: ShapeKind, frame: Frame, color: Color) -> Self {
        Self::new(
            format!("{shape:?}"),
            frame,
            LayerKind::Shape { shape, color },
        )
    }
    pub fn is_photo(&self) -> bool {
        matches!(self.kind, LayerKind::Photo { .. })
    }
    pub fn kind_name(&self) -> &'static str {
        match self.kind {
            LayerKind::Photo { .. } => "PHOTO",
            LayerKind::Text { .. } => "TEXT",
            LayerKind::Image { .. } => "IMAGE",
            LayerKind::Shape { .. } => "SHAPE",
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        self.frame.validate()?;
        if self.name.len() > 256
            || !self.opacity.is_finite()
            || !(0.0..=1.0).contains(&self.opacity)
            || !self.rounding.is_finite()
            || !(0.0..=4096.0).contains(&self.rounding)
            || !self.border.is_finite()
            || !(0.0..=1024.0).contains(&self.border)
        {
            return Err("Invalid layer style or layer name (maximum 256 bytes)".into());
        }
        match &self.kind {
            LayerKind::Photo { shot, .. }
                if *shot >= 8
                    || self.opacity != 1.0
                    || self.rounding != 0.0
                    || self.border != 0.0 =>
            {
                Err("Photo layers use shots 1–8, full opacity, and rectangular frames".into())
            }
            LayerKind::Text {
                text,
                size,
                spacing,
                line_height,
                ..
            } if text.len() > 4096
                || !size.is_finite()
                || !(1.0..=1024.0).contains(size)
                || !spacing.is_finite()
                || !(-50.0..=200.0).contains(spacing)
                || !line_height.is_finite()
                || !(0.5..=3.0).contains(line_height) =>
            {
                Err("Text supports up to 4096 bytes, 1–1024 px type, and valid spacing".into())
            }
            _ => Ok(()),
        }
    }
    /// Visit only the rotated layer's visible canvas bounds. Rotation has the
    /// same top-left origin as SBL/Flash. Photo pixels remain opaque, allowing
    /// arbitrary layer stacks to be projected into legacy SBL artwork.
    pub fn paint(
        &self,
        canvas_size: (u32, u32),
        scale: f64,
        shots: &[DynamicImage],
        mut put: impl FnMut(u32, u32, Rgba<u8>),
    ) {
        if !self.visible {
            return;
        }
        let w = (self.frame.width * scale).round().max(1.0) as u32;
        let h = (self.frame.height * scale).round().max(1.0) as u32;
        let mut tile = match &self.kind {
            LayerKind::Photo { shot, .. } => shots
                .get(*shot as usize)
                .map(|s| crate::layout::cover_crop_resize(s, (w, h)))
                .unwrap_or_else(|| {
                    let colors = [
                        [68, 94, 128, 255],
                        [79, 120, 116, 255],
                        [134, 105, 76, 255],
                        [120, 84, 112, 255],
                    ];
                    RgbaImage::from_pixel(w, h, Rgba(colors[*shot as usize % 4]))
                }),
            LayerKind::Image {
                asset,
                fit,
                flip_x,
                flip_y,
            } => {
                let mut image = match fit {
                    ImageFit::Cover => crate::layout::cover_crop_resize(
                        &DynamicImage::ImageRgba8((*asset.pixels).clone()),
                        (w, h),
                    ),
                    ImageFit::Stretch => image::imageops::resize(
                        asset.pixels.as_ref(),
                        w,
                        h,
                        image::imageops::FilterType::Triangle,
                    ),
                    ImageFit::Contain => {
                        let factor = (w as f64 / asset.pixels.width() as f64)
                            .min(h as f64 / asset.pixels.height() as f64);
                        let scaled = image::imageops::resize(
                            asset.pixels.as_ref(),
                            (asset.pixels.width() as f64 * factor).round().max(1.0) as u32,
                            (asset.pixels.height() as f64 * factor).round().max(1.0) as u32,
                            image::imageops::FilterType::Triangle,
                        );
                        let mut image = RgbaImage::new(w, h);
                        image::imageops::overlay(
                            &mut image,
                            &scaled,
                            ((w - scaled.width()) / 2) as i64,
                            ((h - scaled.height()) / 2) as i64,
                        );
                        image
                    }
                };
                if *flip_x {
                    image::imageops::flip_horizontal_in_place(&mut image);
                }
                if *flip_y {
                    image::imageops::flip_vertical_in_place(&mut image);
                }
                image
            }
            LayerKind::Shape { color, .. } => RgbaImage::from_pixel(w, h, Rgba(*color)),
            LayerKind::Text { .. } => self.render_text(w, h, scale),
        };
        if self.is_photo() {
            for p in tile.pixels_mut() {
                p.0[3] = 255;
            }
        }
        let shape = if let LayerKind::Shape { shape, .. } = self.kind {
            shape
        } else {
            ShapeKind::Rectangle
        };
        let radius = (self.rounding * scale).min(w.min(h) as f64 / 2.0);
        let border = self.border * scale;
        for (x, y, p) in tile.enumerate_pixels_mut() {
            let inside = shape_contains(
                shape,
                x as f64 + 0.5,
                y as f64 + 0.5,
                w as f64,
                h as f64,
                radius,
            );
            if !inside {
                p.0[3] = 0;
            } else {
                if border > 0.0
                    && !shape_contains(
                        shape,
                        x as f64 + 0.5 - border,
                        y as f64 + 0.5 - border,
                        w as f64 - 2.0 * border,
                        h as f64 - 2.0 * border,
                        (radius - border).max(0.0),
                    )
                {
                    p.blend(&Rgba(self.border_color));
                }
                p.0[3] = (p.0[3] as f64 * self.opacity).round() as u8;
            }
        }
        let mut frame = self.frame;
        frame.x *= scale;
        frame.y *= scale;
        frame.width = w as f64;
        frame.height = h as f64;
        let bounds = frame.bounds();
        let (cw, ch) = canvas_size;
        for y in bounds.top.floor().clamp(0.0, ch as f64) as u32
            ..bounds.bottom.ceil().clamp(0.0, ch as f64) as u32
        {
            for x in bounds.left.floor().clamp(0.0, cw as f64) as u32
                ..bounds.right.ceil().clamp(0.0, cw as f64) as u32
            {
                let (px, py) = frame.local(x as f64 + 0.5, y as f64 + 0.5);
                if px >= 0.0 && py >= 0.0 && px < w as f64 && py < h as f64 {
                    let p = *tile.get_pixel(px as u32, py as u32);
                    if p[3] > 0 {
                        put(x, y, p);
                    }
                }
            }
        }
    }
    fn render_text(&self, w: u32, h: u32, scale: f64) -> RgbaImage {
        let LayerKind::Text {
            text,
            family,
            size,
            color,
            align,
            bold,
            italic,
            spacing,
            line_height,
        } = &self.kind
        else {
            unreachable!()
        };
        let font = design_font(*family);
        let size = (*size * scale) as f32;
        let scaled = font.as_scaled(PxScale::from(size));
        let tracking = (*spacing * scale) as f32;
        let measure = |s: &str| {
            let mut last = None;
            let mut width = 0.0;
            for c in s.chars() {
                let id = scaled.glyph_id(c);
                if let Some(last) = last {
                    width += scaled.kern(last, id);
                }
                width += scaled.h_advance(id) + tracking;
                last = Some(id);
            }
            (width - tracking).max(0.0)
        };
        let mut lines = Vec::new();
        for paragraph in text.split('\n') {
            let mut line = String::new();
            for word in paragraph.split_inclusive(char::is_whitespace) {
                if !line.is_empty() && measure(&(line.clone() + word.trim_end())) > w as f32 {
                    lines.push(line.trim_end().to_owned());
                    line.clear();
                }
                for c in word.chars() {
                    let mut next = line.clone();
                    next.push(c);
                    if !line.is_empty() && measure(&next) > w as f32 {
                        lines.push(line.trim_end().to_owned());
                        line.clear();
                    }
                    line.push(c);
                }
            }
            lines.push(line.trim_end().to_owned());
        }
        let mut tile = RgbaImage::new(w, h);
        for (row, line) in lines.iter().enumerate() {
            let baseline = scaled.ascent() + row as f32 * size * (*line_height as f32);
            if baseline - scaled.ascent() > h as f32 {
                break;
            }
            let mut x = match align {
                TextAlign::Left => 0.0,
                TextAlign::Center => (w as f32 - measure(line)) / 2.0,
                TextAlign::Right => w as f32 - measure(line),
            };
            let mut previous = None;
            for c in line.chars() {
                let id = scaled.glyph_id(c);
                if let Some(prev) = previous {
                    x += scaled.kern(prev, id);
                }
                let glyph = id.with_scale_and_position(size, point(x, baseline));
                if let Some(outline) = font.outline_glyph(glyph) {
                    let bounds = outline.px_bounds();
                    outline.draw(|gx, gy, coverage| {
                        let py = gy as i32 + bounds.min.y as i32;
                        let slant = if *italic {
                            ((baseline - py as f32) * 0.18) as i32
                        } else {
                            0
                        };
                        let px = gx as i32 + bounds.min.x as i32 + slant;
                        let weight = if *bold {
                            (size / 28.0).round().clamp(1.0, 6.0) as i32
                        } else {
                            0
                        };
                        for dx in 0..=weight {
                            if px + dx >= 0 && py >= 0 && px + dx < w as i32 && py < h as i32 {
                                let mut c = *color;
                                c[3] = (color[3] as f32 * coverage).round() as u8;
                                tile.get_pixel_mut((px + dx) as u32, py as u32)
                                    .blend(&Rgba(c));
                            }
                        }
                    });
                }
                x += scaled.h_advance(id) + tracking;
                previous = Some(id);
            }
        }
        tile
    }
}

fn design_font(family: FontFamily) -> &'static FontArc {
    static FONTS: OnceLock<[FontArc; 3]> = OnceLock::new();
    let fonts = FONTS.get_or_init(|| {
        let sans =
            FontArc::try_from_slice(epaint_default_fonts::UBUNTU_LIGHT).expect("bundled font");
        let mono =
            FontArc::try_from_slice(epaint_default_fonts::HACK_REGULAR).expect("bundled font");
        let serif = std::env::var_os("WINDIR")
            .map(std::path::PathBuf::from)
            .and_then(|d| std::fs::read(d.join("Fonts/georgia.ttf")).ok())
            .and_then(|b| FontArc::try_from_vec(b).ok())
            .unwrap_or_else(|| sans.clone());
        [sans, serif, mono]
    });
    &fonts[match family {
        FontFamily::Sans => 0,
        FontFamily::Serif => 1,
        FontFamily::Mono => 2,
    }]
}

fn shape_contains(shape: ShapeKind, x: f64, y: f64, w: f64, h: f64, r: f64) -> bool {
    if w <= 0.0 || h <= 0.0 || x < 0.0 || y < 0.0 || x >= w || y >= h {
        return false;
    }
    let nx = (x / w - 0.5) * 2.0;
    let ny = (y / h - 0.5) * 2.0;
    match shape {
        ShapeKind::Rectangle => {
            let dx = (x - w / 2.0).abs() - (w / 2.0 - r);
            let dy = (y - h / 2.0).abs() - (h / 2.0 - r);
            dx.max(0.0).powi(2) + dy.max(0.0).powi(2) <= r * r
        }
        ShapeKind::Ellipse => nx * nx + ny * ny <= 1.0,
        ShapeKind::Diamond => nx.abs() + ny.abs() <= 1.0,
        ShapeKind::Heart => {
            let a = nx * 1.2;
            let b = -ny * 1.2 + 0.25;
            (a * a + b * b - 1.0).powi(3) - a * a * b.powi(3) <= 0.0
        }
        ShapeKind::Star => {
            let mut inside = false;
            const V: [(f64, f64); 10] = [
                (0.0, -1.0),
                (0.253, -0.348),
                (0.951, -0.309),
                (0.409, 0.133),
                (0.588, 0.809),
                (0.0, 0.43),
                (-0.588, 0.809),
                (-0.409, 0.133),
                (-0.951, -0.309),
                (-0.253, -0.348),
            ];
            for i in 0..10 {
                let (ax, ay) = V[i];
                let (bx, by) = V[(i + 1) % 10];
                if (ay > ny) != (by > ny) && nx < (bx - ax) * (ny - ay) / (by - ay) + ax {
                    inside = !inside;
                }
            }
            inside
        }
    }
}
