use axum::{
    Router,
    body::{Body, to_bytes},
    http::{HeaderMap, Request, StatusCode},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgPoolOptions};
use tower::ServiceExt;
use vietqr_server::{AppState, MIGRATOR, create_key, key_hash, router};

async fn call(
    app: &Router,
    method: &str,
    path: &str,
    key: Option<&str>,
    body: Value,
) -> (StatusCode, HeaderMap, Vec<u8>) {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .extension(axum::extract::ConnectInfo(
            "127.0.0.1:1234".parse::<std::net::SocketAddr>().unwrap(),
        ))
        .header("content-type", "application/json");
    if let Some(key) = key {
        request = request.header("authorization", format!("Bearer {key}"));
    }
    let response = app
        .clone()
        .oneshot(request.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    if path.starts_with("/p/") && (status.is_client_error() || status.is_server_error()) {
        assert_eq!(headers["cache-control"], "no-store");
    }
    let bytes = to_bytes(response.into_body(), 2 * 1024 * 1024)
        .await
        .unwrap()
        .to_vec();
    (status, headers, bytes)
}
async fn json_call(
    app: &Router,
    method: &str,
    path: &str,
    key: &str,
    body: Value,
    status: StatusCode,
) -> Value {
    let (actual, _, bytes) = call(app, method, path, Some(key), body).await;
    assert_eq!(
        actual,
        status,
        "{method} {path}: {}",
        String::from_utf8_lossy(&bytes)
    );
    if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap()
    }
}
async fn pool() -> PgPool {
    let pool = PgPoolOptions::new()
        .max_connections(3)
        .connect(&std::env::var("TEST_DATABASE_URL").expect("TEST_DATABASE_URL required"))
        .await
        .unwrap();
    MIGRATOR.run(&pool).await.unwrap();
    MIGRATOR.run(&pool).await.unwrap();
    pool
}
async fn shop_key(pool: &PgPool, name: &str) -> (i64, String) {
    let id = sqlx::query_scalar("INSERT INTO shops (name, bank_bin, account) VALUES ($1,'970436','0000000000') RETURNING id").bind(name).fetch_one(pool).await.unwrap();
    let key = create_key(pool).await.unwrap();
    sqlx::query("UPDATE api_keys SET shop_id=$1 WHERE key_hash=$2")
        .bind(id)
        .bind(key_hash(&key))
        .execute(pool)
        .await
        .unwrap();
    (id, key)
}
async fn cleanup(pool: &PgPool, ids: &[i64]) {
    for table in ["api_keys", "links", "templates"] {
        sqlx::query(&format!("DELETE FROM {table} WHERE shop_id=ANY($1)"))
            .bind(ids)
            .execute(pool)
            .await
            .unwrap();
    }
    sqlx::query("DELETE FROM shops WHERE id=ANY($1)")
        .bind(ids)
        .execute(pool)
        .await
        .unwrap();
}
fn decode(bytes: &[u8]) -> String {
    let image = image::load_from_memory(bytes).unwrap().to_luma8();
    let mut prepared = rqrr::PreparedImage::prepare(image);
    let grids = prepared.detect_grids();
    assert_eq!(grids.len(), 1);
    grids[0].decode().unwrap().1
}
fn jpeg_logo() -> String {
    let image = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
        80,
        50,
        image::Rgb([30, 80, 140]),
    ));
    let mut bytes = std::io::Cursor::new(Vec::new());
    image
        .write_to(&mut bytes, image::ImageFormat::Jpeg)
        .unwrap();
    STANDARD.encode(bytes.into_inner())
}

#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL pointing to a disposable Postgres database"]
async fn shops_templates_links_migrations_crud_and_ownership() {
    let pool = pool().await;
    let (a, key_a) = shop_key(&pool, "Shop A").await;
    let (b, key_b) = shop_key(&pool, "Shop B").await;
    let no_shop = create_key(&pool).await.unwrap();
    let app = router(AppState::new(pool.clone()));
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT plan FROM shops WHERE id=$1")
            .bind(a)
            .fetch_one(&pool)
            .await
            .unwrap(),
        "free"
    );
    // Migration constraints reject dangling keys and cross-shop template references even outside the API.
    assert!(
        sqlx::query("UPDATE api_keys SET shop_id=-1 WHERE key_hash=$1")
            .bind(key_hash(&no_shop))
            .execute(&pool)
            .await
            .is_err()
    );
    for path in ["/v1/templates", "/v1/links"] {
        for method in ["GET", "POST"] {
            json_call(
                &app,
                method,
                path,
                &no_shop,
                json!({}),
                StatusCode::FORBIDDEN,
            )
            .await;
        }
    }
    let template = json_call(&app, "POST", "/v1/templates", &key_b, json!({"name":"Brand", "logo":jpeg_logo(), "fg_color":"#123456", "bg_color":"#fffff0", "frame":"border", "show_shop_name":true}), StatusCode::CREATED).await;
    let tid = template["id"].as_i64().unwrap();
    let bytes: Vec<u8> = sqlx::query_scalar("SELECT logo FROM templates WHERE id=$1")
        .bind(tid)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        image::guess_format(&bytes).unwrap(),
        image::ImageFormat::Png
    );
    assert_eq!(
        STANDARD.decode(template["logo"].as_str().unwrap()).unwrap(),
        bytes
    );
    assert!(
        sqlx::query("INSERT INTO links (shop_id, slug, template_id) VALUES ($1,$2,$3)")
            .bind(a)
            .bind(format!("cross-{a}"))
            .bind(tid)
            .execute(&pool)
            .await
            .is_err()
    );
    let tpath = format!("/v1/templates/{tid}");
    for method in ["GET", "PATCH", "DELETE"] {
        json_call(
            &app,
            method,
            &tpath,
            &key_a,
            json!({"name":"stolen"}),
            StatusCode::NOT_FOUND,
        )
        .await;
    }
    assert_eq!(
        json_call(
            &app,
            "GET",
            "/v1/templates",
            &key_a,
            json!(null),
            StatusCode::OK
        )
        .await,
        json!([])
    );
    json_call(
        &app,
        "POST",
        "/v1/qr",
        &key_a,
        json!({"bank_bin":"970436", "account":"0000000000", "format":"png", "template_id":tid}),
        StatusCode::NOT_FOUND,
    )
    .await;
    json_call(
        &app,
        "POST",
        "/v1/qr",
        &no_shop,
        json!({"bank_bin":"970436", "account":"0000000000", "format":"png", "template_id":tid}),
        StatusCode::FORBIDDEN,
    )
    .await;
    json_call(
        &app,
        "POST",
        "/v1/links",
        &key_a,
        json!({"slug":format!("theft-{a}"), "template_id":tid}),
        StatusCode::NOT_FOUND,
    )
    .await;
    for colors in [
        json!({"fg_color":"#ffffff", "bg_color":"#000000"}),
        json!({"fg_color":"#888888"}),
        json!({"fg_color":"red"}),
    ] {
        let mut input = colors;
        input["name"] = json!("bad");
        let response = json_call(
            &app,
            "POST",
            "/v1/templates",
            &key_a,
            input,
            StatusCode::BAD_REQUEST,
        )
        .await;
        assert_eq!(response["code"], "InvalidColors");
    }
    let updated = json_call(
        &app,
        "PATCH",
        &tpath,
        &key_b,
        json!({"name":"New brand"}),
        StatusCode::OK,
    )
    .await;
    assert_eq!(updated["version"], 2);
    assert_eq!(updated["fg_color"], "#123456");
    assert_eq!(updated["frame"], "border");
    let (status, _, png) = call(&app, "POST", "/v1/qr", Some(&key_b), json!({"bank_bin":"970436", "account":"0000000000", "format":"png", "amount":35000, "memo":"Cơm trưa", "template_id":tid})).await;
    assert_eq!(status, StatusCode::OK);
    let payload = decode(&png);
    assert_eq!(vietqr_core::parse(&payload).unwrap().memo, Some("Com trua"));
    for slug in ["ab", "a/b", "ABC", "a b", &"a".repeat(41)] {
        json_call(
            &app,
            "POST",
            "/v1/links",
            &key_a,
            json!({"slug":slug}),
            StatusCode::BAD_REQUEST,
        )
        .await;
    }
    for amount in [0, vietqr_core::MAX_AMOUNT + 1] {
        json_call(
            &app,
            "POST",
            "/v1/links",
            &key_a,
            json!({"slug":format!("amount-{a}"), "default_amount":amount}),
            StatusCode::BAD_REQUEST,
        )
        .await;
    }
    let link = json_call(&app, "POST", "/v1/links", &key_b, json!({"slug":format!("shop-b-{b}"), "template_id":tid, "default_amount":35000, "default_memo":"Cơm trưa"}), StatusCode::CREATED).await;
    assert_eq!(link["default_memo"], "Com trua");
    let lpath = format!("/v1/links/{}", link["id"]);
    for method in ["GET", "PATCH", "DELETE"] {
        json_call(
            &app,
            method,
            &lpath,
            &key_a,
            json!({"active":false}),
            StatusCode::NOT_FOUND,
        )
        .await;
    }
    assert_eq!(
        json_call(
            &app,
            "GET",
            "/v1/links",
            &key_a,
            json!(null),
            StatusCode::OK
        )
        .await,
        json!([])
    );
    json_call(
        &app,
        "POST",
        "/v1/links",
        &key_a,
        json!({"slug":format!("shop-b-{b}")}),
        StatusCode::CONFLICT,
    )
    .await;
    for slug in [
        json!(format!("renamed-{b}")),
        link["slug"].clone(),
        json!(null),
    ] {
        json_call(
            &app,
            "PATCH",
            &lpath,
            &key_b,
            json!({"slug":slug}),
            StatusCode::BAD_REQUEST,
        )
        .await;
    }
    assert_eq!(
        json_call(&app, "GET", &lpath, &key_b, json!(null), StatusCode::OK).await["slug"],
        link["slug"]
    );
    json_call(
        &app,
        "DELETE",
        &tpath,
        &key_b,
        json!(null),
        StatusCode::CONFLICT,
    )
    .await;
    let updated = json_call(
        &app,
        "PATCH",
        &lpath,
        &key_b,
        json!({"template_id":null, "default_amount":null, "default_memo":null}),
        StatusCode::OK,
    )
    .await;
    assert!(
        updated["template_id"].is_null()
            && updated["default_amount"].is_null()
            && updated["default_memo"].is_null()
    );
    json_call(
        &app,
        "DELETE",
        &tpath,
        &key_b,
        json!(null),
        StatusCode::NO_CONTENT,
    )
    .await;
    json_call(
        &app,
        "GET",
        &tpath,
        &key_b,
        json!(null),
        StatusCode::NOT_FOUND,
    )
    .await;
    json_call(
        &app,
        "DELETE",
        &lpath,
        &key_b,
        json!(null),
        StatusCode::NO_CONTENT,
    )
    .await;
    let row = json_call(&app, "GET", &lpath, &key_b, json!(null), StatusCode::OK).await;
    assert_eq!(row["active"], false);
    assert_eq!(
        json_call(
            &app,
            "GET",
            "/v1/links",
            &key_b,
            json!(null),
            StatusCode::OK
        )
        .await
        .as_array()
        .unwrap()
        .len(),
        1
    );
    sqlx::query("DELETE FROM api_keys WHERE key_hash=$1")
        .bind(key_hash(&no_shop))
        .execute(&pool)
        .await
        .unwrap();
    cleanup(&pool, &[a, b]).await;
}

#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL pointing to a disposable Postgres database"]
async fn payment_page_escaping_overrides_cache_watermarks_and_decoding() {
    let pool = pool().await;
    let (id, key) = shop_key(&pool, "Quán <script>alert(1)</script> & \"Test\"").await;
    let app = router(AppState::new(pool.clone()));
    let slug = format!("page-{id}");
    let link = json_call(
        &app,
        "POST",
        "/v1/links",
        &key,
        json!({"slug":slug}),
        StatusCode::CREATED,
    )
    .await;
    let path = format!("/p/{slug}");
    let (_, _, blank) = call(&app, "GET", &path, None, json!(null)).await;
    let blank = String::from_utf8(blank).unwrap();
    let blank_image = blank
        .split("<img src=\"")
        .nth(1)
        .unwrap()
        .split('"')
        .next()
        .unwrap();
    let (status, _, blank_svg) = call(&app, "GET", blank_image, None, json!(null)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(blank_svg.starts_with(b"<svg"));
    let query = "?amount=35000&memo=C%C6%A1m%20tr%C6%B0a";
    let (status, headers, html) =
        call(&app, "GET", &format!("{path}{query}"), None, json!(null)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["cache-control"], "public, max-age=60");
    let csp = headers["content-security-policy"].to_str().unwrap();
    assert!(
        csp.contains("default-src 'none'")
            && csp.contains("script-src 'sha256-")
            && !csp.contains("unsafe-inline")
    );
    let html = String::from_utf8(html).unwrap();
    assert!(html.contains("Quán &lt;script&gt;alert(1)&lt;/script&gt; &amp; &quot;Test&quot;"));
    assert!(html.contains("Com trua"));
    assert!(!html.contains("<script>alert(1)</script>"));
    assert!(html.contains("Tạo bởi VietQR") && html.contains("Vietcombank / 970436"));
    assert!(html.contains("role=\"status\"") && html.contains("data-copy=\"account\""));
    let image_url = html
        .split("<img src=\"")
        .nth(1)
        .unwrap()
        .split('"')
        .next()
        .unwrap()
        .replace("&amp;", "&");
    let png_url = image_url.replace("qr.svg", "qr.png");
    let (status, png_headers, png) = call(&app, "GET", &png_url, None, json!(null)).await;
    assert_eq!(status, StatusCode::OK);
    let payload = decode(&png);
    let expected = vietqr_core::generate(&vietqr_core::Transfer {
        bank_bin: "970436",
        account: "0000000000",
        service: vietqr_core::Service::Account,
        name: None,
        amount: Some(35000),
        memo: Some("Com trua"),
    })
    .unwrap();
    assert_eq!(payload, expected);
    let free_image = image::load_from_memory(&png).unwrap();
    assert_eq!(free_image.height(), free_image.width() + 36);
    let free_etag = png_headers["etag"].to_str().unwrap();
    let response = app
        .clone()
        .oneshot(
            Request::get(&png_url)
                .extension(axum::extract::ConnectInfo(
                    "127.0.0.1:1234".parse::<std::net::SocketAddr>().unwrap(),
                ))
                .header("if-none-match", free_etag)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_MODIFIED);
    assert_eq!(response.headers()["cache-control"], "public, max-age=3600");
    assert!(
        to_bytes(response.into_body(), 1024)
            .await
            .unwrap()
            .is_empty()
    );
    for suffix in ["", "/qr.png", "/qr.svg"] {
        for query in [
            "?amount=0",
            "?amount=-1",
            "?amount=1.5",
            "?amount=10000000000000",
            "?memo=%F0%9F%98%80",
            "?memo=%3Cscript%3E%26%22",
        ] {
            assert_eq!(
                call(
                    &app,
                    "GET",
                    &format!("{path}{suffix}{query}"),
                    None,
                    json!(null)
                )
                .await
                .0,
                StatusCode::BAD_REQUEST
            );
        }
    }
    sqlx::query("UPDATE shops SET plan='shop' WHERE id=$1")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    let (_, _, paid_html) = call(&app, "GET", &format!("{path}{query}"), None, json!(null)).await;
    assert!(
        !String::from_utf8(paid_html)
            .unwrap()
            .contains("Tạo bởi VietQR")
    );
    let (_, paid_headers, paid_png) = call(&app, "GET", &png_url, None, json!(null)).await;
    let paid_image = image::load_from_memory(&paid_png).unwrap();
    assert_eq!(paid_image.height(), paid_image.width());
    assert_ne!(paid_headers["etag"], png_headers["etag"]);
    assert_eq!(decode(&paid_png), expected);
    let template = json_call(&app, "POST", "/v1/templates", &key, json!({"name":"Page brand", "fg_color":"#123456", "frame":"border", "show_shop_name":true, "logo":jpeg_logo()}), StatusCode::CREATED).await;
    let lpath = format!("/v1/links/{}", link["id"]);
    json_call(
        &app,
        "PATCH",
        &lpath,
        &key,
        json!({"default_amount":35000, "default_memo":"Cơm trưa", "template_id":template["id"]}),
        StatusCode::OK,
    )
    .await;
    let (_, _, fixed_png) = call(
        &app,
        "GET",
        &format!("{path}/qr.png?amount=bad&memo=%F0%9F%98%80"),
        None,
        json!(null),
    )
    .await;
    let fixed_payload = decode(&fixed_png);
    let parsed = vietqr_core::parse(&fixed_payload).unwrap();
    assert_eq!(parsed.amount, Some(35000));
    assert_eq!(parsed.memo, Some("Com trua"));
    let fixed_image = image::load_from_memory(&fixed_png).unwrap();
    assert_eq!(fixed_image.height(), fixed_image.width() + 36); // Shop label, no paid watermark.
    let (_, _, fixed_html) = call(
        &app,
        "GET",
        &format!("{path}?amount=bad&memo=%F0%9F%98%80"),
        None,
        json!(null),
    )
    .await;
    let fixed_html = String::from_utf8(fixed_html).unwrap();
    assert!(fixed_html.contains("35000") && fixed_html.contains("Com trua"));
    let (_, old_headers, _) = call(&app, "GET", &format!("{path}/qr.svg"), None, json!(null)).await;
    json_call(
        &app,
        "PATCH",
        &format!("/v1/templates/{}", template["id"]),
        &key,
        json!({"name":"Version 2"}),
        StatusCode::OK,
    )
    .await;
    let (_, new_headers, svg) =
        call(&app, "GET", &format!("{path}/qr.svg"), None, json!(null)).await;
    assert_ne!(old_headers["etag"], new_headers["etag"]);
    assert!(
        String::from_utf8(svg)
            .unwrap()
            .contains("data:image/png;base64,")
    );
    json_call(
        &app,
        "DELETE",
        &lpath,
        &key,
        json!(null),
        StatusCode::NO_CONTENT,
    )
    .await;
    for path in [
        &path,
        &format!("{path}/qr.png"),
        &format!("{path}/qr.svg"),
        "/p/missing-link",
    ] {
        assert_eq!(
            call(&app, "GET", path, None, json!(null)).await.0,
            StatusCode::NOT_FOUND
        );
    }
    cleanup(&pool, &[id]).await;
}
