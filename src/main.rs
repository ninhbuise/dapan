// poise's #[rest] argument expansion trips this lint on newer compilers.
#![allow(unreachable_code)]

mod calendar;
mod config;
mod db;
mod music;
mod notify;
mod scheduler;
mod tracker;
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
    /// What the bot is doing now and how long recent tasks took (/tasks).
    pub tracker: tracker::Tracker,
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
    // A bad line makes dotenvy silently skip everything after it, so treat it as fatal.
    // (A missing .env is fine: settings can also come from real environment variables.)
    if let Err(e) = dotenvy::dotenv()
        && !e.not_found()
    {
        anyhow::bail!(
            ".env có dòng lỗi: {e}. Giá trị có dấu cách phải để trong ngoặc kép, ví dụ HOME_CITY=\"Hà Nội\""
        );
    }
    // Several deps pull different rustls crypto backends; pick one explicitly.
    let _ = rustls::crypto::ring::default_provider().install_default();
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        // No color codes when output goes to a log file (autostart script).
        .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stdout()))
        .init();

    let cfg = Arc::new(Config::from_env()?);
    match &cfg.tts_local_url {
        Some(url) => tracing::info!(
            "Giọng nói: local {url} ({}), dự phòng Edge",
            cfg.tts_local_voice.as_deref().unwrap_or("mặc định")
        ),
        None => tracing::info!("Giọng nói: Edge (TTS_LOCAL_URL chưa đặt)"),
    }
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
        tracker: tracker::Tracker::default(),
    };

    let mut commands = vec![ping(), help(), tasks()];
    commands.extend(music::commands());
    commands.extend(tts::commands());
    commands.extend(weather::commands());
    commands.extend(calendar::commands());

    let framework = poise::Framework::builder()
        .options(poise::FrameworkOptions {
            commands,
            event_handler: |fw, event| Box::pin(event_handler(fw, event)),
            on_error: |err| Box::pin(on_error(err)),
            pre_command: |ctx| {
                Box::pin(async move {
                    ctx.data()
                        .tracker
                        .start(ctx.id(), ctx.invocation_string(), who(ctx));
                })
            },
            post_command: |ctx| {
                Box::pin(async move {
                    ctx.data().tracker.finish(ctx.id(), true);
                })
            },
            ..Default::default()
        })
        .setup(move |ctx, ready, framework| {
            Box::pin(async move {
                tracing::info!("Đã đăng nhập: {}", ready.user.name);
                let cmds = &framework.options().commands;
                let registered = match data.cfg.guild_id {
                    Some(g) => poise::builtins::register_in_guild(ctx, cmds, GuildId::new(g)).await,
                    None => poise::builtins::register_globally(ctx, cmds).await,
                };
                if let Err(e) = registered {
                    // Exit so the runner script retries, instead of staying online without commands.
                    tracing::error!(
                        "Không đăng ký được lệnh: {e}. Kiểm tra GUILD_ID và mời bot vào server với scope applications.commands."
                    );
                    std::process::exit(2);
                }
                match data.cfg.guild_id {
                    Some(g) => tracing::info!("Đã đăng ký {} lệnh cho server {g}", cmds.len()),
                    None => {
                        // Remove server-only copies left from when GUILD_ID was set,
                        // otherwise those servers show every command twice.
                        for g in &ready.guilds {
                            if let Err(e) = g.id.set_commands(&ctx.http, Vec::new()).await {
                                tracing::warn!("không xoá được lệnh cũ ở server {}: {e}", g.id);
                            }
                        }
                        tracing::info!(
                            "Đã đăng ký {} lệnh toàn cục cho mọi server ({} server hiện tại)",
                            cmds.len(),
                            ready.guilds.len()
                        );
                    }
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
            ctx.data().tracker.note(ctx.id(), format!("lỗi: {error}"));
            ctx.data().tracker.finish(ctx.id(), false);
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

/// "user @ server" label for logs.
fn who(ctx: Context<'_>) -> String {
    let server = ctx
        .guild()
        .map(|g| g.name.clone())
        .unwrap_or_else(|| "DM".into());
    format!("{} @ {server}", ctx.author().name)
}

/// Bot đang làm gì và mất bao lâu
#[poise::command(slash_command, prefix_command)]
async fn tasks(ctx: Context<'_>) -> Result<(), Error> {
    let snap = ctx.data().tracker.snapshot(10);
    let running = if snap.running.len() <= 1 {
        "Không có việc nào khác 😴".to_string() // only this /tasks call itself
    } else {
        snap.running
            .iter()
            .filter(|(name, ..)| !name.starts_with("/tasks"))
            .map(|(name, who, el, stage)| {
                format!(
                    "⏳ `{name}` · {who} · {} · đang **{stage}**",
                    tracker::secs(*el)
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    let recent = if snap.recent.is_empty() {
        "Chưa có".to_string()
    } else {
        snap.recent
            .iter()
            .map(|f| {
                let mark = if f.ok { "✅" } else { "❌" };
                let slow = if f.took.as_secs() >= 5 { " 🐢" } else { "" };
                let detail = if f.detail.is_empty() {
                    String::new()
                } else {
                    format!("\n  └ {}", f.detail.chars().take(150).collect::<String>())
                };
                format!(
                    "{mark} <t:{}:T> `{}` · {} · **{}**{slow}{detail}",
                    f.at.timestamp(),
                    f.name.chars().take(40).collect::<String>(),
                    f.who,
                    tracker::secs(f.took)
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    let e = serenity::CreateEmbed::new()
        .title("📊 Dapan đang làm gì?")
        .field("Đang chạy", running, false)
        .field(
            "Gần đây",
            recent.chars().take(1024).collect::<String>(),
            false,
        )
        .color(notify::PINK);
    ctx.send(poise::CreateReply::default().embed(e).ephemeral(true))
        .await?;
    Ok(())
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
