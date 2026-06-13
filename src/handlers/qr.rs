use askama::Template;
use axum::{
    extract::{Path, State},
    http::{header, StatusCode},
    response::{Html, IntoResponse, Redirect, Response},
};
use axum_extra::extract::cookie::CookieJar;
use qrcode::{render::svg, QrCode};

use crate::{error::AppError, AppState};

/// Small local admin guard. Compares the `admin` cookie value to env `ADMIN_TOKEN`.
/// (Deliberately self-contained; will be de-duplicated against the admin unit at merge time.)
fn require_admin(jar: &CookieJar) -> bool {
    match std::env::var("ADMIN_TOKEN") {
        Ok(token) if !token.is_empty() => {
            jar.get("admin").map(|c| c.value()) == Some(token.as_str())
        }
        _ => false,
    }
}

fn base_url() -> String {
    std::env::var("BASE_URL").unwrap_or_else(|_| "https://sydney.tylerrouze.com".to_string())
}

struct PartyRow {
    id: String,
    invite_code: String,
    label: String,
}

/// GET /admin/parties/:id/qr.svg — render the party's magic-link RSVP URL as an SVG QR code.
pub async fn party_qr_svg(
    State(pool): State<AppState>,
    jar: CookieJar,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    if !require_admin(&jar) {
        return Ok(Redirect::to("/admin/login").into_response());
    }

    let invite_code: Option<String> =
        sqlx::query_scalar("SELECT invite_code FROM parties WHERE id = ?")
            .bind(&id)
            .fetch_optional(&pool)
            .await?;

    let Some(invite_code) = invite_code else {
        return Ok((StatusCode::NOT_FOUND, "Party not found").into_response());
    };

    let url = format!("{}/rsvp?code={}", base_url(), invite_code);
    let code = QrCode::new(url.as_bytes()).map_err(|e| anyhow::anyhow!("qr encode failed: {e}"))?;
    let svg = code.render::<svg::Color>().min_dimensions(256, 256).build();

    Ok(([(header::CONTENT_TYPE, "image/svg+xml")], svg).into_response())
}

#[derive(Template)]
#[template(path = "admin_qr.html")]
struct AdminQrTemplate {
    parties: Vec<PartyRow>,
}

/// GET /admin/qr — list parties with their QR codes and download links.
pub async fn admin_qr_page(
    State(pool): State<AppState>,
    jar: CookieJar,
) -> Result<Response, AppError> {
    if !require_admin(&jar) {
        return Ok(Redirect::to("/admin/login").into_response());
    }

    let parties = sqlx::query_as::<_, (String, String, String)>(
        "SELECT id, invite_code, label FROM parties ORDER BY label",
    )
    .fetch_all(&pool)
    .await?
    .into_iter()
    .map(|(id, invite_code, label)| PartyRow {
        id,
        invite_code,
        label,
    })
    .collect();

    let body = AdminQrTemplate { parties }
        .render()
        .map_err(|e| anyhow::anyhow!("template render failed: {e}"))?;
    Ok(Html(body).into_response())
}
