//! Composing captured shots into a shareable layout (vertical strip or grid).

use image::{DynamicImage, GenericImageView, ImageBuffer, Rgba, RgbaImage};

/// Which multi-photo layout to compose.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LayoutKind {
    /// Classic photo booth strip: shots stacked vertically.
    Strip,
    /// Two-by-two grid (uses the first 4 shots).
    Grid2x2,
}

/// Everything needed to compose a layout. Cell images are center-cropped to
/// the cell aspect ratio and resized with a fast triangle filter.
#[derive(Clone)]
pub struct LayoutOptions {
    /// Inner cell size (photos) in pixels.
    pub cell: (u32, u32),
    /// Border margin around the whole layout, in pixels.
    pub margin: u32,
    /// Gap between cells, in pixels.
    pub gap: u32,
    /// Background color of margins and gaps.
    pub background: [u8; 4],
}

impl Default for LayoutOptions {
    fn default() -> Self {
        Self {
            cell: (640, 480),
            margin: 40,
            gap: 16,
            background: [255, 255, 255, 255],
        }
    }
}

#[derive(Debug)]
pub enum LayoutError {
    /// No shots were supplied.
    Empty,
    /// A shot failed to process.
    Image(image::ImageError),
}

impl std::fmt::Display for LayoutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LayoutError::Empty => write!(f, "no shots were supplied"),
            LayoutError::Image(e) => write!(f, "failed to process shot: {e}"),
        }
    }
}

impl std::error::Error for LayoutError {}

impl From<image::ImageError> for LayoutError {
    fn from(e: image::ImageError) -> Self {
        LayoutError::Image(e)
    }
}

/// Center-crop `img` to exactly `target_aspect` (width / height) and resize.
pub(crate) fn cover_crop_resize(img: &DynamicImage, target: (u32, u32)) -> RgbaImage {
    let (w, h) = img.dimensions();
    let target_aspect = f64::from(target.0) / f64::from(target.1);
    let aspect = f64::from(w) / f64::from(h);
    let (crop_w, crop_h) = if aspect > target_aspect {
        // Too wide: trim the sides.
        ((f64::from(h) * target_aspect) as u32, h)
    } else {
        // Too tall: trim the top/bottom.
        (w, (f64::from(w) / target_aspect) as u32)
    };
    let x = (w.saturating_sub(crop_w)) / 2;
    let y = (h.saturating_sub(crop_h)) / 2;
    let cropped = img.crop_imm(x, y, crop_w.max(1), crop_h.max(1));
    cropped
        .resize_exact(target.0, target.1, image::imageops::FilterType::Triangle)
        .to_rgba8()
}

/// Dimensions of the composed image for `count` shots.
pub fn output_dimensions(kind: LayoutKind, count: usize, opts: &LayoutOptions) -> (u32, u32) {
    let (cw, ch) = opts.cell;
    match kind {
        LayoutKind::Strip => {
            let n = count.max(1) as u32;
            (
                cw + 2 * opts.margin,
                n * ch + (n - 1) * opts.gap + 2 * opts.margin,
            )
        }
        LayoutKind::Grid2x2 => (
            2 * cw + opts.gap + 2 * opts.margin,
            2 * ch + opts.gap + 2 * opts.margin,
        ),
    }
}

/// A rendered photo cell in output pixels. Shared by composition and guest
/// interaction so the clickable slots match the actual saved layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PhotoCell {
    pub shot: usize,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

pub fn photo_cells(count: usize, kind: LayoutKind, opts: &LayoutOptions) -> Vec<PhotoCell> {
    let limit = match kind {
        LayoutKind::Strip => count,
        LayoutKind::Grid2x2 => count.min(4),
    };
    (0..limit)
        .map(|shot| {
            let (column, row) = match kind {
                LayoutKind::Strip => (0, shot as u32),
                LayoutKind::Grid2x2 => ((shot % 2) as u32, (shot / 2) as u32),
            };
            PhotoCell {
                shot,
                x: opts.margin + column * (opts.cell.0 + opts.gap),
                y: opts.margin + row * (opts.cell.1 + opts.gap),
                width: opts.cell.0,
                height: opts.cell.1,
            }
        })
        .collect()
}

/// Compose `shots` into a layout. For `Grid2x2` only the first 4 shots are used.
pub fn compose(
    shots: &[DynamicImage],
    kind: LayoutKind,
    opts: &LayoutOptions,
) -> Result<DynamicImage, LayoutError> {
    if shots.is_empty() {
        return Err(LayoutError::Empty);
    }
    let (out_w, out_h) = output_dimensions(kind, shots.len(), opts);
    let mut canvas: ImageBuffer<Rgba<u8>, Vec<u8>> =
        ImageBuffer::from_pixel(out_w, out_h, Rgba(opts.background));
    for cell in photo_cells(shots.len(), kind, opts) {
        let cell_img = cover_crop_resize(&shots[cell.shot], (cell.width, cell.height));
        image::imageops::overlay(&mut canvas, &cell_img, cell.x as i64, cell.y as i64);
    }
    Ok(DynamicImage::ImageRgba8(canvas))
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Rgb};

    fn solid(width: u32, height: u32, [r, g, b]: [u8; 3]) -> DynamicImage {
        DynamicImage::ImageRgb8(ImageBuffer::from_pixel(width, height, Rgb([r, g, b])))
    }

    fn opts() -> LayoutOptions {
        LayoutOptions {
            cell: (100, 80),
            margin: 10,
            gap: 4,
            background: [255, 255, 255, 255],
        }
    }

    #[test]
    fn strip_dimensions_and_border() {
        let shots = vec![
            solid(320, 240, [255, 0, 0]),
            solid(320, 240, [0, 255, 0]),
            solid(320, 240, [0, 0, 255]),
        ];
        let out = compose(&shots, LayoutKind::Strip, &opts()).unwrap();
        // width = cell + 2*margin, height = 3*cell + 2*gap + 2*margin
        assert_eq!(out.dimensions(), (120, 3 * 80 + 2 * 4 + 20));
        let rgba = out.to_rgba8();
        // Border pixels.
        assert_eq!(rgba.get_pixel(0, 0).0, [255, 255, 255, 255]);
        assert_eq!(rgba.get_pixel(119, 267).0, [255, 255, 255, 255]);
        // Center of first cell should be the shot's red.
        assert_eq!(rgba.get_pixel(60, 50).0, [255, 0, 0, 255]);
        // Center of second cell should be green.
        assert_eq!(rgba.get_pixel(60, 50 + 84).0, [0, 255, 0, 255]);
    }

    #[test]
    fn grid_uses_first_four_shots() {
        let shots = vec![
            solid(10, 10, [1, 1, 1]),
            solid(10, 10, [2, 2, 2]),
            solid(10, 10, [3, 3, 3]),
            solid(10, 10, [4, 4, 4]),
            solid(10, 10, [9, 9, 9]),
        ];
        let out = compose(&shots, LayoutKind::Grid2x2, &opts()).unwrap();
        assert_eq!(out.dimensions(), (2 * 100 + 4 + 20, 2 * 80 + 4 + 20));
        let rgba = out.to_rgba8();
        // Cell centers: (60, 50), (164, 50), (60, 134), (164, 134)
        assert_eq!(rgba.get_pixel(60, 50).0, [1, 1, 1, 255]);
        assert_eq!(rgba.get_pixel(164, 50).0, [2, 2, 2, 255]);
        assert_eq!(rgba.get_pixel(60, 134).0, [3, 3, 3, 255]);
        assert_eq!(rgba.get_pixel(164, 134).0, [4, 4, 4, 255]);
    }

    #[test]
    fn non_cell_aspect_is_cropped_not_stretched() {
        let shots = vec![solid(400, 100, [9, 9, 9])]; // very wide source
        let out = compose(&shots, LayoutKind::Strip, &opts()).unwrap();
        // Cropped to cell aspect, so it paints every cell pixel (no letterbox).
        assert_eq!(out.dimensions(), (120, 80 + 20));
        assert_eq!(out.to_rgba8().get_pixel(10, 10).0, [9, 9, 9, 255]);
    }

    #[test]
    fn empty_shots_error() {
        assert!(matches!(
            compose(&[], LayoutKind::Strip, &opts()),
            Err(LayoutError::Empty)
        ));
    }
}
