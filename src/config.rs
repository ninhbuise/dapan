use anyhow::{Context as _, Result};
use chrono::NaiveTime;
use chrono_tz::Tz;

/// All runtime settings, read from environment variables (or `.env`).
#[derive(Debug, Clone)]
pub struct Config {
    pub discord_token: String,
    /// When set, slash commands are registered to this guild only (shows up instantly).
    pub guild_id: Option<u64>,

    pub spotify_client_id: Option<String>,
    pub spotify_client_secret: Option<String>,

    /// Secret iCal address of a Google Calendar (optional).
    pub ical_url: Option<String>,
    /// Text channel that receives notifications (weather, calendar).
    pub notify_channel_id: Option<u64>,
    /// User that receives notifications by DM.
    pub owner_user_id: Option<u64>,
    /// Minutes before a calendar event to send a reminder.
    pub remind_before_min: i64,

    pub home_city: String,
    pub home_lat: f64,
    pub home_lon: f64,
    pub morning_time: NaiveTime,
    pub tz: Tz,

    pub tts_voice: String,
    /// Pitch shift in semitones applied with FFmpeg (higher = cuter).
    pub tts_pitch: f32,
    /// Speaking speed multiplier (1.0 = normal).
    pub tts_speed: f32,

    pub database_path: String,
}

fn opt(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn opt_parse<T: std::str::FromStr>(key: &str) -> Result<Option<T>>
where
    T::Err: std::fmt::Display,
{
    match opt(key) {
        None => Ok(None),
        Some(v) => v
            .parse::<T>()
            .map(Some)
            .map_err(|e| anyhow::anyhow!("invalid value for {key}: {e}")),
    }
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let morning = opt("MORNING_TIME").unwrap_or_else(|| "06:30".into());
        let tz_name = opt("TIMEZONE").unwrap_or_else(|| "Asia/Ho_Chi_Minh".into());

        Ok(Self {
            discord_token: opt("DISCORD_TOKEN")
                .context("DISCORD_TOKEN is missing (see .env.example)")?,
            guild_id: opt_parse("GUILD_ID")?,
            spotify_client_id: opt("SPOTIFY_CLIENT_ID"),
            spotify_client_secret: opt("SPOTIFY_CLIENT_SECRET"),
            ical_url: opt("ICAL_URL"),
            notify_channel_id: opt_parse("NOTIFY_CHANNEL_ID")?,
            owner_user_id: opt_parse("OWNER_USER_ID")?,
            remind_before_min: opt_parse("REMIND_BEFORE_MIN")?.unwrap_or(15),
            home_city: opt("HOME_CITY").unwrap_or_else(|| "Hà Nội".into()),
            home_lat: opt_parse("HOME_LAT")?.unwrap_or(21.0285),
            home_lon: opt_parse("HOME_LON")?.unwrap_or(105.8542),
            morning_time: NaiveTime::parse_from_str(&morning, "%H:%M")
                .with_context(|| format!("MORNING_TIME must look like 06:30, got {morning}"))?,
            tz: tz_name
                .parse::<Tz>()
                .map_err(|e| anyhow::anyhow!("invalid TIMEZONE {tz_name}: {e}"))?,
            tts_voice: opt("TTS_VOICE").unwrap_or_else(|| {
                "Microsoft Server Speech Text to Speech Voice (vi-VN, HoaiMyNeural)".into()
            }),
            tts_pitch: opt_parse("TTS_PITCH")?.unwrap_or(2.5),
            tts_speed: opt_parse("TTS_SPEED")?.unwrap_or(1.05),
            database_path: opt("DATABASE_PATH").unwrap_or_else(|| "data/dapan.db".into()),
        })
    }
}
