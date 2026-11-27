pub mod auto;
pub mod resolve;
pub mod spotify;

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, Result, bail};
use poise::CreateReply;
use poise::serenity_prelude::{ChannelId, CreateEmbed, GuildId, UserId};
use songbird::Call;
use tokio::sync::Mutex;

use self::auto::{LoopMode, Player, next_id};
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
        skipto(),
        queue(),
        nowplaying(),
        remove(),
        move_track(),
        shuffle(),
        clear(),
        loop_cmd(),
        autoplay(),
        volume(),
        leave(),
    ]
}

/// Info attached to every queued music track.
#[derive(Clone)]
pub struct TrackInfo {
    /// Unique per queued copy (a looped song gets a new id).
    pub id: u64,
    pub title: String,
    pub url: Option<String>,
    pub duration: Option<Duration>,
    pub requested_by: UserId,
    /// Picked by autoplay rather than requested.
    pub auto: bool,
}

/// Handle for queueing songs from commands and background events.
pub fn player(ctx: Context<'_>) -> Player {
    let d = ctx.data();
    Player {
        songbird: d.songbird.clone(),
        http: d.http.clone(),
        discord: ctx.serenity_context().http.clone(),
        state: d.music.clone(),
        volumes: d.volumes.clone(),
        activity: d.last_activity.clone(),
        default_autoplay: d.cfg.autoplay,
    }
}

/// Shuffles in place (Fisher-Yates) with a seed from the OS-randomised hasher.
fn shuffle_slice<T>(items: &mut [T]) {
    use std::hash::{BuildHasher, Hasher};
    let mut x = std::collections::hash_map::RandomState::new()
        .build_hasher()
        .finish()
        | 1;
    for i in (1..items.len()).rev() {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        items.swap(i, (x % (i as u64 + 1)) as usize);
    }
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
#[poise::command(slash_command, guild_only)]
async fn play(
    ctx: Context<'_>,
    #[description = "Link YouTube/Spotify hoặc tên bài hát"] query: String,
    #[description = "Hết hàng chờ thì tự phát bài liên quan (bỏ trống = giữ như cũ)"]
    autoplay: Option<bool>,
) -> Result<(), Error> {
    ctx.defer().await?;
    let data = ctx.data();
    let key = ctx.id();
    data.tracker.stage(key, "vào voice");
    let (guild, call) = call_for_author(ctx).await?;

    data.tracker.stage(key, "tìm bài");
    let tracks = resolve::resolve(&data.http, &data.spotify, &query).await?;
    data.tracker.stage(key, "thêm vào hàng chờ");
    let count = tracks.len();
    let first_title = tracks[0].title.clone();
    let first_url = tracks[0].url.clone();
    let first_dur = tracks[0].duration;

    let p = player(ctx);
    let auto_on = p.with(guild, |g| {
        g.stopped = false;
        g.channel = Some(ctx.channel_id());
        if let Some(on) = autoplay {
            g.autoplay = on;
        }
        g.autoplay
    });
    let mut handler = call.lock().await;
    let was_empty = handler.queue().is_empty();
    for t in tracks {
        let info = TrackInfo {
            id: next_id(),
            title: t.title,
            url: t.url,
            duration: t.duration,
            requested_by: ctx.author().id,
            auto: false,
        };
        p.enqueue(guild, &mut handler, info, t.source).await;
    }
    let pos = handler.queue().len() - 1;
    drop(handler);
    // With autoplay on, line up a related song behind a lone request.
    let bg = p.clone();
    tokio::spawn(async move { bg.fill(guild, None).await });

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
    let msg = if autoplay.is_some() || auto_on {
        msg.footer(poise::serenity_prelude::CreateEmbedFooter::new(if auto_on {
            "🤖 Tự phát: bật · hết hàng chờ sẽ tự chọn bài liên quan"
        } else {
            "🤖 Tự phát: tắt"
        }))
    } else {
        msg
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

/// Nhảy tới bài số N trong hàng chờ (bỏ qua các bài trước nó)
#[poise::command(slash_command, prefix_command, guild_only)]
async fn skipto(
    ctx: Context<'_>,
    #[description = "Số thứ tự trong /queue"]
    #[min = 1]
    position: u32,
) -> Result<(), Error> {
    let position = position as usize;
    let (guild, call) = existing_call(ctx)?;
    let handler = call.lock().await;
    let removed = handler.queue().modify_queue(|q| {
        if position >= q.len() {
            return None;
        }
        Some(q.drain(1..position).collect::<Vec<_>>())
    });
    let Some(removed) = removed else {
        bail!("Không có bài số {position} trong hàng chờ");
    };
    let p = player(ctx);
    p.with(guild, |g| {
        for t in &removed {
            g.suppress.insert(t.data::<TrackInfo>().id);
        }
    });
    for t in &removed {
        let _ = t.stop();
    }
    handler.queue().skip()?;
    ctx.say(format!("⏩ Nhảy tới bài số {position} nè")).await?;
    Ok(())
}

/// Dừng nhạc và xoá hàng chờ
#[poise::command(slash_command, prefix_command, guild_only)]
async fn stop(ctx: Context<'_>) -> Result<(), Error> {
    let (guild, call) = existing_call(ctx)?;
    player(ctx).with(guild, |g| {
        g.stopped = true;
        g.loop_mode = LoopMode::Off;
    });
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

const PAGE: usize = 10;

/// Xem hàng chờ
#[poise::command(slash_command, prefix_command, guild_only, aliases("q"))]
async fn queue(
    ctx: Context<'_>,
    #[description = "Trang (mỗi trang 10 bài)"]
    #[min = 1]
    page: Option<u32>,
) -> Result<(), Error> {
    let page = page.map(|p| p as usize);
    let (guild, call) = existing_call(ctx)?;
    let list = call.lock().await.queue().current_queue();
    let (mode, auto) = player(ctx).with(guild, |g| (g.loop_mode, g.autoplay));
    let status = format!(
        "Lặp: {} · Tự phát: {}",
        mode.label(),
        if auto { "🤖 bật" } else { "tắt" }
    );
    if list.is_empty() {
        ctx.say(format!("Hàng chờ trống trơn (・・;)\n-# {status}"))
            .await?;
        return Ok(());
    }

    let infos: Vec<_> = list.iter().map(|h| h.data::<TrackInfo>()).collect();
    let line = |info: &TrackInfo| {
        let bot = if info.auto { " 🤖" } else { "" };
        format!("{} `{}`{bot}", info.title, fmt_duration(info.duration))
    };
    let mut lines = vec![format!("▶️ **{}**", line(&infos[0]))];

    let upcoming = infos.len() - 1;
    let pages = upcoming.div_ceil(PAGE).max(1);
    let page = page.unwrap_or(1).clamp(1, pages);
    let from = 1 + (page - 1) * PAGE;
    for (i, info) in infos.iter().enumerate().skip(from).take(PAGE) {
        lines.push(format!("`{i}.` {}", line(info)));
    }
    if upcoming == 0 {
        lines.push("_Không còn bài nào phía sau._".into());
    }

    let total: Duration = infos.iter().filter_map(|i| i.duration).sum();
    let footer = format!(
        "{upcoming} bài chờ · tổng {} · trang {page}/{pages} · {status}",
        fmt_duration(Some(total))
    );
    let e = embed("📜 Hàng chờ", lines.join("\n"))
        .footer(poise::serenity_prelude::CreateEmbedFooter::new(footer));
    ctx.send(CreateReply::default().embed(e)).await?;
    Ok(())
}

/// Xoá một bài khỏi hàng chờ
#[poise::command(slash_command, prefix_command, guild_only, aliases("rm"))]
async fn remove(
    ctx: Context<'_>,
    #[description = "Số thứ tự trong /queue"]
    #[min = 1]
    position: u32,
) -> Result<(), Error> {
    let position = position as usize;
    let (guild, call) = existing_call(ctx)?;
    let removed = call
        .lock()
        .await
        .queue()
        .modify_queue(|q| if position < q.len() { q.remove(position) } else { None });
    let Some(t) = removed else {
        bail!("Không có bài số {position} trong hàng chờ");
    };
    let info = t.data::<TrackInfo>();
    player(ctx).with(guild, |g| g.suppress.insert(info.id));
    let _ = t.stop();
    ctx.say(format!("🗑️ Đã xoá **{}**", info.title)).await?;
    Ok(())
}

/// Đổi vị trí một bài trong hàng chờ
#[poise::command(slash_command, prefix_command, guild_only, rename = "move")]
async fn move_track(
    ctx: Context<'_>,
    #[description = "Bài số mấy"]
    #[min = 1]
    from: u32,
    #[description = "Chuyển tới vị trí"]
    #[min = 1]
    to: u32,
) -> Result<(), Error> {
    let (from, to) = (from as usize, to as usize);
    let (_, call) = existing_call(ctx)?;
    let moved = call.lock().await.queue().modify_queue(|q| {
        if from >= q.len() || to >= q.len() {
            return None;
        }
        let t = q.remove(from)?;
        let title = t.data::<TrackInfo>().title.clone();
        q.insert(to, t);
        Some(title)
    });
    let Some(title) = moved else {
        bail!("Vị trí không hợp lệ, xem số thứ tự bằng /queue nha");
    };
    ctx.say(format!("↕️ Chuyển **{title}** tới vị trí {to}"))
        .await?;
    Ok(())
}

/// Trộn ngẫu nhiên các bài đang chờ
#[poise::command(slash_command, prefix_command, guild_only)]
async fn shuffle(ctx: Context<'_>) -> Result<(), Error> {
    let (_, call) = existing_call(ctx)?;
    let n = call.lock().await.queue().modify_queue(|q| {
        let items = q.make_contiguous();
        if items.len() > 2 {
            shuffle_slice(&mut items[1..]);
        }
        items.len().saturating_sub(1)
    });
    if n < 2 {
        bail!("Cần ít nhất 2 bài đang chờ mới trộn được");
    }
    ctx.say(format!("🔀 Trộn {n} bài rồi nè")).await?;
    Ok(())
}

/// Xoá các bài đang chờ (giữ bài đang phát)
#[poise::command(slash_command, prefix_command, guild_only)]
async fn clear(ctx: Context<'_>) -> Result<(), Error> {
    let (guild, call) = existing_call(ctx)?;
    let removed = call
        .lock()
        .await
        .queue()
        .modify_queue(|q| q.drain(1.min(q.len())..).collect::<Vec<_>>());
    player(ctx).with(guild, |g| {
        for t in &removed {
            g.suppress.insert(t.data::<TrackInfo>().id);
        }
    });
    for t in &removed {
        let _ = t.stop();
    }
    ctx.say(format!("🧹 Xoá {} bài đang chờ", removed.len()))
        .await?;
    Ok(())
}

/// Lặp lại: tắt / bài hiện tại / cả hàng chờ
#[poise::command(slash_command, guild_only, rename = "loop")]
async fn loop_cmd(
    ctx: Context<'_>,
    #[description = "Chế độ lặp"] mode: LoopMode,
) -> Result<(), Error> {
    let (guild, call) = existing_call(ctx)?;
    let current = call.lock().await.queue().current();
    if let Some(cur) = &current {
        let _ = if mode == LoopMode::Track {
            cur.enable_loop()
        } else {
            cur.disable_loop()
        };
    } else if mode == LoopMode::Track {
        bail!("Không có bài nào đang phát để lặp");
    }
    player(ctx).with(guild, |g| g.loop_mode = mode);
    let msg = match mode {
        LoopMode::Off => "➡️ Tắt lặp".to_string(),
        LoopMode::Track => "🔂 Lặp bài hiện tại (dùng /skip để qua bài)".to_string(),
        LoopMode::Queue => "🔁 Lặp cả hàng chờ: bài xong sẽ quay lại cuối hàng".to_string(),
    };
    ctx.say(msg).await?;
    Ok(())
}

/// Tự phát bài liên quan khi hàng chờ sắp hết
#[poise::command(slash_command, prefix_command, guild_only, aliases("ap"))]
async fn autoplay(
    ctx: Context<'_>,
    #[description = "Bật hay tắt (bỏ trống = đổi qua lại)"] on: Option<bool>,
) -> Result<(), Error> {
    let guild = ctx
        .guild_id()
        .context("Lệnh này chỉ dùng trong server thôi nha")?;
    let p = player(ctx);
    let now = p.with(guild, |g| {
        g.autoplay = on.unwrap_or(!g.autoplay);
        g.channel = Some(ctx.channel_id());
        if g.autoplay {
            g.stopped = false;
        }
        g.autoplay
    });
    if now {
        ctx.say("🤖 Bật tự phát: khi hàng chờ sắp hết, Dapan sẽ tự chọn bài liên quan nha")
            .await?;
        tokio::spawn(async move { p.fill(guild, None).await });
    } else {
        ctx.say("🤖 Tắt tự phát").await?;
    }
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
    let by = if info.auto {
        "🤖 tự phát".to_string()
    } else {
        format!("<@{}>", info.requested_by)
    };
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
        .field("Yêu cầu bởi", by, true);
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
    player(ctx).with(guild, |g| {
        g.stopped = true;
        g.loop_mode = LoopMode::Off;
    });
    ctx.data()
        .songbird
        .remove(guild)
        .await
        .context("Dapan đang không ở kênh voice nào")?;
    ctx.say("Bái bai 👋").await?;
    Ok(())
}
