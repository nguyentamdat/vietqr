//! QR SVG rendering with error correction M and a four-module quiet zone.

pub use qrcode::types::QrError;
use qrcode::{EcLevel, QrCode, render::svg};

/// Encodes any payload as QR, without validating VietQR semantics.
pub fn render_svg(payload: &str) -> Result<String, QrError> {
    let code = QrCode::with_error_correction_level(payload.as_bytes(), EcLevel::M)?;
    let image = code
        .render::<svg::Color>()
        .quiet_zone(true)
        .module_dimensions(1, 1)
        .build();
    // The browser-facing API returns the SVG element, not an XML document header.
    Ok(image[image
        .find("<svg")
        .expect("qrcode SVG renderer emits an SVG element")..]
        .to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_svg_has_quiet_zone_and_no_payload_markup() {
        let payload = "<script>alert('x')</script>";
        let code = QrCode::with_error_correction_level(payload, EcLevel::M).unwrap();
        let svg = render_svg(payload).unwrap();
        let side = code.width() + 8;
        assert!(svg.starts_with("<svg"));
        assert!(svg.contains(&format!("viewBox=\"0 0 {side} {side}\"")));
        assert!(!svg.contains("<script>"));
        assert!(svg.ends_with("</svg>"));
    }

    #[test]
    fn render_svg_rejects_payload_above_qr_capacity() {
        assert_eq!(render_svg(&"x".repeat(10_000)), Err(QrError::DataTooLong));
    }
}
