pub mod resolve;
pub mod spotify;

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, Result, bail};
use poise::CreateReply;
use poise::serenity_prelude::{ChannelId, CreateEmbed, GuildId, UserId};
use songbird::Call;
use songbird::tracks::Track;
use tokio::sync::Mutex;

use crate::notify::PINK;
use crate::{Context, Data, Error};

pub fn commands() -> Vec<poise::Command<Data, Error>> {
    vec![
        join(),
        play(),
        skip(),
        stop(),
        pause(),
        resume(),
        queue(),
        nowplaying(),
        volume(),
        leave(),
    ]
}

/// Info attached to every queued music track.
pub struct TrackInfo {
    pub title: String,
    pub url: Option<String>,
    pub duration: Option<Duration>,
    pub requested_by: UserId,
}

pub fn fmt_duration(d: Option<Duration>) -> String {
    match d {
        Some(d) => {
            let s = d.as_secs();
            if s >= 3600 {
                format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
            } else {
                format!("{}:{:02}", s / 60, s % 60)
            }
        }
        None => "?".into(),
    }
}

/// Voice channel that `user` is currently in, from the gateway cache.
pub fn user_voice_channel(
    ctx: &poise::serenity_prelude::Context,
    guild: GuildId,
    user: UserId,
) -> Option<ChannelId> {
    let g = ctx.cache.guild(guild)?;
    g.voice_states.get(&user).and_then(|vs| vs.channel_id)
}

/// Returns the existing call for the guild, or joins `channel`.
pub async fn ensure_call(
    data: &Data,
    guild: GuildId,
    channel: Option<ChannelId>,
) -> Result<Arc<Mutex<Call>>> {
    if let Some(call) = data.songbird.get(guild)
        && call.lock().await.current_channel().is_some()
    {
        return Ok(call);
    }
    let channel = channel.context("Bạn vào một kênh voice trước rồi gọi Dapan nha 🥺")?;
    let call = data.songbird.join(guild, channel).await?;
    call.lock().await.deafen(true).await.ok();
    data.touch(guild);
    Ok(call)
}

async fn call_for_author(ctx: Context<'_>) -> Result<(GuildId, Arc<Mutex<Call>>)> {
    let guild = ctx
        .guild_id()
        .context("Lệnh này chỉ dùng trong server thôi nha")?;
    let channel = user_voice_channel(ctx.serenity_context(), guild, ctx.author().id);
    Ok((guild, ensure_call(ctx.data(), guild, channel).await?))
}

fn existing_call(ctx: Context<'_>) -> Result<(GuildId, Arc<Mutex<Call>>)> {
    let guild = ctx
        .guild_id()
        .context("Lệnh này chỉ dùng trong server thôi nha")?;
    let call = ctx
        .data()
        .songbird
        .get(guild)
        .context("Dapan đang không ở kênh voice nào cả")?;
    Ok((guild, call))
}

fn embed(title: &str, desc: impl Into<String>) -> CreateEmbed {
    CreateEmbed::new()
        .title(title)
        .description(desc)
        .color(PINK)
}

/// Vào kênh voice của bạn
#[poise::command(slash_command, guild_only)]
async fn join(ctx: Context<'_>) -> Result<(), Error> {
    call_for_author(ctx).await?;
    ctx.say("Dapan vào rồi nè ✨").await?;
    Ok(())
}

/// Phát nhạc từ YouTube / Spotify (link hoặc tên bài)
#[poise::command(slash_command, prefix_command, guild_only, aliases("p"))]
async fn play(
    ctx: Context<'_>,
    #[description = "Link YouTube/Spotify hoặc tên bài hát"]
    #[rest]
    query: String,
) -> Result<(), Error> {
    ctx.defer().await?;
    let (guild, call) = call_for_author(ctx).await?;
    let data = ctx.data();

    let tracks = resolve::resolve(&data.http, &data.spotify, &query).await?;
    let count = tracks.len();
    let first_title = tracks[0].title.clone();
    let first_url = tracks[0].url.clone();
    let first_dur = tracks[0].duration;
    let vol = data.volume(guild);

    let mut handler = call.lock().await;
    let was_empty = handler.queue().is_empty();
    for t in tracks {
        let info = TrackInfo {
            title: t.title,
            url: t.url,
            duration: t.duration,
            requested_by: ctx.author().id,
        };
        let track = Track::new_with_data(t.source.into(), Arc::new(info)).volume(vol);
        handler.enqueue(track).await;
    }
    let pos = handler.queue().len();
    drop(handler);
    data.touch(guild);

    let msg = if count > 1 {
        embed(
            "📜 Đã thêm playlist",
            format!("Thêm **{count}** bài vào hàng chờ, bắt đầu từ **{first_title}**"),
        )
    } else {
        let link = first_url
            .map(|u| format!("[{first_title}]({u})"))
            .unwrap_or(first_title);
        let title = if was_empty {
            "🎶 Đang phát"
        } else {
            "➕ Đã thêm vào hàng chờ"
        };
        let mut e = embed(title, format!("{link} `{}`", fmt_duration(first_dur)));
        if !was_empty {
            e = e.field("Vị trí", pos.to_string(), true);
        }
        e
    };
    ctx.send(CreateReply::default().embed(msg)).await?;
    Ok(())
}

/// Bỏ qua bài đang phát
#[poise::command(slash_command, prefix_command, guild_only, aliases("s"))]
async fn skip(ctx: Context<'_>) -> Result<(), Error> {
    let (_, call) = existing_call(ctx)?;
    let handler = call.lock().await;
    if handler.queue().is_empty() {
        bail!("Hàng chờ đang trống mà");
    }
    handler.queue().skip()?;
    ctx.say("⏭️ Qua bài khác nha~").await?;
    Ok(())
}

/// Dừng nhạc và xoá hàng chờ
#[poise::command(slash_command, prefix_command, guild_only)]
async fn stop(ctx: Context<'_>) -> Result<(), Error> {
    let (_, call) = existing_call(ctx)?;
    call.lock().await.queue().stop();
    ctx.say("⏹️ Dừng nhạc rồi nè").await?;
    Ok(())
}

/// Tạm dừng
#[poise::command(slash_command, prefix_command, guild_only)]
async fn pause(ctx: Context<'_>) -> Result<(), Error> {
    let (_, call) = existing_call(ctx)?;
    call.lock().await.queue().pause()?;
    ctx.say("⏸️ Tạm dừng").await?;
    Ok(())
}

/// Phát tiếp
#[poise::command(slash_command, prefix_command, guild_only)]
async fn resume(ctx: Context<'_>) -> Result<(), Error> {
    let (guild, call) = existing_call(ctx)?;
    call.lock().await.queue().resume()?;
    ctx.data().touch(guild);
    ctx.say("▶️ Phát tiếp nè").await?;
    Ok(())
}

/// Xem hàng chờ
#[poise::command(slash_command, prefix_command, guild_only, aliases("q"))]
async fn queue(ctx: Context<'_>) -> Result<(), Error> {
    let (_, call) = existing_call(ctx)?;
    let list = call.lock().await.queue().current_queue();
    if list.is_empty() {
        ctx.say("Hàng chờ trống trơn (・・;)").await?;
        return Ok(());
    }
    let mut lines = Vec::new();
    for (i, h) in list.iter().take(15).enumerate() {
        let info = h.data::<TrackInfo>();
        let prefix = if i == 0 {
            "▶️".to_string()
        } else {
            format!("`{i}.`")
        };
        lines.push(format!(
            "{prefix} {} `{}`",
            info.title,
            fmt_duration(info.duration)
        ));
    }
    if list.len() > 15 {
        lines.push(format!("… và {} bài nữa", list.len() - 15));
    }
    ctx.send(CreateReply::default().embed(embed("📜 Hàng chờ", lines.join("\n"))))
        .await?;
    Ok(())
}

/// Bài đang phát
#[poise::command(slash_command, prefix_command, guild_only, aliases("np"))]
async fn nowplaying(ctx: Context<'_>) -> Result<(), Error> {
    let (_, call) = existing_call(ctx)?;
    let current = call
        .lock()
        .await
        .queue()
        .current()
        .context("Không có bài nào đang phát")?;
    let info = current.data::<TrackInfo>();
    let pos = current.get_info().await.map(|s| s.position).ok();
    let link = info
        .url
        .as_ref()
        .map(|u| format!("[{}]({u})", info.title))
        .unwrap_or(info.title.clone());
    let e = embed("🎶 Đang phát", link)
        .field(
            "Thời gian",
            format!("{} / {}", fmt_duration(pos), fmt_duration(info.duration)),
            true,
        )
        .field("Yêu cầu bởi", format!("<@{}>", info.requested_by), true);
    ctx.send(CreateReply::default().embed(e)).await?;
    Ok(())
}

/// Chỉnh âm lượng (0–200%)
#[poise::command(slash_command, prefix_command, guild_only, aliases("vol"))]
async fn volume(
    ctx: Context<'_>,
    #[description = "Phần trăm"]
    #[min = 0]
    #[max = 200]
    percent: u8,
) -> Result<(), Error> {
    let guild = ctx.guild_id().context("guild only")?;
    let v = percent as f32 / 100.0;
    ctx.data().volumes.lock().unwrap().insert(guild, v);
    if let Some(call) = ctx.data().songbird.get(guild) {
        for h in call.lock().await.queue().current_queue() {
            let _ = h.set_volume(v);
        }
    }
    ctx.say(format!("🔊 Âm lượng: {percent}%")).await?;
    Ok(())
}

/// Rời kênh voice
#[poise::command(slash_command, prefix_command, guild_only, aliases("dc"))]
async fn leave(ctx: Context<'_>) -> Result<(), Error> {
    let guild = ctx.guild_id().context("guild only")?;
    ctx.data()
        .songbird
        .remove(guild)
        .await
        .context("Dapan đang không ở kênh voice nào")?;
    ctx.say("Bái bai 👋").await?;
    Ok(())
}
