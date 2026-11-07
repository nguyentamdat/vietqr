use axum::{
    extract::{Path, Query, State, rejection::QueryRejection},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{Html, IntoResponse, Response},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::{
    ApiError, AppState, Format, render,
    shops::{self, Link, Shop},
};

#[derive(Default, Deserialize)]
pub(super) struct Overrides {
    amount: Option<String>,
    memo: Option<String>,
}

struct Payment {
    link: Link,
    shop: Shop,
    payload: String,
    style: render::Style,
    version: i32,
}

async fn load(state: &AppState, slug: &str, query: Overrides) -> Result<Payment, ApiError> {
    let link: Link = sqlx::query_as("SELECT * FROM links WHERE slug=$1 AND active=true")
        .bind(slug)
        .fetch_one(&state.pool)
        .await?;
    let shop = shops::shop(&state.pool, link.shop_id).await?;
    let amount = match link.default_amount {
        Some(amount) => Some(amount as u64),
        None => query
            .amount
            .map(|value| {
                value
                    .parse::<u64>()
                    .map_err(|_| ApiError::from(vietqr_core::Error::InvalidAmount))
            })
            .transpose()?,
    };
    let memo = link.default_memo.as_deref().or(query.memo.as_deref());
    let payload = shops::payment_payload(&shop, amount, memo)?;
    let (style, version) = if let Some(id) = link.template_id {
        let template = shops::owned_template(&state.pool, link.shop_id, id).await?;
        (template.style(&shop)?, template.version)
    } else {
        (
            render::Style {
                watermark: shop.plan == "free",
                ..Default::default()
            },
            0,
        )
    };
    Ok(Payment {
        link,
        shop,
        payload,
        style,
        version,
    })
}

fn overrides(query: Result<Query<Overrides>, QueryRejection>) -> Result<Overrides, ApiError> {
    query
        .map(|Query(q)| q)
        .map_err(|_| ApiError::bad_request("InvalidRequest", "Invalid query parameters"))
}

pub(super) fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn url_component(value: &str) -> String {
    value
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

const STYLE: &str = "*{box-sizing:border-box}body{font:1rem/1.5 system-ui,sans-serif;margin:0;padding:1rem;background:#f4f6f8;color:#17202a}main{max-width:30rem;margin:auto;background:white;padding:1.25rem;border-radius:1rem}h1{font-size:1.5rem;overflow-wrap:anywhere}img{display:block;width:100%;height:auto;max-width:26rem;margin:auto}dl{margin:1rem 0}dt{font-weight:600}dd{margin:0 0 1rem;overflow-wrap:anywhere}button{font:inherit;padding:.65rem 1rem;min-height:44px;cursor:pointer;border:1px solid #506070;border-radius:.4rem;background:white;color:#17202a}button:focus-visible{outline:3px solid #005fcc;outline-offset:2px}small{display:block;text-align:center;color:#46515c}#feedback{min-height:1.5em}";
const SCRIPT: &str = "document.querySelectorAll('button[data-copy]').forEach(b=>b.addEventListener('click',async()=>{const s=document.getElementById('feedback');try{await navigator.clipboard.writeText(document.getElementById(b.dataset.copy).textContent);s.textContent='Đã sao chép';}catch{s.textContent='Không thể sao chép. Vui lòng chọn và sao chép thủ công.';}}));";

fn csp() -> String {
    format!(
        "default-src 'none'; img-src 'self' data:; style-src 'sha256-{}'; script-src 'sha256-{}'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'",
        STANDARD.encode(Sha256::digest(STYLE.as_bytes())),
        STANDARD.encode(Sha256::digest(SCRIPT.as_bytes()))
    )
}
fn security(response: &mut Response) {
    response.headers_mut().insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_str(&csp()).expect("static CSP"),
    );
    response.headers_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    response.headers_mut().insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
}
fn field(label: &str, id: &str, value: &str, copy: bool) -> String {
    let button = if copy {
        format!(
            " <button type=\"button\" data-copy=\"{id}\" aria-label=\"Sao chép {label}\">Sao chép</button>"
        )
    } else {
        String::new()
    };
    format!(
        "<dt>{label}</dt><dd><span id=\"{id}\">{}</span>{button}</dd>",
        escape(value)
    )
}

pub(super) async fn page(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    query: Result<Query<Overrides>, QueryRejection>,
) -> Result<Response, ApiError> {
    let payment = load(&state, &slug, overrides(query)?).await?;
    let parsed = vietqr_core::parse(&payment.payload)?;
    let amount = parsed.amount.map(|v| v.to_string()).unwrap_or_default();
    let memo = parsed.memo.unwrap_or_default();
    let mut params = Vec::new();
    if !amount.is_empty() {
        params.push(format!("amount={amount}"));
    }
    if !memo.is_empty() {
        params.push(format!("memo={}", url_component(memo)));
    }
    let image_url = format!("/p/{}/qr.svg?{}", payment.link.slug, params.join("&"));
    let bank = vietqr_core::banks::by_bin(&payment.shop.bank_bin)
        .map(|b| b.name)
        .unwrap_or("Ngân hàng");
    let mut fields = field(
        "Ngân hàng / BIN",
        "bank",
        &format!("{bank} / {}", payment.shop.bank_bin),
        false,
    );
    if let Some(name) = &payment.shop.account_name {
        fields.push_str(&field("Chủ tài khoản", "owner", name, false));
    }
    fields.push_str(&field(
        "Số tài khoản",
        "account",
        &payment.shop.account,
        true,
    ));
    fields.push_str(&field(
        "Số tiền (VND)",
        "amount",
        &amount,
        !amount.is_empty(),
    ));
    fields.push_str(&field("Nội dung", "memo", memo, !memo.is_empty()));
    let watermark = if payment.shop.plan == "free" {
        format!("<small>{}</small>", render::WATERMARK)
    } else {
        String::new()
    };
    let name = escape(&payment.shop.name);
    let html = format!(
        "<!doctype html><html lang=\"vi\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>Thanh toán — {name}</title><style>{STYLE}</style></head><body><main><h1>{name}</h1><p>Quét mã QR để chuyển khoản. Kiểm tra thông tin người nhận trong ứng dụng ngân hàng trước khi xác nhận.</p><img src=\"{}\" alt=\"Mã QR chuyển khoản\"><dl>{fields}</dl><p id=\"feedback\" role=\"status\" aria-live=\"polite\"></p>{watermark}<p>Trang này không xác nhận giao dịch đã thanh toán.</p></main><script>{SCRIPT}</script></body></html>",
        escape(&image_url)
    );
    let mut response = Html(html).into_response();
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=60"),
    );
    security(&mut response);
    Ok(response)
}

pub(super) async fn qr_image(
    State(state): State<AppState>,
    Path((slug, image)): Path<(String, String)>,
    query: Result<Query<Overrides>, QueryRejection>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let (format, content_type) = match image.as_str() {
        "qr.png" => (Format::Png, "image/png"),
        "qr.svg" => (Format::Svg, "image/svg+xml"),
        _ => return Err(sqlx::Error::RowNotFound.into()),
    };
    let payment = load(&state, &slug, overrides(query)?).await?;
    let (version, payload, style) = (payment.version, payment.payload, payment.style);
    let bytes = state
        .render_job(move || render::render(&payload, format, &style))
        .await?;
    // Hash the rendered representation too: plan/shop edits and format must invalidate cached images.
    let mut hash = Sha256::new();
    hash.update(version.to_be_bytes());
    hash.update(&bytes);
    let etag = format!("\"{:x}\"", hash.finalize());
    let unchanged = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| {
            v.split(',')
                .any(|item| item.trim().trim_start_matches("W/") == etag || item.trim() == "*")
        });
    let mut response = if unchanged {
        StatusCode::NOT_MODIFIED.into_response()
    } else {
        ([(header::CONTENT_TYPE, content_type)], bytes).into_response()
    };
    response.headers_mut().insert(
        header::ETAG,
        HeaderValue::from_str(&etag).expect("hex ETag"),
    );
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=3600"),
    );
    security(&mut response);
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn escapes_html_and_url_components() {
        assert_eq!(
            escape("<script a=\"'&\">"),
            "&lt;script a=&quot;&#39;&amp;&quot;&gt;"
        );
        assert_eq!(url_component("A&B <"), "A%26B%20%3C");
    }
}
