use axum::{
    Extension, Json,
    extract::{Path, State, rejection::JsonRejection},
    http::StatusCode,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use sqlx::PgPool;

use crate::{ApiError, AppState, render};

#[derive(Clone, Copy)]
pub(super) struct KeyShop(pub Option<i64>);
impl KeyShop {
    pub fn required(self) -> Result<i64, ApiError> {
        self.0.ok_or_else(|| {
            ApiError::new(
                StatusCode::FORBIDDEN,
                "ShopRequired",
                "API key must belong to a shop",
            )
        })
    }
}

#[derive(sqlx::FromRow)]
pub(super) struct Shop {
    pub name: String,
    pub bank_bin: String,
    pub account: String,
    pub account_name: Option<String>,
    pub plan: String,
}

pub(super) async fn shop(pool: &PgPool, id: i64) -> Result<Shop, ApiError> {
    Ok(sqlx::query_as(
        "SELECT name, bank_bin, account, account_name, plan FROM shops WHERE id = $1",
    )
    .bind(id)
    .fetch_one(pool)
    .await?)
}

#[derive(sqlx::FromRow)]
pub(super) struct Template {
    pub id: i64,
    pub shop_id: i64,
    pub name: String,
    pub logo: Option<Vec<u8>>,
    pub fg_color: String,
    pub bg_color: String,
    pub frame: String,
    pub show_shop_name: bool,
    pub version: i32,
}
impl Template {
    pub fn style(&self, shop: &Shop) -> Result<render::Style, ApiError> {
        Ok(render::Style {
            logo: self.logo.clone(),
            foreground: render::color(&self.fg_color).ok_or_else(ApiError::internal)?,
            background: render::color(&self.bg_color).ok_or_else(ApiError::internal)?,
            border: self.frame == "border",
            shop_name: self.show_shop_name.then(|| shop.name.clone()),
            watermark: shop.plan == "free",
        })
    }
    fn input(&self) -> TemplateInput {
        TemplateInput {
            name: self.name.clone(),
            logo: self.logo.as_ref().map(|b| STANDARD.encode(b)),
            fg_color: self.fg_color.clone(),
            bg_color: self.bg_color.clone(),
            frame: self.frame.clone(),
            show_shop_name: self.show_shop_name,
        }
    }
    fn view(&self) -> Value {
        let mut value = serde_json::to_value(self.input()).expect("template JSON");
        value["id"] = self.id.into();
        value["shop_id"] = self.shop_id.into();
        value["version"] = self.version.into();
        value
    }
}

pub(super) async fn owned_template(
    pool: &PgPool,
    shop: i64,
    id: i64,
) -> Result<Template, ApiError> {
    Ok(
        sqlx::query_as("SELECT * FROM templates WHERE id = $1 AND shop_id = $2")
            .bind(id)
            .bind(shop)
            .fetch_one(pool)
            .await?,
    )
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct TemplateInput {
    name: String,
    logo: Option<String>,
    #[serde(default = "black")]
    fg_color: String,
    #[serde(default = "white")]
    bg_color: String,
    #[serde(default = "none")]
    frame: String,
    #[serde(default)]
    show_shop_name: bool,
}
fn black() -> String {
    "#000000".into()
}
fn white() -> String {
    "#ffffff".into()
}
fn none() -> String {
    "none".into()
}

fn invalid(message: &str) -> ApiError {
    ApiError::bad_request("InvalidRequest", message)
}
fn name_valid(name: &str) -> bool {
    !name.trim().is_empty() && name.chars().count() <= 100 && !name.chars().any(char::is_control)
}

impl TemplateInput {
    fn validate(&self) -> Result<Option<Vec<u8>>, ApiError> {
        if !name_valid(&self.name) {
            return Err(invalid("Name must contain 1–100 printable characters"));
        }
        if !matches!(self.frame.as_str(), "none" | "border") {
            return Err(invalid("frame must be none or border"));
        }
        render::validate_colors(&self.fg_color, &self.bg_color)?;
        self.logo.as_deref().map(render::normalize_logo).transpose()
    }
}

fn input<T: DeserializeOwned>(body: Result<Json<Value>, JsonRejection>) -> Result<T, ApiError> {
    let Json(value) = body.map_err(|_| invalid("Invalid JSON or body size"))?;
    serde_json::from_value(value).map_err(|_| invalid("Invalid fields or types"))
}

fn patched<T: Serialize + DeserializeOwned>(
    old: T,
    patch: Result<Json<Value>, JsonRejection>,
) -> Result<T, ApiError> {
    let Json(patch) = patch.map_err(|_| invalid("Invalid JSON or body size"))?;
    let fields = patch
        .as_object()
        .ok_or_else(|| invalid("PATCH must be an object"))?;
    let mut value = serde_json::to_value(old).map_err(|_| ApiError::internal())?;
    for (key, item) in fields {
        value[key] = item.clone();
    }
    serde_json::from_value(value).map_err(|_| invalid("Invalid fields or types"))
}

pub(super) async fn create_template(
    State(state): State<AppState>,
    Extension(key): Extension<KeyShop>,
    body: Result<Json<Value>, JsonRejection>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let shop = key.required()?;
    let input: TemplateInput = input(body)?;
    let (input, logo) = state
        .render_job(move || {
            let logo = input.validate()?;
            Ok((input, logo))
        })
        .await?;
    let row: Template = sqlx::query_as("INSERT INTO templates (shop_id, name, logo, fg_color, bg_color, frame, show_shop_name) VALUES ($1,$2,$3,$4,$5,$6,$7) RETURNING *")
        .bind(shop).bind(input.name).bind(logo).bind(input.fg_color).bind(input.bg_color).bind(input.frame).bind(input.show_shop_name)
        .fetch_one(&state.pool).await?;
    Ok((StatusCode::CREATED, Json(row.view())))
}

pub(super) async fn list_templates(
    State(state): State<AppState>,
    Extension(key): Extension<KeyShop>,
) -> Result<Json<Vec<Value>>, ApiError> {
    let rows: Vec<Template> =
        sqlx::query_as("SELECT * FROM templates WHERE shop_id = $1 ORDER BY id")
            .bind(key.required()?)
            .fetch_all(&state.pool)
            .await?;
    Ok(Json(rows.iter().map(Template::view).collect()))
}

pub(super) async fn get_template(
    State(state): State<AppState>,
    Extension(key): Extension<KeyShop>,
    Path(id): Path<i64>,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(
        owned_template(&state.pool, key.required()?, id)
            .await?
            .view(),
    ))
}

pub(super) async fn update_template(
    State(state): State<AppState>,
    Extension(key): Extension<KeyShop>,
    Path(id): Path<i64>,
    body: Result<Json<Value>, JsonRejection>,
) -> Result<Json<Value>, ApiError> {
    let shop = key.required()?;
    let old = owned_template(&state.pool, shop, id).await?;
    let input: TemplateInput = patched(old.input(), body)?;
    let (input, logo) = state
        .render_job(move || {
            let logo = input.validate()?;
            Ok((input, logo))
        })
        .await?;
    let row: Template = sqlx::query_as("UPDATE templates SET name=$3, logo=$4, fg_color=$5, bg_color=$6, frame=$7, show_shop_name=$8, version=version+1 WHERE id=$1 AND shop_id=$2 RETURNING *")
        .bind(id).bind(shop).bind(input.name).bind(logo).bind(input.fg_color).bind(input.bg_color).bind(input.frame).bind(input.show_shop_name)
        .fetch_one(&state.pool).await?;
    Ok(Json(row.view()))
}

pub(super) async fn delete_template(
    State(state): State<AppState>,
    Extension(key): Extension<KeyShop>,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    let result = sqlx::query("DELETE FROM templates WHERE id=$1 AND shop_id=$2")
        .bind(id)
        .bind(key.required()?)
        .execute(&state.pool)
        .await?;
    if result.rows_affected() == 0 {
        return Err(sqlx::Error::RowNotFound.into());
    }
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct LinkInput {
    slug: String,
    template_id: Option<i64>,
    default_amount: Option<u64>,
    default_memo: Option<String>,
    #[serde(default = "active")]
    active: bool,
}
fn active() -> bool {
    true
}

#[derive(sqlx::FromRow, Serialize)]
pub(super) struct Link {
    pub id: i64,
    pub shop_id: i64,
    pub slug: String,
    pub template_id: Option<i64>,
    pub default_amount: Option<i64>,
    pub default_memo: Option<String>,
    pub active: bool,
}

fn valid_slug(slug: &str) -> bool {
    (3..=40).contains(&slug.len())
        && slug
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

impl LinkInput {
    async fn validate(&mut self, pool: &PgPool, shop_id: i64) -> Result<(), ApiError> {
        if !valid_slug(&self.slug) {
            return Err(invalid("slug must match [a-z0-9-], 3–40 characters"));
        }
        if let Some(id) = self.template_id {
            owned_template(pool, shop_id, id).await?;
        }
        let shop = shop(pool, shop_id).await?;
        let payload = payment_payload(&shop, self.default_amount, self.default_memo.as_deref())?;
        self.default_memo = vietqr_core::parse(&payload)?.memo.map(str::to_owned);
        Ok(())
    }
}

pub(super) fn payment_payload(
    shop: &Shop,
    amount: Option<u64>,
    memo: Option<&str>,
) -> Result<String, ApiError> {
    Ok(vietqr_core::generate(&vietqr_core::Transfer {
        bank_bin: &shop.bank_bin,
        account: &shop.account,
        service: vietqr_core::Service::Account,
        name: shop.account_name.as_deref(),
        amount,
        memo,
    })?)
}

async fn owned_link(pool: &PgPool, shop: i64, id: i64) -> Result<Link, ApiError> {
    Ok(
        sqlx::query_as("SELECT * FROM links WHERE id=$1 AND shop_id=$2")
            .bind(id)
            .bind(shop)
            .fetch_one(pool)
            .await?,
    )
}

pub(super) async fn create_link(
    State(state): State<AppState>,
    Extension(key): Extension<KeyShop>,
    body: Result<Json<Value>, JsonRejection>,
) -> Result<(StatusCode, Json<Link>), ApiError> {
    let shop = key.required()?;
    let mut input: LinkInput = input(body)?;
    input.validate(&state.pool, shop).await?;
    let row = sqlx::query_as("INSERT INTO links (shop_id, slug, template_id, default_amount, default_memo, active) VALUES ($1,$2,$3,$4,$5,$6) RETURNING *")
        .bind(shop).bind(input.slug).bind(input.template_id).bind(input.default_amount.map(|v| v as i64)).bind(input.default_memo).bind(input.active)
        .fetch_one(&state.pool).await?;
    Ok((StatusCode::CREATED, Json(row)))
}

pub(super) async fn list_links(
    State(state): State<AppState>,
    Extension(key): Extension<KeyShop>,
) -> Result<Json<Vec<Link>>, ApiError> {
    Ok(Json(
        sqlx::query_as("SELECT * FROM links WHERE shop_id=$1 ORDER BY id")
            .bind(key.required()?)
            .fetch_all(&state.pool)
            .await?,
    ))
}

pub(super) async fn get_link(
    State(state): State<AppState>,
    Extension(key): Extension<KeyShop>,
    Path(id): Path<i64>,
) -> Result<Json<Link>, ApiError> {
    Ok(Json(owned_link(&state.pool, key.required()?, id).await?))
}

pub(super) async fn update_link(
    State(state): State<AppState>,
    Extension(key): Extension<KeyShop>,
    Path(id): Path<i64>,
    body: Result<Json<Value>, JsonRejection>,
) -> Result<Json<Link>, ApiError> {
    let shop = key.required()?;
    let old = owned_link(&state.pool, shop, id).await?;
    let old = LinkInput {
        slug: old.slug,
        template_id: old.template_id,
        default_amount: old.default_amount.map(|v| v as u64),
        default_memo: old.default_memo,
        active: old.active,
    };
    let Json(patch) = body.map_err(|_| invalid("Invalid JSON"))?;
    if patch.get("slug").is_some() {
        return Err(invalid("slug is immutable"));
    }
    let mut input: LinkInput = patched(old, Ok(Json(patch)))?;
    input.validate(&state.pool, shop).await?;
    let row = sqlx::query_as("UPDATE links SET template_id=$3, default_amount=$4, default_memo=$5, active=$6 WHERE id=$1 AND shop_id=$2 RETURNING *")
        .bind(id).bind(shop).bind(input.template_id).bind(input.default_amount.map(|v| v as i64)).bind(input.default_memo).bind(input.active)
        .fetch_one(&state.pool).await?;
    Ok(Json(row))
}

pub(super) async fn deactivate_link(
    State(state): State<AppState>,
    Extension(key): Extension<KeyShop>,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    let result = sqlx::query("UPDATE links SET active=false WHERE id=$1 AND shop_id=$2")
        .bind(id)
        .bind(key.required()?)
        .execute(&state.pool)
        .await?;
    if result.rows_affected() == 0 {
        return Err(sqlx::Error::RowNotFound.into());
    }
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn slug_validation() {
        for slug in ["abc", "abc-123", &"x".repeat(40)] {
            assert!(valid_slug(slug));
        }
        for slug in ["ab", "ABC", "a b", "a/b", "ábc", &"x".repeat(41)] {
            assert!(!valid_slug(slug));
        }
    }
    #[test]
    fn patch_preserves_fields_and_clears_nullable_values() {
        let old = LinkInput {
            slug: "abc".into(),
            template_id: Some(1),
            default_amount: Some(100),
            default_memo: Some("TEST".into()),
            active: true,
        };
        let new: LinkInput = patched(
            old,
            Ok(Json(
                serde_json::json!({"default_amount":null, "default_memo":null, "template_id":null}),
            )),
        )
        .unwrap();
        assert_eq!(new.slug, "abc");
        assert!(
            new.active
                && new.template_id.is_none()
                && new.default_amount.is_none()
                && new.default_memo.is_none()
        );
        assert!(patched(new, Ok(Json(serde_json::json!({"shop_id":2})))).is_err());
    }
}
