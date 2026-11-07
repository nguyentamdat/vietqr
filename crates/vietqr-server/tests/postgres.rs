use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use serde_json::{Value, json};
use sqlx::postgres::PgPoolOptions;
use tower::ServiceExt;
use vietqr_server::{AppState, MIGRATOR, create_key, key_hash, router};

fn request(key: &str, body: Value) -> Request<Body> {
    Request::post("/v1/qr")
        .extension(axum::extract::ConnectInfo(
            "127.0.0.1:1234".parse::<std::net::SocketAddr>().unwrap(),
        ))
        .header("authorization", format!("bEaReR {key}"))
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL pointing to a disposable Postgres database"]
async fn postgres_auth_revocation_rate_limit_and_key_storage() {
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&std::env::var("TEST_DATABASE_URL").expect("TEST_DATABASE_URL is required"))
        .await
        .unwrap();
    MIGRATOR.run(&pool).await.unwrap();
    MIGRATOR.run(&pool).await.unwrap();
    let key = create_key(&pool).await.unwrap();
    let other_key = create_key(&pool).await.unwrap();
    assert_eq!(key.len(), 68);
    assert_ne!(key, other_key);
    let (hash, prefix, no_shop): (String, String, bool) = sqlx::query_as(
        "SELECT key_hash, prefix, shop_id IS NULL FROM api_keys WHERE key_hash = $1",
    )
    .bind(key_hash(&key))
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(hash, key_hash(&key));
    assert_ne!(hash, key);
    assert_eq!(prefix, key[..12]);
    assert!(no_shop);
    let app = router(AppState::new(pool.clone()));
    let response = app
        .clone()
        .oneshot(request("nonexistent", json!({})))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let response = app
        .clone()
        .oneshot(request(
            &key,
            json!({
                "bank_bin": "970436", "account": "0000000000", "format": "png"
            }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let payload = response.headers()["x-vietqr-payload"]
        .to_str()
        .unwrap()
        .to_owned();
    assert_eq!(vietqr_core::parse(&payload).unwrap().account, "0000000000");
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
    for _ in 1..60 {
        assert_eq!(
            app.clone()
                .oneshot(request(&key, json!({})))
                .await
                .unwrap()
                .status(),
            StatusCode::BAD_REQUEST
        );
    }
    let response = app.clone().oneshot(request(&key, json!({}))).await.unwrap();
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(response.headers()["retry-after"], "60");
    assert_eq!(
        app.clone()
            .oneshot(request(&other_key, json!({})))
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );
    sqlx::query("UPDATE api_keys SET revoked_at = now() WHERE key_hash = $1")
        .bind(key_hash(&key))
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        app.oneshot(request(&key, json!({})))
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    sqlx::query("DELETE FROM api_keys WHERE key_hash IN ($1, $2)")
        .bind(key_hash(&key))
        .bind(key_hash(&other_key))
        .execute(&pool)
        .await
        .unwrap();
}
