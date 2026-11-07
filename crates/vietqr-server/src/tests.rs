use super::*;
use axum::{
    body::{Body, to_bytes},
    http::Request,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use image::{DynamicImage, ImageFormat, Rgb, RgbImage};
use rqrr::BitGrid;
use serde_json::{Value, json};
use std::io::Cursor;
use tower::ServiceExt;

fn request(body: &Value) -> Request<Body> {
    Request::post("/v1/qr")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

fn input() -> Value {
    json!({"bank_bin": "970436", "account": "0000000000", "amount": 10000, "memo": "TEST M1", "format": "png"})
}

fn handler() -> Router {
    Router::new()
        .route("/v1/qr", post(qr))
        .layer(DefaultBodyLimit::max(360 * 1024))
}

async fn body(response: Response) -> Vec<u8> {
    to_bytes(response.into_body(), 2 * 1024 * 1024)
        .await
        .unwrap()
        .to_vec()
}

fn logo(format: ImageFormat) -> String {
    let mut image = RgbImage::from_pixel(64, 40, Rgb([32, 90, 190]));
    for y in 8..32 {
        for x in 24..40 {
            image.put_pixel(x, y, Rgb([255, 255, 255]));
        }
    }
    let mut bytes = Cursor::new(Vec::new());
    DynamicImage::ImageRgb8(image)
        .write_to(&mut bytes, format)
        .unwrap();
    STANDARD.encode(bytes.into_inner())
}

#[tokio::test]
async fn handler_validation_uses_core_error_variants() {
    for (field, value, code) in [
        ("bank_bin", json!("97043x"), "InvalidBin"),
        ("account", json!("bad account"), "InvalidAccount"),
        ("amount", json!(0), "InvalidAmount"),
        (
            "amount",
            json!(vietqr_core::MAX_AMOUNT + 1),
            "InvalidAmount",
        ),
        ("memo", json!("😀"), "InvalidMemo"),
        ("name", json!("\n"), "InvalidName"),
    ] {
        let mut input = input();
        input[field] = value;
        let response = handler().oneshot(request(&input)).await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{field}");
        let error: Value = serde_json::from_slice(&body(response).await).unwrap();
        assert_eq!(error["code"], code);
    }
}

#[tokio::test]
async fn handler_rejects_invalid_json_types_options_and_body_size() {
    for (field, value) in [
        ("format", json!("pdf")),
        ("service", json!("wallet")),
        ("profile", json!("other")),
        ("unknown", json!(true)),
        ("account", json!(123)),
        ("amount", json!(-1)),
        ("amount", json!(1.5)),
        ("memo", json!("x".repeat(360 * 1024))),
    ] {
        let mut input = input();
        input[field] = value;
        let response = handler().oneshot(request(&input)).await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{field}");
        let error: Value = serde_json::from_slice(&body(response).await).unwrap();
        assert_eq!(error["code"], "InvalidRequest");
    }
    let response = handler()
        .oneshot(
            Request::post("/v1/qr")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from("{"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn handler_svg_defaults_and_strict_card_profile() {
    for strict in [false, true] {
        let mut input = input();
        input["format"] = json!("svg");
        input["name"] = json!("TÊN TEST");
        if strict {
            input["profile"] = json!("strict");
            input["service"] = json!("card");
        }
        let response = handler().oneshot(request(&input)).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::CONTENT_TYPE], "image/svg+xml");
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        let payload = response.headers()["x-vietqr-payload"]
            .to_str()
            .unwrap()
            .to_owned();
        let parsed = vietqr_core::parse(&payload).unwrap();
        assert_eq!(parsed.name, Some("TEN TEST"));
        assert_eq!(
            parsed.service,
            if strict {
                Service::Card
            } else {
                Service::Account
            }
        );
        let expected = vietqr_core::generate_with_profile(
            &Transfer {
                bank_bin: "970436",
                account: "0000000000",
                amount: Some(10000),
                memo: Some("TEST M1"),
                name: Some("TÊN TEST"),
                service: parsed.service,
            },
            if strict {
                Profile::Strict
            } else {
                Profile::Npm
            },
        )
        .unwrap();
        assert_eq!(payload, expected);
        assert!(body(response).await.starts_with(b"<svg"));
    }
    let mut input = input();
    input["memo"] = json!("x".repeat(26));
    input["profile"] = json!("strict");
    assert_eq!(
        handler().oneshot(request(&input)).await.unwrap().status(),
        StatusCode::BAD_REQUEST
    );
    input["profile"] = json!("npm");
    assert_eq!(
        handler().oneshot(request(&input)).await.unwrap().status(),
        StatusCode::OK
    );
}

#[tokio::test]
async fn handler_png_logo_decodes_exact_payload_and_preserves_quiet_zone() {
    for format in [ImageFormat::Png, ImageFormat::Jpeg] {
        for long in [false, true] {
            let mut input = input();
            input["logo"] = json!(logo(format));
            if long {
                input["account"] = json!("A".repeat(50));
                input["name"] = json!("N".repeat(50));
                input["memo"] = json!("M".repeat(50));
                input["amount"] = json!(vietqr_core::MAX_AMOUNT);
            }
            let response = handler().oneshot(request(&input)).await.unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(response.headers()[header::CONTENT_TYPE], "image/png");
            let payload = response.headers()["x-vietqr-payload"]
                .to_str()
                .unwrap()
                .to_owned();
            vietqr_core::validate(&payload).unwrap();
            let image = image::load_from_memory(&body(response).await)
                .unwrap()
                .to_luma8();
            for (x, y, pixel) in image.enumerate_pixels() {
                if x < 32 || y < 32 || x >= image.width() - 32 || y >= image.height() - 32 {
                    assert_eq!(pixel.0, [255]);
                }
            }
            let mut prepared = rqrr::PreparedImage::prepare(image);
            let grids = prepared.detect_grids();
            assert_eq!(grids.len(), 1);
            let (metadata, decoded) = grids[0].decode().unwrap();
            assert_eq!(metadata.ecc_level, 2); // QR format bits 10 denote H.
            assert_eq!(decoded, payload);
            let modules =
                qrcode::QrCode::with_error_correction_level(payload.as_bytes(), qrcode::EcLevel::H)
                    .unwrap()
                    .width();
            assert_eq!(grids[0].grid.size(), modules);
        }
    }
}

#[tokio::test]
async fn handler_rejects_unsafe_oversized_and_corrupt_logos() {
    let mut huge = Cursor::new(Vec::new());
    DynamicImage::ImageRgb8(RgbImage::new(2049, 1))
        .write_to(&mut huge, ImageFormat::Png)
        .unwrap();
    for logo in [
        "not base64".to_owned(),
        STANDARD.encode(b"<svg><script>alert(1)</script></svg>"),
        STANDARD.encode(vec![0u8; 256 * 1024 + 1]),
        STANDARD.encode(b"\x89PNG\r\n\x1a\ncorrupt"),
        STANDARD.encode(huge.into_inner()),
    ] {
        let mut input = input();
        input["logo"] = json!(logo);
        let response = handler().oneshot(request(&input)).await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let error: Value = serde_json::from_slice(&body(response).await).unwrap();
        assert_eq!(error["code"], "InvalidLogo");
    }
}

#[tokio::test]
async fn handler_svg_logo_embeds_reencoded_png_only() {
    let mut input = input();
    input["format"] = json!("svg");
    input["logo"] = json!(logo(ImageFormat::Jpeg));
    let response = handler().oneshot(request(&input)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let svg = String::from_utf8(body(response).await).unwrap();
    let encoded = svg
        .split("data:image/png;base64,")
        .nth(1)
        .unwrap()
        .split('"')
        .next()
        .unwrap();
    let bytes = STANDARD.decode(encoded).unwrap();
    assert_eq!(image::guess_format(&bytes).unwrap(), ImageFormat::Png);
    let tile = image::load_from_memory(&bytes).unwrap().to_rgba8();
    assert_eq!(tile.get_pixel(0, 0).0, [255, 255, 255, 255]);
    assert!(!svg.contains("image/jpeg"));
    assert!(!svg.contains("<script"));
}

#[test]
fn rate_limiter_is_per_key_expires_and_cleans_up() {
    let mut limiter = RateLimiter::default();
    let now = Instant::now();
    for _ in 0..60 {
        assert!(limiter.allow(1, now));
    }
    assert!(!limiter.allow(1, now + Duration::from_secs(59)));
    assert!(limiter.allow(2, now));
    assert!(limiter.allow(1, now + Duration::from_secs(60)));
    assert_eq!(limiter.windows.len(), 1);
}

#[test]
fn key_hash_is_sha256() {
    assert_eq!(
        key_hash("abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

#[tokio::test]
async fn health_and_missing_or_malformed_auth_need_no_database() {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy("postgres://unused:unused@127.0.0.1:1/unused")
        .unwrap();
    let app = router(AppState::new(pool));
    assert_eq!(
        app.clone()
            .oneshot(Request::get("/healthz").body(Body::empty()).unwrap())
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    for auth in [None, Some("Basic nope"), Some("Bearer ")] {
        let mut request = request(&input());
        if let Some(auth) = auth {
            request
                .headers_mut()
                .insert(header::AUTHORIZATION, HeaderValue::from_str(auth).unwrap());
        }
        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(response.headers()[header::WWW_AUTHENTICATE], "Bearer");
    }
}
