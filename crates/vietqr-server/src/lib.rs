mod payment;
mod render;
mod shops;

use std::{
    collections::HashMap,
    hash::Hash,
    net::{IpAddr, SocketAddr},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use axum::{
    Extension, Json, Router,
    extract::{ConnectInfo, DefaultBodyLimit, Request, State, rejection::JsonRejection},
    http::{HeaderValue, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use tokio::sync::Semaphore;
use vietqr_core::{Profile, Service, Transfer};

pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

#[derive(Clone)]
pub struct AppState {
    pool: PgPool,
    limiter: Arc<Mutex<RateLimiter<i64>>>,
    ip_limiter: Arc<Mutex<RateLimiter<IpAddr>>>,
    requests: Arc<Semaphore>,
    public_requests: Arc<Semaphore>,
    render_slots: Arc<Semaphore>,
    /// Only these peers may relay CF-Connecting-IP; block traffic bypassing Cloudflare.
    pub trusted_proxy_cidrs: Vec<ipnet::IpNet>,
}

impl AppState {
    pub fn new(pool: PgPool) -> Self {
        Self {
            pool,
            limiter: Arc::new(Mutex::new(RateLimiter::default())),
            ip_limiter: Arc::new(Mutex::new(RateLimiter::default())),
            requests: Arc::new(Semaphore::new(64)),
            public_requests: Arc::new(Semaphore::new(16)),
            render_slots: Arc::new(Semaphore::new(
                std::thread::available_parallelism().map_or(1, usize::from),
            )),
            trusted_proxy_cidrs: Vec::new(),
        }
    }

    async fn render_job<T: Send + 'static>(
        &self,
        work: impl FnOnce() -> Result<T, ApiError> + Send + 'static,
    ) -> Result<T, ApiError> {
        let permit = self
            .render_slots
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| ApiError::internal())?;
        tokio::task::spawn_blocking(move || {
            // Keep the permit inside the blocking task even if its HTTP request is cancelled.
            let _permit = permit;
            work()
        })
        .await
        .map_err(|_| ApiError::internal())?
    }
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/v1/qr", post(qr))
        .route(
            "/v1/templates",
            post(shops::create_template).get(shops::list_templates),
        )
        .route(
            "/v1/templates/{id}",
            get(shops::get_template)
                .patch(shops::update_template)
                .delete(shops::delete_template),
        )
        .route("/v1/links", post(shops::create_link).get(shops::list_links))
        .route(
            "/v1/links/{id}",
            get(shops::get_link)
                .patch(shops::update_link)
                .delete(shops::deactivate_link),
        )
        .route_layer(middleware::from_fn_with_state(state.clone(), authenticate))
        .layer(DefaultBodyLimit::max(BODY_LIMIT))
        .route("/p/{slug}", get(payment::page))
        .route("/p/{slug}/{image}", get(payment::qr_image))
        .layer(middleware::from_fn_with_state(state.clone(), guard_request))
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

const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const BODY_TIMEOUT: Duration = Duration::from_secs(5);
const BODY_LIMIT: usize = 360 * 1024;

async fn guard_request(State(state): State<AppState>, request: Request, next: Next) -> Response {
    let result = tokio::time::timeout(REQUEST_TIMEOUT, guarded_request(state, request, next)).await;
    let mut response = result
        .unwrap_or_else(|_| Err(request_timeout()))
        .unwrap_or_else(IntoResponse::into_response);
    if response.status().is_client_error() || response.status().is_server_error() {
        response
            .headers_mut()
            .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    }
    response
}

fn request_timeout() -> ApiError {
    ApiError::new(
        StatusCode::REQUEST_TIMEOUT,
        "RequestTimeout",
        "Request timed out",
    )
}

async fn guarded_request(
    state: AppState,
    request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    // Public payment traffic cannot be starved by slow API uploads.
    let requests = if request.uri().path().starts_with("/p/") {
        &state.public_requests
    } else {
        &state.requests
    };
    let _permit = requests.clone().try_acquire_owned().map_err(|_| {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "Busy",
            "Too many concurrent requests",
        )
    })?;
    let peer = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .ok_or_else(ApiError::internal)?
        .0
        .ip();
    let ip = if state
        .trusted_proxy_cidrs
        .iter()
        .any(|cidr| cidr.contains(&peer))
    {
        request
            .headers()
            .get("cf-connecting-ip")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse().ok())
            .unwrap_or(peer)
    } else {
        peer
    };
    if !state
        .ip_limiter
        .lock()
        .map_err(|_| ApiError::internal())?
        .allow(ip_bucket(ip), Instant::now(), 120)
    {
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "RateLimited",
            "120 requests per minute per IP (IPv6 /64)",
        ));
    }
    let (parts, body) = request.into_parts();
    let bytes = tokio::time::timeout(BODY_TIMEOUT, axum::body::to_bytes(body, BODY_LIMIT))
        .await
        .map_err(|_| request_timeout())?
        .map_err(|_| ApiError::bad_request("InvalidRequest", "Invalid body or body size"))?;
    Ok(next
        .run(Request::from_parts(parts, axum::body::Body::from(bytes)))
        .await)
}

fn ip_bucket(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(ip) => match ip.to_ipv4_mapped() {
            Some(ip) => IpAddr::V4(ip),
            None => IpAddr::V6((u128::from(ip) & (u128::MAX << 64)).into()),
        },
        ip => ip,
    }
}

fn bearer_token(value: &str) -> Option<&str> {
    let (scheme, token) = value.split_once(' ')?;
    let token = token.trim_start_matches(' ');
    (scheme.eq_ignore_ascii_case("bearer")
        && token.len() == 68
        && token.starts_with("vqr_")
        && token.as_bytes()[4..].iter().all(u8::is_ascii_hexdigit))
    .then_some(token)
}

async fn authenticate(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let key = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(bearer_token)
        .ok_or_else(ApiError::unauthorized)?;
    let identity: Option<(i64, Option<i64>)> = sqlx::query_as(
        "SELECT id, shop_id FROM api_keys WHERE key_hash = $1 AND revoked_at IS NULL",
    )
    .bind(key_hash(key))
    .fetch_optional(&state.pool)
    .await
    .map_err(|error| {
        eprintln!("API key lookup failed: {error}");
        ApiError::internal()
    })?;
    let (id, shop_id) = identity.ok_or_else(ApiError::unauthorized)?;
    request.extensions_mut().insert(shops::KeyShop(shop_id));
    if !state
        .limiter
        .lock()
        .map_err(|_| ApiError::internal())?
        .allow(id, Instant::now(), 60)
    {
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "RateLimited",
            "60 requests per minute",
        ));
    }
    Ok(next.run(request).await)
}

struct RateLimiter<K> {
    windows: HashMap<K, (Instant, u32)>,
    last_cleanup: Instant,
}

impl<K> Default for RateLimiter<K> {
    fn default() -> Self {
        Self {
            windows: HashMap::new(),
            last_cleanup: Instant::now(),
        }
    }
}

impl<K: Eq + Hash> RateLimiter<K> {
    fn allow(&mut self, id: K, now: Instant, limit: u32) -> bool {
        let window = Duration::from_secs(60);
        if now.duration_since(self.last_cleanup) >= window {
            self.windows
                .retain(|_, (start, _)| now.duration_since(*start) < window);
            self.last_cleanup = now;
        }
        let (start, count) = self.windows.entry(id).or_insert((now, 0));
        if now.duration_since(*start) >= window {
            *start = now;
            *count = 0;
        }
        if *count >= limit {
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
    template_id: Option<i64>,
}

async fn qr(
    State(state): State<AppState>,
    Extension(key): Extension<shops::KeyShop>,
    input: Result<Json<QrRequest>, JsonRejection>,
) -> Result<Response, ApiError> {
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
    let style = if let Some(id) = input.template_id {
        if input.logo.is_some() {
            return Err(ApiError::bad_request(
                "InvalidRequest",
                "logo and template_id are mutually exclusive",
            ));
        }
        let template = shops::owned_template(&state.pool, key.required()?, id).await?;
        let shop = shops::shop(&state.pool, key.required()?).await?;
        template.style(&shop)?
    } else {
        render::Style::default()
    };
    let bytes = state
        .render_job(move || {
            let mut style = style;
            if let Some(logo) = input.logo {
                style.logo = Some(render::normalize_logo(&logo)?);
            }
            render::render(&payload, input.format, &style)
        })
        .await?;
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

impl From<sqlx::Error> for ApiError {
    fn from(error: sqlx::Error) -> Self {
        match &error {
            sqlx::Error::RowNotFound => Self::new(StatusCode::NOT_FOUND, "NotFound", "Not found"),
            sqlx::Error::Database(db)
                if db.is_unique_violation() || db.is_foreign_key_violation() =>
            {
                Self::new(
                    StatusCode::CONFLICT,
                    "Conflict",
                    "Slug already exists or resource is referenced",
                )
            }
            _ => {
                eprintln!("Database error: {error}");
                Self::internal()
            }
        }
    }
}
#[derive(Debug)]
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
