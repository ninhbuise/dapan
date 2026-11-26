// poise's #[rest] argument expansion trips this lint on newer compilers.
#![allow(unreachable_code)]

mod calendar;
mod config;
mod db;
mod music;
mod notify;
mod scheduler;
mod tts;
mod weather;

use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Instant;

use poise::serenity_prelude as serenity;
use serenity::{ChannelId, GuildId};
use songbird::{SerenityInit, Songbird};
use sqlx::SqlitePool;
use tokio::sync::RwLock;

use crate::config::Config;

pub type Error = anyhow::Error;
pub type Context<'a> = poise::Context<'a, Data, Error>;

/// State shared by every command and background task.
pub struct Data {
    pub cfg: Arc<Config>,
    pub http: reqwest::Client,
    pub db: SqlitePool,
    pub songbird: Arc<Songbird>,
    pub spotify: music::spotify::Spotify,
    /// Music volume per guild (0.0 – 2.0).
    pub volumes: StdMutex<HashMap<GuildId, f32>>,
    /// Last time something was played or spoken in a guild (used for idle disconnect).
    pub last_activity: Arc<StdMutex<HashMap<GuildId, Instant>>>,
    /// Text channel per guild whose messages are read aloud.
    pub tts_channels: StdMutex<HashMap<GuildId, ChannelId>>,
    /// One speaker at a time per guild.
    pub tts_locks: StdMutex<HashMap<GuildId, Arc<tokio::sync::Mutex<()>>>>,
    /// Upcoming events from the iCal feed.
    pub calendar: Arc<RwLock<Vec<calendar::ical::Occurrence>>>,
}

impl Data {
    pub fn volume(&self, guild: GuildId) -> f32 {
        *self.volumes.lock().unwrap().get(&guild).unwrap_or(&0.6)
    }
    pub fn touch(&self, guild: GuildId) {
        self.last_activity
            .lock()
            .unwrap()
            .insert(guild, Instant::now());
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = dotenvy::dotenv();
    // Several deps pull different rustls crypto backends; pick one explicitly.
    let _ = rustls::crypto::ring::default_provider().install_default();
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    let cfg = Arc::new(Config::from_env()?);
    let db = db::connect(&cfg.database_path).await?;
    let http = reqwest::Client::builder()
        .user_agent("dapan-discord-bot/0.1")
        .build()?;
    let songbird = Songbird::serenity();

    let tts_channels: HashMap<GuildId, ChannelId> =
        sqlx::query_as::<_, (i64, i64)>("SELECT guild_id, channel_id FROM tts_channels")
            .fetch_all(&db)
            .await?
            .into_iter()
            .map(|(g, c)| (GuildId::new(g as u64), ChannelId::new(c as u64)))
            .collect();

    let data = Data {
        cfg: cfg.clone(),
        http: http.clone(),
        db,
        songbird: songbird.clone(),
        spotify: music::spotify::Spotify::new(
            http.clone(),
            cfg.spotify_client_id.clone(),
            cfg.spotify_client_secret.clone(),
        ),
        volumes: StdMutex::new(HashMap::new()),
        last_activity: Arc::new(StdMutex::new(HashMap::new())),
        tts_channels: StdMutex::new(tts_channels),
        tts_locks: StdMutex::new(HashMap::new()),
        calendar: Arc::new(RwLock::new(Vec::new())),
    };

    let mut commands = vec![ping(), help()];
    commands.extend(music::commands());
    commands.extend(tts::commands());
    commands.extend(weather::commands());
    commands.extend(calendar::commands());

    let framework = poise::Framework::builder()
        .options(poise::FrameworkOptions {
            commands,
            event_handler: |fw, event| Box::pin(event_handler(fw, event)),
            on_error: |err| Box::pin(on_error(err)),
            ..Default::default()
        })
        .setup(move |ctx, ready, framework| {
            Box::pin(async move {
                tracing::info!("Đã đăng nhập: {}", ready.user.name);
                let cmds = &framework.options().commands;
                match data.cfg.guild_id {
                    Some(g) => {
                        poise::builtins::register_in_guild(ctx, cmds, GuildId::new(g)).await?
                    }
                    None => poise::builtins::register_globally(ctx, cmds).await?,
                }
                scheduler::spawn_all(ctx.http.clone(), &data);
                Ok(data)
            })
        })
        .build();

    let intents =
        serenity::GatewayIntents::non_privileged() | serenity::GatewayIntents::MESSAGE_CONTENT;

    let mut client = serenity::ClientBuilder::new(&cfg.discord_token, intents)
        .framework(framework)
        .register_songbird_with(songbird)
        .await?;
    client.start().await?;
    Ok(())
}

async fn event_handler(
    fw: poise::FrameworkContext<'_, Data, Error>,
    event: &serenity::FullEvent,
) -> Result<(), Error> {
    if let serenity::FullEvent::Message { new_message } = event {
        tts::on_message(fw.serenity_context, fw.user_data, new_message).await;
    }
    Ok(())
}

async fn on_error(err: poise::FrameworkError<'_, Data, Error>) {
    match err {
        poise::FrameworkError::Command { error, ctx, .. } => {
            tracing::warn!("command /{} failed: {error:#}", ctx.command().name);
            let _ = ctx.say(format!("Huhu, có lỗi rồi (｡•́︿•̀｡): {error}")).await;
        }
        other => {
            if let Err(e) = poise::builtins::on_error(other).await {
                tracing::error!("error while handling error: {e}");
            }
        }
    }
}

/// Kiểm tra bot còn sống không
#[poise::command(slash_command, prefix_command)]
async fn ping(ctx: Context<'_>) -> Result<(), Error> {
    ctx.say("Pong! Dapan đây nè ٩(◕‿◕)۶").await?;
    Ok(())
}

/// Danh sách lệnh
#[poise::command(slash_command, prefix_command)]
async fn help(ctx: Context<'_>) -> Result<(), Error> {
    let lines: Vec<String> = ctx
        .framework()
        .options()
        .commands
        .iter()
        .map(|c| format!("`/{}` — {}", c.name, c.description.as_deref().unwrap_or("")))
        .collect();
    let e = serenity::CreateEmbed::new()
        .title("Dapan có thể làm gì nè? (◕‿◕✿)")
        .description(lines.join("\n"))
        .footer(serenity::CreateEmbedFooter::new(
            "nhạc 🎶 · đọc chữ 🗣️ · thời tiết ☀️ · nhắc lịch 📅",
        ))
        .color(notify::PINK);
    ctx.send(poise::CreateReply::default().embed(e).ephemeral(true))
        .await?;
    Ok(())
}
