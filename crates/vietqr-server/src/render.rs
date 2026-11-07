use std::io::Cursor;

use ab_glyph::{Font, FontRef, PxScale, ScaleFont, point};
use base64::{Engine, engine::general_purpose::STANDARD};
use image::{DynamicImage, ImageFormat, ImageReader, Rgba, RgbaImage, imageops};
use qrcode::{Color, EcLevel, QrCode};

use crate::{ApiError, Format};

const MAX_LOGO_BYTES: usize = 256 * 1024;
const SCALE: u32 = 8;
const QUIET: u32 = 4;
pub(super) const WATERMARK: &str = "Tạo bởi VietQR";

pub(super) struct Style {
    pub logo: Option<Vec<u8>>,
    pub foreground: [u8; 3],
    pub background: [u8; 3],
    pub border: bool,
    pub shop_name: Option<String>,
    pub watermark: bool,
}
impl Default for Style {
    fn default() -> Self {
        Self {
            logo: None,
            foreground: [0; 3],
            background: [255; 3],
            border: false,
            shop_name: None,
            watermark: false,
        }
    }
}

pub(super) fn color(value: &str) -> Option<[u8; 3]> {
    if value.len() != 7
        || !value.starts_with('#')
        || !value.as_bytes()[1..].iter().all(u8::is_ascii_hexdigit)
    {
        return None;
    }
    Some([
        u8::from_str_radix(&value[1..3], 16).ok()?,
        u8::from_str_radix(&value[3..5], 16).ok()?,
        u8::from_str_radix(&value[5..7], 16).ok()?,
    ])
}
fn luminance(rgb: [u8; 3]) -> f64 {
    let linear = rgb.map(|v| {
        let s = f64::from(v) / 255.0;
        if s <= 0.04045 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powf(2.4)
        }
    });
    0.2126 * linear[0] + 0.7152 * linear[1] + 0.0722 * linear[2]
}

pub(super) fn validate_colors(fg: &str, bg: &str) -> Result<(), ApiError> {
    let valid = color(fg).zip(color(bg)).is_some_and(|(fg, bg)| {
        let (dark, light) = (luminance(fg), luminance(bg));
        // Conservative QR policy: dark modules, light background, WCAG contrast at least 7:1.
        dark <= 0.2 && light >= 0.8 && (light + 0.05) / (dark + 0.05) >= 7.0
    });
    if valid {
        Ok(())
    } else {
        Err(ApiError::bad_request(
            "InvalidColors",
            "Use #RRGGBB dark foreground, light background, contrast >= 7:1",
        ))
    }
}

fn invalid_logo() -> ApiError {
    ApiError::bad_request(
        "InvalidLogo",
        "Logo must be base64 PNG/JPEG, at most 256 KiB and 1024x1024 pixels",
    )
}

fn decode_logo(encoded: &str) -> Result<DynamicImage, ApiError> {
    if encoded.len() > MAX_LOGO_BYTES.div_ceil(3) * 4 {
        return Err(invalid_logo());
    }
    let bytes = STANDARD.decode(encoded).map_err(|_| invalid_logo())?;
    if bytes.len() > MAX_LOGO_BYTES {
        return Err(invalid_logo());
    }
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| invalid_logo())?;
    if !matches!(reader.format(), Some(ImageFormat::Png | ImageFormat::Jpeg)) {
        return Err(invalid_logo());
    }
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(1024);
    limits.max_image_height = Some(1024);
    limits.max_alloc = Some(16 * 1024 * 1024);
    reader.limits(limits);
    reader.decode().map_err(|_| invalid_logo())
}

fn png(image: DynamicImage) -> Result<Vec<u8>, ApiError> {
    let mut output = Cursor::new(Vec::new());
    image
        .write_to(&mut output, ImageFormat::Png)
        .map_err(|_| ApiError::internal())?;
    Ok(output.into_inner())
}

pub(super) fn normalize_logo(encoded: &str) -> Result<Vec<u8>, ApiError> {
    // Store only the small raster actually used by the QR, never original metadata or SVG.
    png(decode_logo(encoded)?.resize(240, 240, imageops::FilterType::Lanczos3))
}

fn label(text: &str, width: u32, size: f32) -> RgbaImage {
    let font =
        FontRef::try_from_slice(include_bytes!("../assets/DejaVuSans.ttf")).expect("bundled font");
    let scaled = font.as_scaled(PxScale::from(size));
    let text_width: f32 = text
        .chars()
        .map(|c| scaled.h_advance(font.glyph_id(c)))
        .sum();
    let scale = size * ((width as f32 - 32.0) / text_width.max(1.0)).min(1.0);
    let scaled = font.as_scaled(PxScale::from(scale));
    let mut image = RgbaImage::from_pixel(width, 36, Rgba([255; 4]));
    let mut x = ((width as f32 - text_width * scale / size) / 2.0).max(0.0);
    for c in text.chars() {
        let id = font.glyph_id(c);
        let glyph = id.with_scale_and_position(scale, point(x, 26.0));
        if let Some(outlined) = font.outline_glyph(glyph) {
            let bounds = outlined.px_bounds();
            outlined.draw(|gx, gy, coverage| {
                let px = bounds.min.x as i32 + gx as i32;
                let py = bounds.min.y as i32 + gy as i32;
                if px >= 0 && py >= 0 && px < width as i32 && py < 36 {
                    let value = (255.0 * (1.0 - coverage)) as u8;
                    image.put_pixel(px as u32, py as u32, Rgba([value, value, value, 255]));
                }
            });
        }
        x += scaled.h_advance(id);
    }
    image
}

pub(super) fn render(payload: &str, format: Format, style: &Style) -> Result<Vec<u8>, ApiError> {
    let logo = style
        .logo
        .as_deref()
        .map(image::load_from_memory)
        .transpose()
        .map_err(|_| ApiError::internal())?;
    let ec = if logo.is_some() {
        EcLevel::H
    } else {
        EcLevel::M
    };
    let code = QrCode::with_error_correction_level(payload.as_bytes(), ec)
        .map_err(|_| ApiError::bad_request("QrCapacity", "Payload exceeds QR capacity"))?;
    let width = code.width() as u32;
    let side = (width + 2 * QUIET) * SCALE;
    // Padding included: a quarter-width square covers at most 6.25% of module area.
    let logo_modules = width / 4;
    let logo_start = (QUIET + (width - logo_modules) / 2) * SCALE;
    let tile = logo.map(|logo| {
        let side = logo_modules * SCALE;
        let mut tile = RgbaImage::from_pixel(side, side, Rgba([255; 4]));
        let logo = logo.resize(
            side - 2 * SCALE,
            side - 2 * SCALE,
            imageops::FilterType::Lanczos3,
        );
        imageops::overlay(
            &mut tile,
            &logo,
            i64::from((side - logo.width()) / 2),
            i64::from((side - logo.height()) / 2),
        );
        tile
    });
    let mut labels = Vec::new();
    if let Some(name) = &style.shop_name {
        labels.push(label(name, side, 24.0));
    }
    if style.watermark {
        labels.push(label(WATERMARK, side, 16.0));
    }
    let height = side + labels.len() as u32 * 36;
    let fg = Rgba([
        style.foreground[0],
        style.foreground[1],
        style.foreground[2],
        255,
    ]);
    let bg = Rgba([
        style.background[0],
        style.background[1],
        style.background[2],
        255,
    ]);
    match format {
        Format::Svg => {
            let fg = format!("#{:02x}{:02x}{:02x}", fg[0], fg[1], fg[2]);
            let bg = format!("#{:02x}{:02x}{:02x}", bg[0], bg[1], bg[2]);
            let mut svg = format!(
                "<svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\" width=\"{side}\" height=\"{height}\" viewBox=\"0 0 {side} {height}\"><rect width=\"{side}\" height=\"{height}\" fill=\"white\"/><rect width=\"{side}\" height=\"{side}\" fill=\"{bg}\"/><path fill=\"{fg}\" d=\""
            );
            for y in 0..width {
                for x in 0..width {
                    if code[(x as usize, y as usize)] == Color::Dark {
                        svg.push_str(&format!(
                            "M{},{}h{SCALE}v{SCALE}h-{SCALE}z ",
                            (x + QUIET) * SCALE,
                            (y + QUIET) * SCALE
                        ));
                    }
                }
            }
            svg.push_str("\"/>");
            if style.border {
                svg.push_str(&format!("<rect x=\"1\" y=\"1\" width=\"{}\" height=\"{}\" fill=\"none\" stroke=\"{fg}\" stroke-width=\"2\"/>", side - 2, side - 2));
            }
            if let Some(tile) = tile {
                svg.push_str(&svg_image(tile, logo_start, logo_start)?);
            }
            for (i, label) in labels.into_iter().enumerate() {
                svg.push_str(&svg_image(label, 0, side + i as u32 * 36)?);
            }
            svg.push_str("</svg>");
            Ok(svg.into_bytes())
        }
        Format::Png => {
            let mut image = RgbaImage::from_pixel(side, height, Rgba([255; 4]));
            for y in 0..side {
                for x in 0..side {
                    image.put_pixel(x, y, bg);
                }
            }
            for y in 0..width {
                for x in 0..width {
                    if code[(x as usize, y as usize)] == Color::Dark {
                        for dy in 0..SCALE {
                            for dx in 0..SCALE {
                                image.put_pixel(
                                    (x + QUIET) * SCALE + dx,
                                    (y + QUIET) * SCALE + dy,
                                    fg,
                                );
                            }
                        }
                    }
                }
            }
            if style.border {
                for i in 0..side {
                    image.put_pixel(i, 0, fg);
                    image.put_pixel(i, side - 1, fg);
                    image.put_pixel(0, i, fg);
                    image.put_pixel(side - 1, i, fg);
                }
            }
            if let Some(tile) = tile {
                imageops::overlay(
                    &mut image,
                    &tile,
                    i64::from(logo_start),
                    i64::from(logo_start),
                );
            }
            for (i, label) in labels.into_iter().enumerate() {
                imageops::overlay(&mut image, &label, 0, i64::from(side + i as u32 * 36));
            }
            png(DynamicImage::ImageRgba8(image))
        }
    }
}

fn svg_image(image: RgbaImage, x: u32, y: u32) -> Result<String, ApiError> {
    let (width, height) = image.dimensions();
    let encoded = STANDARD.encode(png(DynamicImage::ImageRgba8(image))?);
    Ok(format!(
        "<image x=\"{x}\" y=\"{y}\" width=\"{width}\" height=\"{height}\" href=\"data:image/png;base64,{encoded}\" xlink:href=\"data:image/png;base64,{encoded}\"/>"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn logo_limits_cover_both_axes_and_accept_1024_rgba16() {
        for (width, height) in [(1025, 1), (1, 1025), (1024, 1024)] {
            let image = DynamicImage::ImageRgba16(image::ImageBuffer::from_pixel(
                width,
                height,
                Rgba([65535u16; 4]),
            ));
            let mut bytes = Cursor::new(Vec::new());
            image.write_to(&mut bytes, ImageFormat::Png).unwrap();
            let normalized = normalize_logo(&STANDARD.encode(bytes.into_inner()));
            if width <= 1024 && height <= 1024 {
                let image = image::load_from_memory(&normalized.unwrap()).unwrap();
                assert!(image.width() <= 240 && image.height() <= 240);
            } else {
                assert!(normalized.is_err());
            }
        }
    }

    #[test]
    fn colors_require_dark_on_light_high_contrast_hex() {
        for (fg, bg) in [("#000000", "#ffffff"), ("#123456", "#FFFFF0")] {
            assert!(validate_colors(fg, bg).is_ok());
        }
        for (fg, bg) in [
            ("#ffffff", "#000000"),
            ("#777777", "#ffffff"),
            ("#000000", "#888888"),
            ("red", "#ffffff"),
            ("#000", "#fff"),
            ("#000000", "#gggggg"),
        ] {
            assert!(validate_colors(fg, bg).is_err());
        }
    }
}
