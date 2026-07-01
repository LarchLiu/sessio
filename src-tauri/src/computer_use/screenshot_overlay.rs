use std::path::Path;

use image::{DynamicImage, ImageBuffer, ImageFormat, Rgba, RgbaImage};

use super::provider::Point;

const GRID_SPACING: u32 = 50;
const AXIS_BAND: u32 = 24;

pub fn render_reference_overlay_png(
    path: &Path,
    click_marker: Option<Point>,
) -> Result<Vec<u8>, String> {
    let image = image::open(path).map_err(|e| format!("open screenshot overlay source: {e}"))?;
    let mut canvas = image.to_rgba8();
    draw_grid(&mut canvas);
    draw_axes(&mut canvas);
    if let Some(marker) = click_marker {
        draw_click_marker(&mut canvas, marker);
    }

    let mut output = Vec::new();
    DynamicImage::ImageRgba8(canvas)
        .write_to(&mut std::io::Cursor::new(&mut output), ImageFormat::Png)
        .map_err(|e| format!("encode screenshot overlay: {e}"))?;
    Ok(output)
}

fn draw_grid(image: &mut RgbaImage) {
    let width = image.width();
    let height = image.height();
    let major = Rgba([64, 196, 255, 140]);
    let minor = Rgba([64, 196, 255, 60]);

    let mut x = 0;
    while x < width {
        let color = if x % (GRID_SPACING * 2) == 0 {
            major
        } else {
            minor
        };
        for y in 0..height {
            blend_pixel(image, x, y, color);
        }
        x = x.saturating_add(GRID_SPACING);
    }

    let mut y = 0;
    while y < height {
        let color = if y % (GRID_SPACING * 2) == 0 {
            major
        } else {
            minor
        };
        for x in 0..width {
            blend_pixel(image, x, y, color);
        }
        y = y.saturating_add(GRID_SPACING);
    }
}

fn draw_axes(image: &mut RgbaImage) {
    let width = image.width();
    let height = image.height();
    let band = Rgba([14, 18, 28, 180]);
    let tick = Rgba([255, 255, 255, 210]);

    for y in 0..height.min(AXIS_BAND) {
        for x in 0..width {
            blend_pixel(image, x, y, band);
        }
    }
    for x in 0..width.min(AXIS_BAND) {
        for y in 0..height {
            blend_pixel(image, x, y, band);
        }
    }

    let mut x = 0;
    while x < width {
        let tick_height = if x % (GRID_SPACING * 2) == 0 { 14 } else { 9 };
        for y in 0..tick_height.min(height) {
            blend_pixel(image, x, y, tick);
            if x + 1 < width {
                blend_pixel(image, x + 1, y, tick);
            }
        }
        x = x.saturating_add(GRID_SPACING);
    }

    let mut y = 0;
    while y < height {
        let tick_width = if y % (GRID_SPACING * 2) == 0 { 14 } else { 9 };
        for x in 0..tick_width.min(width) {
            blend_pixel(image, x, y, tick);
            if y + 1 < height {
                blend_pixel(image, x, y + 1, tick);
            }
        }
        y = y.saturating_add(GRID_SPACING);
    }
}

fn draw_click_marker(image: &mut RgbaImage, marker: Point) {
    if !marker.x.is_finite() || !marker.y.is_finite() {
        return;
    }
    let cx = marker.x.round() as i32;
    let cy = marker.y.round() as i32;
    let ring = Rgba([255, 95, 31, 230]);
    let fill = Rgba([255, 255, 255, 200]);
    let accent = Rgba([255, 95, 31, 255]);

    fill_circle(image, cx, cy, 7, fill);
    stroke_circle(image, cx, cy, 14, 3, ring);
    draw_crosshair(image, cx, cy, 22, accent);
}

fn draw_crosshair(image: &mut RgbaImage, cx: i32, cy: i32, radius: i32, color: Rgba<u8>) {
    for dx in -radius..=radius {
        if dx.abs() <= 5 {
            continue;
        }
        paint(image, cx + dx, cy, color);
    }
    for dy in -radius..=radius {
        if dy.abs() <= 5 {
            continue;
        }
        paint(image, cx, cy + dy, color);
    }
}

fn fill_circle(image: &mut RgbaImage, cx: i32, cy: i32, radius: i32, color: Rgba<u8>) {
    let radius_sq = radius * radius;
    for dy in -radius..=radius {
        for dx in -radius..=radius {
            if dx * dx + dy * dy <= radius_sq {
                paint(image, cx + dx, cy + dy, color);
            }
        }
    }
}

fn stroke_circle(
    image: &mut RgbaImage,
    cx: i32,
    cy: i32,
    radius: i32,
    thickness: i32,
    color: Rgba<u8>,
) {
    let outer = radius * radius;
    let inner = (radius - thickness).max(0);
    let inner_sq = inner * inner;
    for dy in -radius..=radius {
        for dx in -radius..=radius {
            let dist = dx * dx + dy * dy;
            if dist <= outer && dist >= inner_sq {
                paint(image, cx + dx, cy + dy, color);
            }
        }
    }
}

fn paint(image: &mut RgbaImage, x: i32, y: i32, color: Rgba<u8>) {
    if x < 0 || y < 0 {
        return;
    }
    let x = x as u32;
    let y = y as u32;
    if x >= image.width() || y >= image.height() {
        return;
    }
    blend_pixel(image, x, y, color);
}

fn blend_pixel(image: &mut ImageBuffer<Rgba<u8>, Vec<u8>>, x: u32, y: u32, src: Rgba<u8>) {
    let dst = image.get_pixel_mut(x, y);
    let alpha = src[3] as f32 / 255.0;
    let inv_alpha = 1.0 - alpha;
    let blended = [
        (src[0] as f32 * alpha + dst[0] as f32 * inv_alpha).round() as u8,
        (src[1] as f32 * alpha + dst[1] as f32 * inv_alpha).round() as u8,
        (src[2] as f32 * alpha + dst[2] as f32 * inv_alpha).round() as u8,
        255,
    ];
    *dst = Rgba(blended);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlay_render_returns_png_bytes() {
        let dir =
            std::env::temp_dir().join(format!("sessio-cu-overlay-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("source.png");
        let image = RgbaImage::from_pixel(120, 80, Rgba([30, 30, 30, 255]));
        image.save(&path).unwrap();

        let overlay =
            render_reference_overlay_png(&path, Some(Point { x: 40.0, y: 35.0 })).unwrap();

        assert!(!overlay.is_empty());
        assert_eq!(&overlay[..8], b"\x89PNG\r\n\x1a\n");
        let _ = std::fs::remove_dir_all(dir);
    }
}
