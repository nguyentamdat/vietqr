use std::io::Cursor;

use base64::{Engine, engine::general_purpose::STANDARD};
use image::{DynamicImage, ImageFormat, ImageReader, Rgba, RgbaImage, imageops};
use qrcode::{Color, EcLevel, QrCode};

use crate::{ApiError, Format};

const MAX_LOGO_BYTES: usize = 256 * 1024;
const SCALE: u32 = 8;
const QUIET: u32 = 4;

fn invalid_logo() -> ApiError {
    ApiError::bad_request(
        "InvalidLogo",
        "Logo must be base64 PNG/JPEG, at most 256 KiB and 2048x2048 pixels",
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
    limits.max_image_width = Some(2048);
    limits.max_image_height = Some(2048);
    limits.max_alloc = Some(32 * 1024 * 1024);
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

pub(super) fn render(
    payload: &str,
    format: Format,
    logo: Option<&str>,
) -> Result<Vec<u8>, ApiError> {
    let logo = logo.map(decode_logo).transpose()?;
    let ec = if logo.is_some() {
        EcLevel::H
    } else {
        EcLevel::M
    };
    let code = QrCode::with_error_correction_level(payload.as_bytes(), ec)
        .map_err(|_| ApiError::bad_request("QrCapacity", "Payload exceeds QR capacity"))?;
    let width = code.width() as u32;
    // Padding included: a quarter-width square covers at most 6.25% of module area.
    let logo_modules = width / 4;
    let logo_start = QUIET + (width - logo_modules) / 2;
    let tile = logo.map(|logo| {
        let side = logo_modules * SCALE;
        let mut tile = RgbaImage::from_pixel(side, side, Rgba([255, 255, 255, 255]));
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
    match format {
        Format::Svg => {
            let mut svg = vietqr_render::render_svg_with_ec_level(payload, ec)
                .map_err(|_| ApiError::internal())?;
            if let Some(tile) = tile {
                // Embed only freshly encoded raster bytes, never user-supplied SVG or metadata.
                let encoded = STANDARD.encode(png(DynamicImage::ImageRgba8(tile))?);
                let image = format!(
                    "<image x=\"{logo_start}\" y=\"{logo_start}\" width=\"{logo_modules}\" height=\"{logo_modules}\" href=\"data:image/png;base64,{encoded}\"/></svg>"
                );
                svg.truncate(svg.len() - "</svg>".len());
                svg.push_str(&image);
            }
            Ok(svg.into_bytes())
        }
        Format::Png => {
            let side = (width + 2 * QUIET) * SCALE;
            let mut image = RgbaImage::from_pixel(side, side, Rgba([255, 255, 255, 255]));
            for y in 0..width {
                for x in 0..width {
                    if code[(x as usize, y as usize)] == Color::Dark {
                        for dy in 0..SCALE {
                            for dx in 0..SCALE {
                                image.put_pixel(
                                    (x + QUIET) * SCALE + dx,
                                    (y + QUIET) * SCALE + dy,
                                    Rgba([0, 0, 0, 255]),
                                );
                            }
                        }
                    }
                }
            }
            if let Some(tile) = tile {
                imageops::overlay(
                    &mut image,
                    &tile,
                    i64::from(logo_start * SCALE),
                    i64::from(logo_start * SCALE),
                );
            }
            png(DynamicImage::ImageRgba8(image))
        }
    }
}
