use sqlx::{
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
    SqlitePool,
};

pub type Db = SqlitePool;

pub async fn connect(database_url: &str) -> anyhow::Result<Db> {
    // Strip the sqlite: scheme and use the path directly to avoid URL-parsing
    // quirks with relative paths. `create_if_missing` ensures the file is
    // created on first run.
    let path = database_url.strip_prefix("sqlite:").unwrap_or(database_url);
    let opts = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true);

    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(opts)
        .await?;
    Ok(pool)
}
