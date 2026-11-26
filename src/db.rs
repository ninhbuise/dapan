use std::str::FromStr;

use anyhow::Result;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions};

pub async fn connect(path: &str) -> Result<SqlitePool> {
    if let Some(parent) = std::path::Path::new(path).parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }
    let opts = SqliteConnectOptions::from_str(&format!("sqlite://{path}"))?.create_if_missing(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(4)
        .connect_with(opts)
        .await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS reminders (
            id         INTEGER PRIMARY KEY AUTOINCREMENT,
            user_id    INTEGER NOT NULL,
            channel_id INTEGER NOT NULL,
            text       TEXT    NOT NULL,
            remind_at  INTEGER NOT NULL,
            sent       INTEGER NOT NULL DEFAULT 0
        )",
    )
    .execute(&pool)
    .await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS tts_channels (
            guild_id   INTEGER PRIMARY KEY,
            channel_id INTEGER NOT NULL
        )",
    )
    .execute(&pool)
    .await?;

    Ok(pool)
}
