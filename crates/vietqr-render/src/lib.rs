//! QR SVG rendering with default error correction M (or an explicit level) and a four-module quiet zone.

pub use qrcode::{EcLevel, types::QrError};
use qrcode::{QrCode, render::svg};

/// Encodes any payload as QR, without validating VietQR semantics.
pub fn render_svg(payload: &str) -> Result<String, QrError> {
    render_svg_with_ec_level(payload, EcLevel::M)
}

/// Encodes with an explicit correction level; the quiet zone remains four modules.
pub fn render_svg_with_ec_level(payload: &str, ec_level: EcLevel) -> Result<String, QrError> {
    let code = QrCode::with_error_correction_level(payload.as_bytes(), ec_level)?;
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
    fn render_svg_with_ec_level_h_keeps_quiet_zone() {
        let payload = "logo overlay payload";
        let code = QrCode::with_error_correction_level(payload, EcLevel::H).unwrap();
        let svg = render_svg_with_ec_level(payload, EcLevel::H).unwrap();
        let side = code.width() + 8;
        assert!(svg.starts_with("<svg"));
        assert!(svg.contains(&format!("viewBox=\"0 0 {side} {side}\"")));
        assert_eq!(
            render_svg_with_ec_level(payload, EcLevel::M).unwrap(),
            render_svg(payload).unwrap()
        );
    }

    #[test]
    fn render_svg_rejects_payload_above_qr_capacity() {
        assert_eq!(render_svg(&"x".repeat(10_000)), Err(QrError::DataTooLong));
    }
}
