mod render;

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Request, State, rejection::JsonRejection},
    http::{HeaderValue, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use vietqr_core::{Profile, Service, Transfer};

pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

#[derive(Clone)]
pub struct AppState {
    pool: PgPool,
    limiter: Arc<Mutex<RateLimiter>>,
}

impl AppState {
    pub fn new(pool: PgPool) -> Self {
        Self {
            pool,
            limiter: Arc::new(Mutex::new(RateLimiter::default())),
        }
    }
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/v1/qr", post(qr))
        .route_layer(middleware::from_fn_with_state(state.clone(), authenticate))
        .layer(DefaultBodyLimit::max(360 * 1024))
        .route("/healthz", get(|| async { "ok\n" }))
        .with_state(state)
}

pub fn key_hash(key: &str) -> String {
    format!("{:x}", Sha256::digest(key.as_bytes()))
}

/// Returns the secret once; only its SHA-256 hash and display prefix are persisted.
pub async fn create_key(pool: &PgPool) -> Result<String, Box<dyn std::error::Error>> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes)?;
    let key = format!(
        "vqr_{}",
        bytes.iter().map(|b| format!("{b:02x}")).collect::<String>()
    );
    sqlx::query("INSERT INTO api_keys (key_hash, prefix) VALUES ($1, $2)")
        .bind(key_hash(&key))
        .bind(&key[..12])
        .execute(pool)
        .await?;
    Ok(key)
}

async fn authenticate(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let key = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .filter(|key| !key.is_empty() && key.len() <= 256)
        .ok_or_else(ApiError::unauthorized)?;
    let id: Option<i64> =
        sqlx::query_scalar("SELECT id FROM api_keys WHERE key_hash = $1 AND revoked_at IS NULL")
            .bind(key_hash(key))
            .fetch_optional(&state.pool)
            .await
            .map_err(|error| {
                eprintln!("API key lookup failed: {error}");
                ApiError::internal()
            })?;
    let id = id.ok_or_else(ApiError::unauthorized)?;
    if !state
        .limiter
        .lock()
        .map_err(|_| ApiError::internal())?
        .allow(id, Instant::now())
    {
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "RateLimited",
            "60 requests per minute",
        ));
    }
    Ok(next.run(request).await)
}

#[derive(Default)]
struct RateLimiter {
    windows: HashMap<i64, (Instant, u32)>,
}

impl RateLimiter {
    fn allow(&mut self, id: i64, now: Instant) -> bool {
        let window = Duration::from_secs(60);
        self.windows
            .retain(|_, (start, _)| now.duration_since(*start) < window);
        let (_, count) = self.windows.entry(id).or_insert((now, 0));
        if *count >= 60 {
            return false;
        }
        *count += 1;
        true
    }
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "lowercase")]
enum RequestService {
    #[default]
    Account,
    Card,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "lowercase")]
enum RequestProfile {
    #[default]
    Npm,
    Strict,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Format {
    Png,
    Svg,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct QrRequest {
    bank_bin: String,
    account: String,
    #[serde(default)]
    service: RequestService,
    name: Option<String>,
    amount: Option<u64>,
    memo: Option<String>,
    #[serde(default)]
    profile: RequestProfile,
    format: Format,
    logo: Option<String>,
}

async fn qr(input: Result<Json<QrRequest>, JsonRejection>) -> Result<Response, ApiError> {
    let Json(input) = input.map_err(|_| {
        ApiError::bad_request(
            "InvalidRequest",
            "Invalid JSON, field type, format or body size",
        )
    })?;
    let payload = vietqr_core::generate_with_profile(
        &Transfer {
            bank_bin: &input.bank_bin,
            account: &input.account,
            service: match input.service {
                RequestService::Account => Service::Account,
                RequestService::Card => Service::Card,
            },
            name: input.name.as_deref(),
            amount: input.amount,
            memo: input.memo.as_deref(),
        },
        match input.profile {
            RequestProfile::Npm => Profile::Npm,
            RequestProfile::Strict => Profile::Strict,
        },
    )
    .map_err(ApiError::from)?;
    let payload_header = HeaderValue::from_str(&payload).map_err(|_| ApiError::internal())?;
    let bytes = tokio::task::spawn_blocking(move || {
        render::render(&payload, input.format, input.logo.as_deref())
    })
    .await
    .map_err(|_| ApiError::internal())??;
    let content_type = match input.format {
        Format::Png => "image/png",
        Format::Svg => "image/svg+xml",
    };
    Ok((
        [
            (header::CONTENT_TYPE, HeaderValue::from_static(content_type)),
            (header::CACHE_CONTROL, HeaderValue::from_static("no-store")),
            (
                header::HeaderName::from_static("x-vietqr-payload"),
                payload_header,
            ),
        ],
        bytes,
    )
        .into_response())
}

struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: String,
}

impl ApiError {
    fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
        }
    }

    fn bad_request(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, code, message)
    }

    fn unauthorized() -> Self {
        Self::new(
            StatusCode::UNAUTHORIZED,
            "Unauthorized",
            "Missing, invalid or revoked API key",
        )
    }

    fn internal() -> Self {
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "InternalError",
            "Internal server error",
        )
    }
}

impl From<vietqr_core::Error> for ApiError {
    fn from(error: vietqr_core::Error) -> Self {
        use vietqr_core::Error::*;
        let code = match error {
            InvalidTag(_) => "InvalidTag",
            InvalidLength { .. } => "InvalidLength",
            MalformedTlv { .. } => "MalformedTlv",
            DuplicateTag(_) => "DuplicateTag",
            MissingTag(_) => "MissingTag",
            InvalidValue { .. } => "InvalidValue",
            InvalidBin => "InvalidBin",
            InvalidAccount => "InvalidAccount",
            InvalidAmount => "InvalidAmount",
            InvalidMemo => "InvalidMemo",
            InvalidName => "InvalidName",
            PayloadTooLong => "PayloadTooLong",
            InvalidCrc => "InvalidCrc",
        };
        Self::bad_request(code, error.to_string())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        #[derive(Serialize)]
        struct Body<'a> {
            code: &'a str,
            message: &'a str,
        }
        let mut response = (
            self.status,
            Json(Body {
                code: self.code,
                message: &self.message,
            }),
        )
            .into_response();
        if self.status == StatusCode::UNAUTHORIZED {
            response
                .headers_mut()
                .insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
        }
        if self.status == StatusCode::TOO_MANY_REQUESTS {
            response
                .headers_mut()
                .insert(header::RETRY_AFTER, HeaderValue::from_static("60"));
        }
        response
    }
}

#[cfg(test)]
mod tests;
