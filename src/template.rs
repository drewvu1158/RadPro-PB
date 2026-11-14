//! Owns SBL interpretation, validation, embedded artwork, and composition.
use crate::amf::{self, Object, Value};
use image::{DynamicImage, ImageReader, Pixel, Rgba, RgbaImage};
use std::{
    io::{Cursor, Read},
    path::Path,
    sync::Arc,
};

// 16 million RGBA pixels = 64 MiB per decoded artwork or rendered canvas.
const MAX_PIXELS: u64 = 16_000_000;

#[derive(Clone, Debug)]
pub struct PhotoSlot {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub rotation: f64,
    pub photo: u32,
    raw: Object,
}

#[derive(Clone, Copy, Debug)]
pub struct SlotBounds {
    pub left: f64,
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
}

impl SlotBounds {
    pub fn x_anchors(self) -> [f64; 3] {
        [self.left, (self.left + self.right) * 0.5, self.right]
    }
    pub fn y_anchors(self) -> [f64; 3] {
        [self.top, (self.top + self.bottom) * 0.5, self.bottom]
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

impl PhotoSlot {
    pub fn new(x: f64, y: f64, width: f64, height: f64, photo: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
            rotation: 0.0,
            photo,
            raw: Object::new(),
        }
    }

    fn corners_for_size(&self, width: f64, height: f64) -> [(f64, f64); 4] {
        let (s, c) = self.rotation.to_radians().sin_cos();
        [(0.0, 0.0), (width, 0.0), (width, height), (0.0, height)]
            .map(|(x, y)| (self.x + c * x - s * y, self.y + s * x + c * y))
    }

    pub fn corners(&self) -> [(f64, f64); 4] {
        self.corners_for_size(self.width, self.height)
    }

    /// Visual bounds, including rotation around the SBL top-left origin.
    pub fn bounds(&self) -> SlotBounds {
        let corners = self.corners();
        SlotBounds {
            left: corners.iter().map(|p| p.0).fold(f64::INFINITY, f64::min),
            right: corners
                .iter()
                .map(|p| p.0)
                .fold(f64::NEG_INFINITY, f64::max),
            top: corners.iter().map(|p| p.1).fold(f64::INFINITY, f64::min),
            bottom: corners
                .iter()
                .map(|p| p.1)
                .fold(f64::NEG_INFINITY, f64::max),
        }
    }

    pub fn contains(&self, x: f64, y: f64) -> bool {
        let (s, c) = self.rotation.to_radians().sin_cos();
        let dx = x - self.x;
        let dy = y - self.y;
        let local_x = c * dx + s * dy;
        let local_y = -s * dx + c * dy;
        local_x >= 0.0 && local_y >= 0.0 && local_x < self.width && local_y < self.height
    }
}

#[derive(Clone)]
pub struct Template {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub slots: Vec<PhotoSlot>,
    pub background: Option<Arc<RgbaImage>>,
    pub overlay: Option<Arc<RgbaImage>>,
    raw: Object,
}

fn number(o: &Object, k: &str) -> Result<f64, String> {
    match o.get(k) {
        Some(Value::Number(n)) if n.is_finite() => Ok(*n),
        _ => Err(format!("Missing or invalid SBL number: {k}")),
    }
}
fn integer(o: &Object, k: &str) -> Result<u32, String> {
    let n = number(o, k)?;
    if n < 0.0 || n > u32::MAX as f64 || n.fract() != 0.0 {
        return Err(format!("Invalid SBL integer: {k}"));
    }
    Ok(n as u32)
}
fn array<'a>(o: &'a Object, k: &str) -> Result<&'a [Value], String> {
    match o.get(k) {
        Some(Value::Array(a)) => Ok(a.as_slice()),
        None | Some(Value::Null) => Ok(&[]),
        _ => Err(format!("Invalid SBL array: {k}")),
    }
}
fn object(v: &Value) -> Result<&Object, String> {
    match v {
        Value::Object(o) => Ok(o),
        _ => Err("Expected an SBL object".into()),
    }
}

fn decode_art(bytes: &[u8]) -> Result<Arc<RgbaImage>, String> {
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(MAX_PIXELS * 4);
    reader.limits(limits);
    // Check pixel expansion before any decoder allocates a complete image.
    let (w, h) = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| e.to_string())?
        .into_dimensions()
        .map_err(|e| format!("Artwork header: {e}"))?;
    if u64::from(w) * u64::from(h) > MAX_PIXELS {
        return Err("Artwork exceeds the 16 megapixel memory budget".into());
    }
    let image = reader
        .decode()
        .map_err(|e| format!("Could not decode template artwork: {e}"))?;
    Ok(Arc::new(image.to_rgba8()))
}

fn read_bounded(path: &Path) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take(amf::MAX_FILE as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > amf::MAX_FILE {
        return Err("File exceeds the 32 MiB import memory budget".into());
    }
    Ok(bytes)
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
        for key in ["texts", "images"] {
            if !array(&raw, key)?.is_empty() {
                return Err(format!("This SBL has separate {key} layers, which are not supported yet. Flatten these into a PNG overlay in Sparkbooth and export again. Your current layout was kept."));
            }
        }
        let name = match raw.get("name") {
            Some(Value::String(s)) => s.to_string(),
            _ => "Imported layout".into(),
        };
        let mut ordered_slots = Vec::new();
        for (position, val) in array(&raw, "photos")?.iter().enumerate() {
            let p = object(val)?;
            let order = if p.contains_key("orderindex") {
                integer(p, "orderindex")?
            } else {
                position as u32
            };
            ordered_slots.push((
                order,
                PhotoSlot {
                    x: number(p, "x")?,
                    y: number(p, "y")?,
                    width: number(p, "width")?,
                    height: number(p, "height")?,
                    rotation: number(p, "rotation")?,
                    photo: integer(p, "photoindex")?,
                    raw: p.clone(),
                },
            ));
        }
        // SBL orderindex owns stacking, not the incidental AMF array order.
        ordered_slots.sort_by_key(|(order, _)| *order);
        let slots = ordered_slots.into_iter().map(|(_, slot)| slot).collect();
        let asset = |embedded: &str, path: &str| -> Result<Option<Arc<RgbaImage>>, String> {
            match raw.get(embedded) {
                Some(Value::Bytes(b)) if !b.is_empty() => decode_art(b).map(Some),
                None | Some(Value::Null) => match raw.get(path) {
                    Some(Value::String(s)) if !s.is_empty() => Err(format!("The SBL references {path} artwork but does not embed it. Re-export with the artwork included.")),
                    _ => Ok(None),
                },
                _ => Err(format!("Invalid embedded SBL artwork: {embedded}")),
            }
        };
        let template = Self {
            name,
            width: integer(&raw, "width")?,
            height: integer(&raw, "height")?,
            slots,
            background: asset("backgroundimage", "background")?,
            overlay: asset("overlayimage", "overlay")?,
            raw,
        };
        template.validate()?;
        Ok(template)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.width == 0
            || self.height == 0
            || self.width > 8192
            || self.height > 8192
            || u64::from(self.width) * u64::from(self.height) > MAX_PIXELS
        {
            return Err("Canvas must fit within 8192 pixels per side and 16 megapixels".into());
        }
        if self.slots.is_empty() || self.slots.len() > 64 {
            return Err("A layout needs 1–64 photo placements".into());
        }
        for p in &self.slots {
            if ![p.x, p.y, p.width, p.height, p.rotation]
                .iter()
                .all(|n| n.is_finite())
                || p.width < 1.0
                || p.height < 1.0
                || p.width > 8192.0
                || p.height > 8192.0
                || p.width * p.height > MAX_PIXELS as f64
                || p.photo >= 8
                || p.x.abs() > 16384.0
                || p.y.abs() > 16384.0
            {
                return Err("Photo placements must be finite, at least 1 pixel, within the image memory budget, and use shots 1–8".into());
            }
        }
        Ok(())
    }

    pub fn shots_needed(&self) -> u32 {
        self.slots.iter().map(|p| p.photo + 1).max().unwrap_or(1)
    }

    pub fn builtin(duplicate: bool) -> Self {
        let mut slots = Vec::new();
        for column in 0..if duplicate { 2 } else { 1 } {
            for photo in 0..4 {
                slots.push(PhotoSlot::new(
                    30.0 + column as f64 * 600.0,
                    30.0 + photo as f64 * 435.0,
                    540.0,
                    405.0,
                    photo,
                ));
            }
        }
        let mut raw = Object::new();
        raw.insert("version".into(), Value::Number(2.0));
        raw.insert("type".into(), Value::Number(0.0));
        raw.insert(
            "paper".into(),
            Value::String(Arc::from(if duplicate { "4 x 6" } else { "2 x 6" })),
        );
        for key in ["texts", "images"] {
            raw.insert(key.into(), Value::Array(Arc::new(vec![])));
        }
        Self {
            name: if duplicate {
                "Double strip · 4 × 6"
            } else {
                "Classic strip · 2 × 6"
            }
            .into(),
            width: if duplicate { 1200 } else { 600 },
            height: 1800,
            slots,
            background: None,
            overlay: None,
            raw,
        }
    }

    pub fn grid() -> Self {
        let mut t = Self::builtin(true);
        t.name = "Photo grid · 4 × 6".into();
        t.width = 1800;
        t.height = 1200;
        t.slots = (0..4)
            .map(|i| {
                PhotoSlot::new(
                    90.0 + (i % 2) as f64 * 900.0,
                    30.0 + (i / 2) as f64 * 600.0,
                    720.0,
                    540.0,
                    i,
                )
            })
            .collect();
        t
    }

    pub fn set_art(&mut self, overlay: bool, path: Option<&Path>) -> Result<(), String> {
        let (image, bytes) = match path {
            Some(path) => {
                let bytes = read_bounded(path)?;
                (Some(decode_art(&bytes)?), Value::Bytes(Arc::from(bytes)))
            }
            None => (None, Value::Null),
        };
        self.raw.insert(
            if overlay {
                "overlayimage"
            } else {
                "backgroundimage"
            }
            .into(),
            bytes,
        );
        self.raw.insert(
            if overlay { "overlay" } else { "background" }.into(),
            if image.is_some() {
                Value::String(Arc::from(if overlay {
                    "overlay.png"
                } else {
                    "background.png"
                }))
            } else {
                Value::Null
            },
        );
        if overlay {
            self.overlay = image;
        } else {
            self.background = image;
        }
        Ok(())
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>, String> {
        self.validate()?;
        let mut raw = self.raw.clone();
        raw.insert("name".into(), Value::String(Arc::from(self.name.as_str())));
        raw.insert("width".into(), Value::Number(self.width as f64));
        raw.insert("height".into(), Value::Number(self.height as f64));
        raw.insert(
            "proportion".into(),
            Value::Number(self.width as f64 / self.height as f64),
        );
        raw.insert(
            "orientation".into(),
            Value::String(Arc::from(if self.width > self.height {
                "Landscape"
            } else {
                "Portrait"
            })),
        );
        raw.insert(
            "photos".into(),
            Value::Array(Arc::new(
                self.slots
                    .iter()
                    .enumerate()
                    .map(|(i, p)| {
                        let mut o = p.raw.clone();
                        for (key, n) in [
                            ("x", p.x),
                            ("y", p.y),
                            ("width", p.width),
                            ("height", p.height),
                            ("rotation", p.rotation),
                            ("photoindex", p.photo as f64),
                            ("orderindex", i as f64),
                        ] {
                            o.insert(key.into(), Value::Number(n));
                        }
                        Value::Object(Arc::new(o))
                    })
                    .collect(),
            )),
        );
        amf::encode(&Value::Object(Arc::new(raw)))
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

    /// Same renderer for editor previews, saved layouts, and the print image.
    /// Rotation follows Flash's top-left origin; photoindex repeats a shot.
    pub fn render(
        &self,
        shots: &[DynamicImage],
        max_edge: Option<u32>,
    ) -> Result<DynamicImage, String> {
        self.validate()?;
        if !shots.is_empty() && shots.len() < self.shots_needed() as usize {
            return Err("Not enough captured photos for the selected template".into());
        }
        let scale = max_edge
            .map(|m| (m as f64 / self.width.max(self.height) as f64).min(1.0))
            .unwrap_or(1.0);
        let w = (self.width as f64 * scale).round().max(1.0) as u32;
        let h = (self.height as f64 * scale).round().max(1.0) as u32;
        let mut canvas = RgbaImage::from_pixel(w, h, Rgba([255, 255, 255, 255]));
        let art = |canvas: &mut RgbaImage, image: &Option<Arc<RgbaImage>>| {
            if let Some(image) = image {
                let resized = image::imageops::resize(
                    image.as_ref(),
                    w,
                    h,
                    image::imageops::FilterType::Triangle,
                );
                image::imageops::overlay(canvas, &resized, 0, 0);
            }
        };
        art(&mut canvas, &self.background);
        for slot in &self.slots {
            let sw = (slot.width * scale).round().max(1.0) as u32;
            let sh = (slot.height * scale).round().max(1.0) as u32;
            let photo = shots
                .get(slot.photo as usize)
                .map(|p| crate::layout::cover_crop_resize(p, (sw, sh)));
            let (s, c) = slot.rotation.to_radians().sin_cos();
            let x = slot.x * scale;
            let y = slot.y * scale;
            let corners = slot
                .corners_for_size(sw as f64 / scale, sh as f64 / scale)
                .map(|(x, y)| (x * scale, y * scale));
            let minx = corners
                .iter()
                .map(|p| p.0)
                .fold(f64::INFINITY, f64::min)
                .floor()
                .max(0.0) as u32;
            let maxx = corners
                .iter()
                .map(|p| p.0)
                .fold(f64::NEG_INFINITY, f64::max)
                .ceil()
                .clamp(0.0, w as f64) as u32;
            let miny = corners
                .iter()
                .map(|p| p.1)
                .fold(f64::INFINITY, f64::min)
                .floor()
                .max(0.0) as u32;
            let maxy = corners
                .iter()
                .map(|p| p.1)
                .fold(f64::NEG_INFINITY, f64::max)
                .ceil()
                .clamp(0.0, h as f64) as u32;
            for cy in miny..maxy {
                for cx in minx..maxx {
                    let dx = cx as f64 + 0.5 - x;
                    let dy = cy as f64 + 0.5 - y;
                    let px = c * dx + s * dy;
                    let py = -s * dx + c * dy;
                    if px >= 0.0 && py >= 0.0 && px < sw as f64 && py < sh as f64 {
                        let p = match &photo {
                            Some(p) => *p.get_pixel(px as u32, py as u32),
                            None => {
                                const COLORS: [[u8; 4]; 4] = [
                                    [68, 94, 128, 255],
                                    [79, 120, 116, 255],
                                    [134, 105, 76, 255],
                                    [120, 84, 112, 255],
                                ];
                                Rgba(COLORS[slot.photo as usize % 4])
                            }
                        };
                        canvas.get_pixel_mut(cx, cy).blend(&p);
                    }
                }
            }
        }
        art(&mut canvas, &self.overlay);
        Ok(DynamicImage::ImageRgba8(canvas))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn duplicate_slots_reuse_captures_and_round_trip() {
        let t = Template::builtin(true);
        assert_eq!(t.shots_needed(), 4);
        let loaded = Template::from_bytes(&t.to_bytes().unwrap()).unwrap();
        assert_eq!(loaded.slots.len(), 8);
        let shots: Vec<_> = (0..4)
            .map(|i| {
                DynamicImage::ImageRgba8(RgbaImage::from_pixel(4, 4, Rgba([i * 50, 0, 0, 255])))
            })
            .collect();
        let image = loaded.render(&shots, Some(600)).unwrap().to_rgba8();
        assert_eq!(image.get_pixel(50, 80), image.get_pixel(250, 80));
        assert_eq!(image.get_pixel(50, 225), image.get_pixel(250, 225));
        assert!(loaded.render(&shots[..2], None).is_err());
    }
    #[test]
    fn rejects_bad_geometry_and_unsupported_layers() {
        let mut t = Template::builtin(false);
        t.slots[0].width = f64::NAN;
        assert!(t.validate().is_err());
        t.slots[0].width = 100.0;
        t.raw
            .insert("texts".into(), Value::Array(Arc::new(vec![Value::Null])));
        assert!(Template::from_bytes(&t.to_bytes().unwrap())
            .err()
            .expect("unsupported text layers")
            .contains("texts"));
    }

    #[test]
    fn rotation_and_overlay_are_composed_in_source_order() {
        let mut t = Template::builtin(false);
        t.width = 10;
        t.height = 10;
        let mut slot = PhotoSlot::new(8.0, 1.0, 4.0, 3.0, 0);
        slot.rotation = 90.0;
        t.slots = vec![slot];
        let shot = DynamicImage::ImageRgba8(RgbaImage::from_pixel(4, 3, Rgba([255, 0, 0, 255])));
        let image = t.render(&[shot.clone()], None).unwrap().to_rgba8();
        assert_eq!(image.get_pixel(6, 2).0, [255, 0, 0, 255]);
        assert_eq!(image.get_pixel(9, 2).0, [255, 255, 255, 255]);
        t.overlay = Some(Arc::new(RgbaImage::from_pixel(
            10,
            10,
            Rgba([0, 0, 255, 255]),
        )));
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
    #[ignore = "requires the two public SBL samples; see README compatibility checks"]
    fn public_sbl_samples_import_render_and_round_trip() {
        let dir = std::path::PathBuf::from(
            std::env::var_os("SPARKBOOTH_SBL_SAMPLE_DIR").expect("sample directory"),
        );
        for (file, size, placements, captures, overlay) in [
            ("softhearts.sbl", (1800, 1200), 4, 4, false),
            ("duplicate.sbl", (1200, 1800), 6, 3, true),
        ] {
            let template = Template::load(&dir.join(file)).unwrap();
            assert_eq!((template.width, template.height), size);
            assert_eq!(template.slots.len(), placements);
            assert_eq!(template.shots_needed(), captures);
            assert!(template.background.is_some());
            assert_eq!(template.overlay.is_some(), overlay);
            let shots: Vec<_> = (0..captures)
                .map(|i| {
                    DynamicImage::ImageRgba8(RgbaImage::from_pixel(
                        80,
                        60,
                        Rgba([50 * i as u8, 120, 200, 255]),
                    ))
                })
                .collect();
            let image = template.render(&shots, None).unwrap();
            let loaded = Template::from_bytes(&template.to_bytes().unwrap()).unwrap();
            assert_eq!(
                loaded.render(&shots, None).unwrap().as_bytes(),
                image.as_bytes()
            );
            image.save(dir.join(format!("{file}-render.png"))).unwrap();
            println!("{file}: {}x{}, {} placements / {} captures, embedded art and rendered round trip verified",size.0,size.1,placements,captures);
        }
    }
}
