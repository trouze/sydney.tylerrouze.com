use std::io::{Cursor, Write};

use axum::{
    extract::{Path, State},
    http::{header, StatusCode},
    response::{IntoResponse, Redirect, Response},
};
use axum_extra::extract::cookie::CookieJar;
use qrcode::{render::svg, QrCode};

use crate::{error::AppError, handlers::admin::require_admin, AppState};

fn base_url() -> String {
    std::env::var("BASE_URL").unwrap_or_else(|_| "https://sydney.tylerrouze.com".to_string())
}

fn render_qr_svg(invite_code: &str) -> Result<String, AppError> {
    let url = format!("{}/rsvp?code={}", base_url(), invite_code);
    let code = QrCode::new(url.as_bytes()).map_err(|e| anyhow::anyhow!("qr encode failed: {e}"))?;
    Ok(code.render::<svg::Color>().min_dimensions(256, 256).build())
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

    let svg = render_qr_svg(&invite_code)?;
    Ok(([(header::CONTENT_TYPE, "image/svg+xml")], svg).into_response())
}

/// GET /admin/qr/all.zip — ZIP of QR SVGs for all parties with at least one attending guest.
pub async fn download_all_qr_zip(
    State(pool): State<AppState>,
    jar: CookieJar,
) -> Result<Response, AppError> {
    if !require_admin(&jar) {
        return Ok(Redirect::to("/admin/login").into_response());
    }

    let parties = sqlx::query_as::<_, (String, String)>(
        "SELECT DISTINCT p.invite_code, p.label
         FROM parties p
         JOIN guests g ON g.party_id = p.id
         JOIN current_rsvp cr ON cr.guest_id = g.id
         WHERE cr.attending = 1
         ORDER BY p.label",
    )
    .fetch_all(&pool)
    .await?;

    if parties.is_empty() {
        return Ok((StatusCode::NOT_FOUND, "No attending parties yet").into_response());
    }

    let mut buf = Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut buf);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for (invite_code, _label) in &parties {
            let svg = render_qr_svg(invite_code)?;
            zip.start_file(format!("qr-{invite_code}.svg"), options)
                .map_err(|e| anyhow::anyhow!("zip error: {e}"))?;
            zip.write_all(svg.as_bytes())
                .map_err(|e| anyhow::anyhow!("zip write error: {e}"))?;
        }
        zip.finish().map_err(|e| anyhow::anyhow!("zip finish error: {e}"))?;
    }

    let bytes = buf.into_inner();
    Ok((
        [
            (header::CONTENT_TYPE, "application/zip"),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=\"qr-codes.zip\"",
            ),
        ],
        bytes,
    )
        .into_response())
}
