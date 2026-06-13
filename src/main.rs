use axum::{
    http::{StatusCode, Uri},
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
    Router,
};
use std::net::SocketAddr;
use tower_http::{services::ServeDir, trace::TraceLayer};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod db;
mod error;
mod handlers;
mod listmonk;
mod models;
mod registry;

pub type AppState = sqlx::SqlitePool;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(
            std::env::var("RUST_LOG")
                .unwrap_or_else(|_| "wedding_rsvp=debug,tower_http=debug".into()),
        ))
        .with(tracing_subscriber::fmt::layer())
        .init();

    let database_url =
        std::env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite:data/wedding.db".to_string());

    // Ensure the data directory exists for the SQLite file
    if let Some(path) = database_url.strip_prefix("sqlite:") {
        let parent = std::path::Path::new(path)
            .parent()
            .unwrap_or(std::path::Path::new("."));
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|e| anyhow::anyhow!("failed to create db directory: {e}"))?;
        }
    }

    let pool = db::connect(&database_url).await?;
    sqlx::migrate!("./migrations").run(&pool).await?;

    let app = Router::new()
        .route("/", get(handlers::home::home))
        .route("/details", get(handlers::details::details))
        .route("/story", get(handlers::story::story))
        .route("/registry", get(handlers::registry::registry))
        .route("/rsvp", get(handlers::rsvp::rsvp_page))
        .route("/rsvp", post(handlers::rsvp::rsvp_submit))
        // ---- admin (shared-secret auth) ----
        .route("/admin", get(handlers::admin::dashboard))
        .route("/admin/login", get(handlers::admin::login_page))
        .route("/admin/login", post(handlers::admin::login_submit))
        .route("/admin/logout", post(handlers::admin::logout))
        .route("/admin/analytics", get(handlers::admin::analytics))
        .route("/admin/parties", post(handlers::admin::create_party))
        .route(
            "/admin/parties/:id/edit",
            get(handlers::admin::edit_party_page),
        )
        .route("/admin/parties/:id", post(handlers::admin::update_party))
        .route("/admin/import", get(handlers::admin::import_page).post(handlers::admin::import_commit))
        .route("/admin/import/export", get(handlers::admin::import_export))
        .route("/admin/import/preview", post(handlers::admin::import_preview))
        .route("/admin/meals", get(handlers::admin::meals_page).post(handlers::admin::save_meal))
        .route("/admin/events", get(handlers::admin::events_page).post(handlers::admin::save_event))
        .route("/admin/qr/all.zip", get(handlers::qr::download_all_qr_zip))
        .route("/admin/parties/:id/qr.svg", get(handlers::qr::party_qr_svg))
        // ---- end admin ----
        .nest_service("/static", ServeDir::new("static"))
        // axum 0.7 does not auto-redirect trailing slashes, so `/admin/` would
        // 404. Bounce any unmatched `/path/` to `/path` before giving up.
        .fallback(trailing_slash_redirect)
        .with_state(pool)
        .layer(TraceLayer::new_for_http());

    let addr = SocketAddr::from(([0, 0, 0, 0], 8080));
    tracing::info!("listening on {addr}");

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

/// Redirect `/some/path/` -> `/some/path` (preserving the query string) for any
/// route that didn't otherwise match; everything else is a genuine 404.
async fn trailing_slash_redirect(uri: Uri) -> Response {
    let path = uri.path();
    if path.len() > 1 {
        if let Some(trimmed) = path.strip_suffix('/') {
            let target = match uri.query() {
                Some(q) => format!("{trimmed}?{q}"),
                None => trimmed.to_string(),
            };
            return Redirect::permanent(&target).into_response();
        }
    }
    StatusCode::NOT_FOUND.into_response()
}
