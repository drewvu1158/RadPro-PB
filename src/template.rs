//! SBL interpretation, editable Radiate metadata, validation, and export.
use crate::{
    amf::{self, Object, Value},
    design::*,
};
use image::{DynamicImage, ImageReader, Pixel, Rgba, RgbaImage};
use std::{
    io::{Cursor, Read},
    path::Path,
    sync::Arc,
};

#[derive(Clone)]
pub struct Template {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub background: Background,
    /// Back to front; rendering, selection, and serialization share this order.
    pub layers: Vec<Layer>,
    raw: Object,
}
fn number(o: &Object, k: &str) -> Result<f64, String> {
    match o.get(k) {
        Some(Value::Number(n)) if n.is_finite() => Ok(*n),
        _ => Err(format!("Missing or invalid layout number: {k}")),
    }
}
fn integer(o: &Object, k: &str) -> Result<u32, String> {
    let n = number(o, k)?;
    if n < 0.0 || n > u32::MAX as f64 || n.fract() != 0.0 {
        return Err(format!("Invalid layout integer: {k}"));
    }
    Ok(n as u32)
}
fn string(o: &Object, k: &str) -> Result<String, String> {
    match o.get(k) {
        Some(Value::String(s)) => Ok(s.to_string()),
        _ => Err(format!("Missing layout text: {k}")),
    }
}
fn boolean(o: &Object, k: &str) -> Result<bool, String> {
    match o.get(k) {
        Some(Value::Bool(b)) => Ok(*b),
        _ => Err(format!("Invalid layout flag: {k}")),
    }
}
fn array<'a>(o: &'a Object, k: &str) -> Result<&'a [Value], String> {
    match o.get(k) {
        Some(Value::Array(a)) => Ok(a),
        None | Some(Value::Null) => Ok(&[]),
        _ => Err(format!("Invalid layout array: {k}")),
    }
}
fn object(v: &Value) -> Result<&Object, String> {
    match v {
        Value::Object(o) => Ok(o),
        _ => Err("Expected a layout object".into()),
    }
}
fn n(o: &mut Object, k: &str, v: impl Into<f64>) {
    o.insert(k.into(), Value::Number(v.into()));
}
fn s(o: &mut Object, k: &str, v: &str) {
    o.insert(k.into(), Value::String(Arc::from(v)));
}
fn b(o: &mut Object, k: &str, v: bool) {
    o.insert(k.into(), Value::Bool(v));
}
fn color(o: &Object, k: &str) -> Result<Color, String> {
    Ok(integer(o, k)?.to_be_bytes())
}
fn put_color(o: &mut Object, k: &str, v: Color) {
    n(o, k, u32::from_be_bytes(v));
}
fn read_bounded(path: &Path) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take(amf::MAX_FILE as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > amf::MAX_FILE {
        return Err("File exceeds the 32 MiB import budget".into());
    }
    Ok(bytes)
}
pub fn load_image(path: &Path) -> Result<ImageAsset, String> {
    decode_art(Arc::from(read_bounded(path)?))
}
fn decode_art(bytes: Arc<[u8]>) -> Result<ImageAsset, String> {
    let reader = || {
        ImageReader::new(Cursor::new(bytes.as_ref()))
            .with_guessed_format()
            .map_err(|e| e.to_string())
    };
    let (w, h) = reader()?
        .into_dimensions()
        .map_err(|e| format!("Artwork header: {e}"))?;
    if w == 0 || h == 0 || w > 8192 || h > 8192 || u64::from(w) * u64::from(h) > MAX_PIXELS {
        return Err("Artwork must fit within 8192 px per side and 16 megapixels".into());
    }
    let mut reader = reader()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(MAX_PIXELS * 4);
    reader.limits(limits);
    let pixels = Arc::new(
        reader
            .decode()
            .map_err(|e| format!("Could not decode artwork: {e}"))?
            .to_rgba8(),
    );
    Ok(ImageAsset { pixels, bytes })
}
fn frame(o: &Object) -> Result<Frame, String> {
    Ok(Frame {
        x: number(o, "x")?,
        y: number(o, "y")?,
        width: number(o, "width")?,
        height: number(o, "height")?,
        rotation: number(o, "rotation")?,
    })
}
fn put_frame(o: &mut Object, f: Frame) {
    for (k, v) in [
        ("x", f.x),
        ("y", f.y),
        ("width", f.width),
        ("height", f.height),
        ("rotation", f.rotation),
    ] {
        n(o, k, v);
    }
}
fn png(image: RgbaImage) -> Result<Arc<[u8]>, String> {
    let mut bytes = Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(image)
        .write_to(&mut bytes, image::ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    Ok(Arc::from(bytes.into_inner()))
}

impl Template {
    pub fn load(path: &Path) -> Result<Self, String> {
        Self::from_bytes(&read_bounded(path)?)
    }
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        let root = amf::decode(bytes)?;
        let raw = object(&root)?.clone();
        if integer(&raw, "version")? != 2 {
            return Err("This SBL version is not supported; import a version 2 layout".into());
        }
        if let Some(design) = raw.get("radiate_design") {
            return Self::from_design(object(design)?, raw.clone());
        }
        for key in ["texts", "images"] {
            if !array(&raw, key)?.is_empty() {
                return Err(format!("This Sparkbooth SBL has separate {key} layers with an unsupported encoding. Flatten those layers to PNG in Sparkbooth first; Radiate's own text and image layers remain editable."));
            }
        }
        let width = integer(&raw, "width")?;
        let height = integer(&raw, "height")?;
        let art = |embedded: &str, path: &str| -> Result<Option<Layer>, String> {
            match raw.get(embedded){
            Some(Value::Bytes(bytes)) if !bytes.is_empty()=>Ok(Some(Layer::new(path,Frame::new(0.0,0.0,width as f64,height as f64),LayerKind::Image{asset:decode_art(bytes.clone())?,fit:ImageFit::Stretch,flip_x:false,flip_y:false}))),
            None|Some(Value::Null)=>match raw.get(path){Some(Value::String(s)) if !s.is_empty()=>Err(format!("The SBL references {path} artwork without embedding it. Re-export with artwork included.")),_=>Ok(None)},
            _=>Err(format!("Invalid embedded SBL artwork: {embedded}")),
        }
        };
        let mut photos = Vec::new();
        for (i, value) in array(&raw, "photos")?.iter().enumerate() {
            let p = object(value)?;
            let order = if p.contains_key("orderindex") {
                integer(p, "orderindex")?
            } else {
                i as u32
            };
            let shot = integer(p, "photoindex")?;
            let mut layer = Layer::photo(frame(p)?, shot);
            layer.kind = LayerKind::Photo {
                shot,
                source: p.clone(),
            };
            photos.push((order, layer));
        }
        photos.sort_by_key(|p| p.0);
        let mut layers = Vec::new();
        if let Some(l) = art("backgroundimage", "background")? {
            layers.push(l);
        }
        layers.extend(photos.into_iter().map(|p| p.1));
        if let Some(l) = art("overlayimage", "overlay")? {
            layers.push(l);
        }
        let t = Self {
            name: string(&raw, "name").unwrap_or_else(|_| "Imported layout".into()),
            width,
            height,
            background: Background::default(),
            layers,
            raw,
        };
        t.validate()?;
        Ok(t)
    }
    fn from_design(o: &Object, raw: Object) -> Result<Self, String> {
        if integer(o, "version")? != 1 {
            return Err("This design needs a newer version of Radiate Photobooth".into());
        }
        let bg = object(o.get("background").ok_or("Missing design background")?)?;
        let style = match string(bg, "style")?.as_str() {
            "Solid" => BackgroundStyle::Solid,
            "Gradient" => BackgroundStyle::Gradient,
            "Radial" => BackgroundStyle::Radial,
            "Dots" => BackgroundStyle::Dots,
            "Checker" => BackgroundStyle::Checker,
            _ => return Err("Unknown background style".into()),
        };
        let mut layers = Vec::new();
        let mut decoded_pixels = 0u64;
        for v in array(o, "layers")? {
            if layers.len() >= 128 {
                return Err("A design supports up to 128 layers".into());
            }
            let l = object(v)?;
            let kind = match string(l, "kind")?.as_str() {
                "photo" => LayerKind::Photo {
                    shot: integer(l, "shot")?,
                    source: match l.get("source") {
                        Some(v) => object(v)?.clone(),
                        None => Object::new(),
                    },
                },
                "text" => LayerKind::Text {
                    text: string(l, "text")?,
                    family: match string(l, "family")?.as_str() {
                        "Sans" => FontFamily::Sans,
                        "Serif" => FontFamily::Serif,
                        "Mono" => FontFamily::Mono,
                        _ => return Err("Unknown typeface".into()),
                    },
                    size: number(l, "size")?,
                    color: color(l, "color")?,
                    align: match string(l, "align")?.as_str() {
                        "Left" => TextAlign::Left,
                        "Center" => TextAlign::Center,
                        "Right" => TextAlign::Right,
                        _ => return Err("Unknown text alignment".into()),
                    },
                    bold: boolean(l, "bold")?,
                    italic: boolean(l, "italic")?,
                    spacing: number(l, "spacing")?,
                    line_height: number(l, "line_height")?,
                },
                "image" => {
                    let Some(Value::Bytes(bytes)) = l.get("image") else {
                        return Err("Missing embedded image layer".into());
                    };
                    let (w, h) = ImageReader::new(Cursor::new(bytes.as_ref()))
                        .with_guessed_format()
                        .map_err(|e| e.to_string())?
                        .into_dimensions()
                        .map_err(|e| e.to_string())?;
                    decoded_pixels += u64::from(w) * u64::from(h);
                    if decoded_pixels > 32_000_000 {
                        return Err(
                            "Combined image layers exceed the 32 megapixel artwork budget".into(),
                        );
                    }
                    LayerKind::Image {
                        asset: decode_art(bytes.clone())?,
                        fit: match string(l, "fit")?.as_str() {
                            "Cover" => ImageFit::Cover,
                            "Contain" => ImageFit::Contain,
                            "Stretch" => ImageFit::Stretch,
                            _ => return Err("Unknown image fitting mode".into()),
                        },
                        flip_x: boolean(l, "flip_x")?,
                        flip_y: boolean(l, "flip_y")?,
                    }
                }
                "shape" => LayerKind::Shape {
                    shape: match string(l, "shape")?.as_str() {
                        "Rectangle" => ShapeKind::Rectangle,
                        "Ellipse" => ShapeKind::Ellipse,
                        "Heart" => ShapeKind::Heart,
                        "Star" => ShapeKind::Star,
                        "Diamond" => ShapeKind::Diamond,
                        _ => return Err("Unknown shape".into()),
                    },
                    color: color(l, "color")?,
                },
                _ => return Err("Unknown design layer type".into()),
            };
            layers.push(Layer {
                name: string(l, "name")?,
                frame: frame(l)?,
                visible: boolean(l, "visible")?,
                locked: boolean(l, "locked")?,
                opacity: number(l, "opacity")?,
                rounding: number(l, "rounding")?,
                border: number(l, "border")?,
                border_color: color(l, "border_color")?,
                kind,
            });
        }
        let t = Self {
            name: string(o, "name")?,
            width: integer(o, "width")?,
            height: integer(o, "height")?,
            background: Background {
                color: color(bg, "color")?,
                secondary: color(bg, "secondary")?,
                style,
                angle: number(bg, "angle")?,
                spacing: number(bg, "spacing")?,
            },
            layers,
            raw,
        };
        t.validate()?;
        Ok(t)
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.width == 0
            || self.height == 0
            || self.width > 8192
            || self.height > 8192
            || u64::from(self.width) * u64::from(self.height) > MAX_PIXELS
        {
            return Err("Canvas must fit within 8192 px per side and 16 megapixels".into());
        }
        if self.name.len() > 256
            || self.layers.len() > 128
            || self.photo_count() == 0
            || self.photo_count() > 64
        {
            return Err("A design needs 1–64 photo layers, up to 128 total layers, and a name under 256 bytes".into());
        }
        if !self.background.angle.is_finite()
            || !self.background.spacing.is_finite()
            || !(4.0..=1000.0).contains(&self.background.spacing)
        {
            return Err("Background pattern spacing must be 4–1000 px".into());
        }
        let mut pixels = 0;
        let mut asset_bytes = 0usize;
        for l in &self.layers {
            l.validate()?;
            if let LayerKind::Image { asset, .. } = &l.kind {
                pixels += u64::from(asset.pixels.width()) * u64::from(asset.pixels.height());
                asset_bytes = asset_bytes.saturating_add(asset.bytes.len());
            }
        }
        if pixels > 32_000_000 {
            return Err("Combined image layers exceed the 32 megapixel artwork budget".into());
        }
        if asset_bytes > amf::MAX_FILE {
            return Err("Combined source images exceed 32 MiB. Use smaller image files.".into());
        }
        Ok(())
    }
    pub fn photo_count(&self) -> usize {
        self.layers.iter().filter(|l| l.is_photo()).count()
    }
    /// Visible photo regions in the same back-to-front order used by rendering.
    /// Repeated shot indices intentionally refer to the same captured photo.
    pub fn photo_regions(&self) -> impl DoubleEndedIterator<Item = (usize, Frame)> + '_ {
        self.layers
            .iter()
            .filter(|layer| layer.visible)
            .filter_map(|layer| match layer.kind {
                LayerKind::Photo { shot, .. } => Some((shot as usize, layer.frame)),
                _ => None,
            })
    }

    pub fn shots_needed(&self) -> u32 {
        self.layers
            .iter()
            .filter(|l| l.visible)
            .filter_map(|l| {
                if let LayerKind::Photo { shot, .. } = l.kind {
                    Some(shot + 1)
                } else {
                    None
                }
            })
            .max()
            .unwrap_or(1)
    }
    pub fn builtin(duplicate: bool) -> Self {
        let mut layers = Vec::new();
        for column in 0..if duplicate { 2 } else { 1 } {
            for shot in 0..4 {
                layers.push(Layer::photo(
                    Frame::new(
                        30.0 + column as f64 * 600.0,
                        30.0 + shot as f64 * 435.0,
                        540.0,
                        405.0,
                    ),
                    shot,
                ));
            }
        }
        let mut raw = Object::new();
        n(&mut raw, "version", 2.0);
        n(&mut raw, "type", 0.0);
        s(&mut raw, "paper", if duplicate { "4 x 6" } else { "2 x 6" });
        Self {
            name: if duplicate {
                "Double strip · 4 × 6"
            } else {
                "Classic strip · 2 × 6"
            }
            .into(),
            width: if duplicate { 1200 } else { 600 },
            height: 1800,
            background: Background::default(),
            layers,
            raw,
        }
    }
    pub fn grid() -> Self {
        let mut t = Self::builtin(true);
        t.name = "Photo grid · 4 × 6".into();
        t.width = 1800;
        t.height = 1200;
        t.layers = (0..4)
            .map(|i| {
                Layer::photo(
                    Frame::new(
                        90.0 + (i % 2) as f64 * 900.0,
                        30.0 + (i / 2) as f64 * 600.0,
                        720.0,
                        540.0,
                    ),
                    i,
                )
            })
            .collect();
        t
    }
    pub fn render(
        &self,
        shots: &[DynamicImage],
        max_edge: Option<u32>,
    ) -> Result<DynamicImage, String> {
        self.validate()?;
        if !shots.is_empty() && shots.len() < self.shots_needed() as usize {
            return Err("Not enough captured photos for this design".into());
        }
        let scale = max_edge
            .map(|m| (m as f64 / self.width.max(self.height) as f64).min(1.0))
            .unwrap_or(1.0);
        let w = (self.width as f64 * scale).round().max(1.0) as u32;
        let h = (self.height as f64 * scale).round().max(1.0) as u32;
        let mut canvas = self.background.render(w, h, scale);
        for layer in &self.layers {
            layer.paint((w, h), scale, shots, |x, y, p| {
                canvas.get_pixel_mut(x, y).blend(&p)
            });
        }
        Ok(DynamicImage::ImageRgba8(canvas))
    }
    fn design_value(&self) -> Value {
        let mut root = Object::new();
        n(&mut root, "version", 1.0);
        s(&mut root, "name", &self.name);
        n(&mut root, "width", self.width);
        n(&mut root, "height", self.height);
        let mut bg = Object::new();
        put_color(&mut bg, "color", self.background.color);
        put_color(&mut bg, "secondary", self.background.secondary);
        s(&mut bg, "style", &format!("{:?}", self.background.style));
        n(&mut bg, "angle", self.background.angle);
        n(&mut bg, "spacing", self.background.spacing);
        root.insert("background".into(), Value::Object(Arc::new(bg)));
        let layers = self
            .layers
            .iter()
            .map(|l| {
                let mut o = Object::new();
                s(&mut o, "name", &l.name);
                put_frame(&mut o, l.frame);
                b(&mut o, "visible", l.visible);
                b(&mut o, "locked", l.locked);
                n(&mut o, "opacity", l.opacity);
                n(&mut o, "rounding", l.rounding);
                n(&mut o, "border", l.border);
                put_color(&mut o, "border_color", l.border_color);
                match &l.kind {
                    LayerKind::Photo { shot, source } => {
                        s(&mut o, "kind", "photo");
                        n(&mut o, "shot", *shot);
                        o.insert("source".into(), Value::Object(Arc::new(source.clone())));
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
                        s(&mut o, "kind", "text");
                        s(&mut o, "text", text);
                        s(&mut o, "family", &format!("{family:?}"));
                        s(&mut o, "align", &format!("{align:?}"));
                        n(&mut o, "size", *size);
                        put_color(&mut o, "color", *color);
                        b(&mut o, "bold", *bold);
                        b(&mut o, "italic", *italic);
                        n(&mut o, "spacing", *spacing);
                        n(&mut o, "line_height", *line_height);
                    }
                    LayerKind::Image {
                        asset,
                        fit,
                        flip_x,
                        flip_y,
                    } => {
                        s(&mut o, "kind", "image");
                        o.insert("image".into(), Value::Bytes(asset.bytes.clone()));
                        s(&mut o, "fit", &format!("{fit:?}"));
                        b(&mut o, "flip_x", *flip_x);
                        b(&mut o, "flip_y", *flip_y);
                    }
                    LayerKind::Shape { shape, color } => {
                        s(&mut o, "kind", "shape");
                        s(&mut o, "shape", &format!("{shape:?}"));
                        put_color(&mut o, "color", *color);
                    }
                }
                Value::Object(Arc::new(o))
            })
            .collect();
        root.insert("layers".into(), Value::Array(Arc::new(layers)));
        Value::Object(Arc::new(root))
    }
    /// At each pixel the last opaque photo owns occlusion: reset the overlay
    /// wherever a photo paints, then keep only later artwork. Outside all photo
    /// coverage, artwork lives only in the background. Arbitrary interleaving
    /// therefore exports without relying on Sparkbooth's text-layer encoding.
    fn fallback_art(&self) -> Result<(Arc<[u8]>, Arc<[u8]>), String> {
        let size = (self.width, self.height);
        let mut bg = self.background.render(size.0, size.1, 1.0);
        let mut overlay = RgbaImage::new(size.0, size.1);
        let mut coverage = vec![false; (u64::from(size.0) * u64::from(size.1)) as usize];
        for layer in &self.layers {
            if layer.is_photo() {
                layer.paint(size, 1.0, &[], |x, y, _| {
                    overlay.put_pixel(x, y, Rgba([0; 4]));
                    coverage[(y * size.0 + x) as usize] = true;
                });
            } else {
                layer.paint(size, 1.0, &[], |x, y, p| {
                    bg.get_pixel_mut(x, y).blend(&p);
                    overlay.get_pixel_mut(x, y).blend(&p);
                });
            }
        }
        for (i, p) in overlay.pixels_mut().enumerate() {
            if !coverage[i] {
                *p = Rgba([0; 4]);
            }
        }
        Ok((png(bg)?, png(overlay)?))
    }
    pub fn to_bytes(&self) -> Result<Vec<u8>, String> {
        self.validate()?;
        let mut raw = self.raw.clone();
        n(&mut raw, "version", 2.0);
        s(&mut raw, "name", &self.name);
        n(&mut raw, "width", self.width);
        n(&mut raw, "height", self.height);
        n(
            &mut raw,
            "proportion",
            self.width as f64 / self.height as f64,
        );
        s(
            &mut raw,
            "orientation",
            if self.width > self.height {
                "Landscape"
            } else {
                "Portrait"
            },
        );
        for key in ["texts", "images"] {
            raw.insert(key.into(), Value::Array(Arc::new(vec![])));
        }
        let photos = self
            .layers
            .iter()
            .filter(|l| l.visible && l.is_photo())
            .enumerate()
            .map(|(i, l)| {
                let LayerKind::Photo { shot, source } = &l.kind else {
                    unreachable!()
                };
                let mut p = source.clone();
                put_frame(&mut p, l.frame);
                n(&mut p, "photoindex", *shot);
                n(&mut p, "orderindex", i as f64);
                Value::Object(Arc::new(p))
            })
            .collect();
        raw.insert("photos".into(), Value::Array(Arc::new(photos)));
        let (background, overlay) = self.fallback_art()?;
        raw.insert("backgroundimage".into(), Value::Bytes(background));
        s(&mut raw, "background", "radiate-background.png");
        raw.insert("overlayimage".into(), Value::Bytes(overlay));
        s(&mut raw, "overlay", "radiate-overlay.png");
        raw.insert("radiate_design".into(), self.design_value());
        let bytes = amf::encode(&Value::Object(Arc::new(raw)))?;
        if bytes.len() > amf::MAX_FILE {
            return Err("Design exceeds 32 MiB. Use smaller source images before saving.".into());
        }
        Ok(bytes)
    }
    pub fn save(&self, path: &Path) -> Result<(), String> {
        let bytes = self.to_bytes()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        if path.exists() {
            let backup = path.with_file_name(format!(
                "{}.backup-{}",
                path.file_name().unwrap_or_default().to_string_lossy(),
                chrono::Local::now().format("%Y%m%d-%H%M%S-%f")
            ));
            std::fs::copy(path, backup).map_err(|e| format!("Could not back up layout: {e}"))?;
        }
        let temp = path.with_extension("sbl.tmp");
        std::fs::write(&temp, bytes).map_err(|e| e.to_string())?;
        std::fs::rename(temp, path).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn shots(n: u8) -> Vec<DynamicImage> {
        (0..n)
            .map(|i| {
                DynamicImage::ImageRgba8(RgbaImage::from_pixel(
                    80,
                    60,
                    Rgba([i * 40, 120, 200, 255]),
                ))
            })
            .collect()
    }
    #[test]
    fn duplicate_slots_reuse_captures_and_round_trip() {
        let t = Template::builtin(true);
        assert_eq!(t.shots_needed(), 4);
        let loaded = Template::from_bytes(&t.to_bytes().unwrap()).unwrap();
        assert_eq!(loaded.photo_count(), 8);
        let image = loaded.render(&shots(4), Some(600)).unwrap().to_rgba8();
        assert_eq!(image.get_pixel(50, 80), image.get_pixel(250, 80));
        assert!(loaded.render(&shots(2), None).is_err());
    }
    #[test]
    fn malformed_geometry_and_unknown_design_versions_fail() {
        let mut t = Template::builtin(false);
        t.layers[0].frame.width = f64::NAN;
        assert!(t.validate().is_err());
        let mut raw = object(&amf::decode(&Template::builtin(false).to_bytes().unwrap()).unwrap())
            .unwrap()
            .clone();
        raw.insert(
            "radiate_design".into(),
            Value::Object(Arc::new(Object::from([(
                "version".into(),
                Value::Number(20.0),
            )]))),
        );
        assert!(
            Template::from_bytes(&amf::encode(&Value::Object(Arc::new(raw))).unwrap())
                .err()
                .unwrap()
                .contains("newer")
        );
    }
    #[test]
    fn interleaved_art_flattens_and_editable_round_trip_matches_output() {
        let mut t = Template::builtin(false);
        t.width = 100;
        t.height = 100;
        t.layers = vec![
            Layer::photo(Frame::new(5.0, 5.0, 65.0, 65.0), 0),
            Layer::shape(
                ShapeKind::Rectangle,
                Frame::new(0.0, 0.0, 90.0, 90.0),
                [240, 10, 30, 120],
            ),
            Layer::photo(
                Frame {
                    rotation: 35.0,
                    ..Frame::new(70.0, 20.0, 40.0, 40.0)
                },
                1,
            ),
            Layer::text("Hi", Frame::new(15.0, 20.0, 75.0, 70.0)),
        ];
        let original = t.render(&shots(2), None).unwrap();
        let bytes = t.to_bytes().unwrap();
        let loaded = Template::from_bytes(&bytes).unwrap();
        assert!(matches!(loaded.layers[3].kind, LayerKind::Text { .. }));
        assert!(
            original.as_bytes() == loaded.render(&shots(2), None).unwrap().as_bytes(),
            "editable design changed after reload"
        );
        let mut raw = object(&amf::decode(&bytes).unwrap()).unwrap().clone();
        raw.remove("radiate_design");
        let fallback =
            Template::from_bytes(&amf::encode(&Value::Object(Arc::new(raw))).unwrap()).unwrap();
        let rendered = fallback.render(&shots(2), None).unwrap();
        let max_error = original
            .as_bytes()
            .iter()
            .zip(rendered.as_bytes())
            .map(|(a, b)| a.abs_diff(*b))
            .max()
            .unwrap();
        assert!(
            max_error <= 2,
            "flattened alpha compositing differs by {max_error} levels"
        );
    }
    #[test]
    fn rotation_and_overlay_follow_stack_order() {
        let mut t = Template::builtin(false);
        t.width = 10;
        t.height = 10;
        t.layers = vec![Layer::photo(
            Frame {
                rotation: 90.0,
                ..Frame::new(8.0, 1.0, 4.0, 3.0)
            },
            0,
        )];
        let shot = DynamicImage::ImageRgba8(RgbaImage::from_pixel(4, 3, Rgba([255, 0, 0, 255])));
        let image = t.render(&[shot.clone()], None).unwrap().to_rgba8();
        assert_eq!(image.get_pixel(6, 2).0, [255, 0, 0, 255]);
        assert_eq!(image.get_pixel(9, 2).0, [255; 4]);
        t.layers.push(Layer::shape(
            ShapeKind::Rectangle,
            Frame::new(0.0, 0.0, 10.0, 10.0),
            [0, 0, 255, 255],
        ));
        assert_eq!(
            t.render(&[shot], None)
                .unwrap()
                .to_rgba8()
                .get_pixel(6, 2)
                .0,
            [0, 0, 255, 255]
        );
    }
    #[test]
    fn unsupported_external_text_layers_remain_actionable() {
        let mut raw = object(&amf::decode(&Template::builtin(false).to_bytes().unwrap()).unwrap())
            .unwrap()
            .clone();
        raw.remove("radiate_design");
        raw.insert("texts".into(), Value::Array(Arc::new(vec![Value::Null])));
        assert!(
            Template::from_bytes(&amf::encode(&Value::Object(Arc::new(raw))).unwrap())
                .err()
                .unwrap()
                .contains("texts")
        );
    }
    #[test]
    fn image_styles_and_text_survive_file_save_and_overwrite_backup() {
        let mut t = Template::builtin(false);
        t.width = 120;
        t.height = 180;
        t.layers = vec![Layer::photo(Frame::new(10.0, 10.0, 100.0, 80.0), 0)];
        let bytes = png(RgbaImage::from_fn(8, 4, |x, y| {
            Rgba([x as u8 * 30, y as u8 * 60, 120, 210])
        }))
        .unwrap();
        let mut image = Layer::new(
            "Logo",
            Frame::new(15.0, 95.0, 90.0, 40.0),
            LayerKind::Image {
                asset: decode_art(bytes).unwrap(),
                fit: ImageFit::Contain,
                flip_x: true,
                flip_y: true,
            },
        );
        image.rounding = 7.0;
        image.border = 2.0;
        image.opacity = 0.75;
        image.locked = true;
        t.layers.push(image);
        let mut text = Layer::text(
            "Two lines\nof memories",
            Frame::new(10.0, 140.0, 100.0, 40.0),
        );
        if let LayerKind::Text {
            size,
            family,
            align,
            bold,
            italic,
            spacing,
            line_height,
            ..
        } = &mut text.kind
        {
            *size = 13.0;
            *family = FontFamily::Mono;
            *align = TextAlign::Right;
            *bold = true;
            *italic = true;
            *spacing = 0.4;
            *line_height = 1.3;
        }
        t.layers.push(text);
        t.background.style = BackgroundStyle::Gradient;
        let dir = std::env::temp_dir().join(format!(
            "radiate-design-save-{}-{}",
            std::process::id(),
            chrono::Local::now().timestamp_nanos_opt().unwrap()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("design.sbl");
        t.save(&path).unwrap();
        let first = std::fs::read(&path).unwrap();
        let restored = Template::load(&path).unwrap();
        assert!(
            t.render(&shots(1), None).unwrap().as_bytes()
                == restored.render(&shots(1), None).unwrap().as_bytes(),
            "image/text style changed on disk round trip"
        );
        assert!(restored.layers[1].locked);
        assert!(matches!(
            restored.layers[1].kind,
            LayerKind::Image {
                flip_x: true,
                flip_y: true,
                ..
            }
        ));
        t.name = "Changed".into();
        t.save(&path).unwrap();
        let backup = std::fs::read_dir(&dir)
            .unwrap()
            .map(|p| p.unwrap().path())
            .find(|p| {
                p.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .contains(".backup-")
            })
            .unwrap();
        assert_eq!(std::fs::read(backup).unwrap(), first);
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    #[ignore = "requires the two public SBL samples; see README compatibility checks"]
    fn public_sbl_samples_import_render_and_round_trip() {
        let dir = std::path::PathBuf::from(
            std::env::var_os("SPARKBOOTH_SBL_SAMPLE_DIR").expect("sample directory"),
        );
        for (file, size, placements, captures) in [
            ("softhearts.sbl", (1800, 1200), 4, 4),
            ("duplicate.sbl", (1200, 1800), 6, 3),
        ] {
            let t = Template::load(&dir.join(file)).unwrap();
            assert_eq!((t.width, t.height), size);
            assert_eq!(t.photo_count(), placements);
            assert_eq!(t.shots_needed(), captures);
            let image = t.render(&shots(captures as u8), None).unwrap();
            let loaded = Template::from_bytes(&t.to_bytes().unwrap()).unwrap();
            assert_eq!(
                loaded
                    .render(&shots(captures as u8), None)
                    .unwrap()
                    .as_bytes(),
                image.as_bytes()
            );
            image.save(dir.join(format!("{file}-render.png"))).unwrap();
        }
    }
}
