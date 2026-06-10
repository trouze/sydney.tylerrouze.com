use axum::{
    routing::{get, post},
    Router,
};
use std::net::SocketAddr;
use tower_http::{services::ServeDir, trace::TraceLayer};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod db;
mod error;
mod handlers;
mod models;

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
        .route("/rsvp", get(handlers::rsvp::rsvp_page))
        .route("/rsvp", post(handlers::rsvp::rsvp_submit))
        .nest_service("/static", ServeDir::new("static"))
        .with_state(pool)
        .layer(TraceLayer::new_for_http());

    let addr = SocketAddr::from(([0, 0, 0, 0], 8080));
    tracing::info!("listening on {addr}");

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
