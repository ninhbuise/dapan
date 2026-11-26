pub mod edge;
pub mod local;
pub mod split;

use std::process::Stdio;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail};
use poise::serenity_prelude::{self as serenity, ChannelId, GuildId, Message};
use songbird::Call;
use songbird::events::{Event, EventContext, EventHandler, TrackEvent};
use songbird::input::Input;
use tokio::io::AsyncWriteExt;
use tokio::sync::{Mutex, mpsc, oneshot};

use crate::config::Config;
use crate::music::{ensure_call, user_voice_channel};
use crate::tracker::secs;
use crate::{Context, Data, Error};

/// Longest text read in one go (Discord messages are at most 2000 characters).
const MAX_CHARS: usize = 2000;
/// Music volume multiplier while Dapan is talking.
const DUCK: f32 = 0.3;
/// How many chunks may be synthesised ahead of the one playing.
const PREFETCH: usize = 2;

pub fn commands() -> Vec<poise::Command<Data, Error>> {
    vec![say(), ttschannel()]
}

/// Fires a oneshot when a TTS track ends (or errors).
struct Done(Arc<StdMutex<Option<oneshot::Sender<()>>>>);

#[async_trait::async_trait]
impl EventHandler for Done {
    async fn act(&self, _ctx: &EventContext<'_>) -> Option<Event> {
        if let Some(tx) = self.0.lock().unwrap().take() {
            let _ = tx.send(());
        }
        None
    }
}

fn guild_lock(data: &Data, guild: GuildId) -> Arc<Mutex<()>> {
    data.tts_locks
        .lock()
        .unwrap()
        .entry(guild)
        .or_default()
        .clone()
}

/// FFmpeg filter that shifts pitch by `semitones` and sets overall speed to `speed`.
fn voice_filter(semitones: f32, speed: f32, sample_rate: u32) -> String {
    let f = 2f32.powf(semitones / 12.0);
    // asetrate raises pitch *and* speed by f; atempo then corrects speed to the target.
    format!(
        "asetrate={rate},aresample={sample_rate},atempo={tempo:.4}",
        rate = (sample_rate as f32 * f).round() as u32,
        tempo = speed / f,
    )
}

/// Pitches/speeds up audio (any format FFmpeg reads) and returns MP3.
async fn pitch_shift(
    audio: &[u8],
    semitones: f32,
    speed: f32,
    sample_rate: u32,
) -> Result<Vec<u8>> {
    let mut child = tokio::process::Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-i", "pipe:0", "-af"])
        .arg(voice_filter(semitones, speed, sample_rate))
        .args(["-f", "mp3", "-b:a", "96k", "pipe:1"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .context("không chạy được ffmpeg (đã cài và có trong PATH chưa?)")?;

    let mut stdin = child.stdin.take().context("no stdin")?;
    let input = audio.to_vec();
    let writer = tokio::spawn(async move {
        let _ = stdin.write_all(&input).await;
        // dropping stdin closes the pipe so ffmpeg finishes
    });
    let out = child.wait_with_output().await?;
    let _ = writer.await;
    if !out.status.success() || out.stdout.is_empty() {
        bail!("ffmpeg: {}", String::from_utf8_lossy(&out.stderr));
    }
    Ok(out.stdout)
}

/// Synthesises one chunk with the local server (if configured) or Edge, then applies
/// pitch/speed. Returns the audio and a short timing summary for /tasks.
async fn synthesize(cfg: &Config, http: &reqwest::Client, text: &str) -> Result<(Vec<u8>, String)> {
    let t = Instant::now();
    let mut note = String::new();
    let mut local_result = None;
    if let Some(url) = &cfg.tts_local_url {
        match local::synthesize(http, url, cfg.tts_local_voice.as_deref(), text).await {
            Ok(wav) => local_result = Some(wav),
            Err(e) => {
                tracing::warn!("local TTS failed, falling back to Edge: {e:#}");
                note.push_str("local lỗi → ");
            }
        }
    }
    let (audio, rate) = match local_result {
        Some(wav) => {
            note.push_str(&format!("local {}", secs(t.elapsed())));
            (wav, 48_000)
        }
        None => {
            let t = Instant::now();
            let (mp3, attempts) = edge::synthesize(cfg, text).await?;
            note.push_str(&format!("Edge {}", secs(t.elapsed())));
            if attempts > 1 {
                note.push_str(&format!(" ({attempts} lần thử)"));
            }
            (mp3, 24_000)
        }
    };

    if cfg.tts_pitch == 0.0 && (cfg.tts_speed - 1.0).abs() < f32::EPSILON {
        return Ok((audio, note));
    }
    let t = Instant::now();
    match pitch_shift(&audio, cfg.tts_pitch, cfg.tts_speed, rate).await {
        Ok(v) => {
            note.push_str(&format!(" + FFmpeg {}", secs(t.elapsed())));
            Ok((v, note))
        }
        Err(e) => {
            tracing::warn!("ffmpeg pitch shift failed, using the plain voice: {e:#}");
            Ok((audio, note))
        }
    }
}

/// Plays one clip and waits until it ends (or 3 minutes, as a safety net).
async fn play_clip(call: &Arc<Mutex<Call>>, audio: Vec<u8>) -> Result<()> {
    let (tx, rx) = oneshot::channel();
    let done = Arc::new(StdMutex::new(Some(tx)));
    {
        let mut handler = call.lock().await;
        let voice = handler.play_input(Input::from(audio));
        let _ = voice.set_volume(1.0);
        voice.add_event(Event::Track(TrackEvent::End), Done(done.clone()))?;
        voice.add_event(Event::Track(TrackEvent::Error), Done(done))?;
    }
    let _ = tokio::time::timeout(Duration::from_secs(180), rx).await;
    Ok(())
}

/// Speaks `text`: joins voice if needed, splits long text into sentences, synthesises
/// ahead while playing, and lowers the music while talking. Timings go to tracker `key`.
pub async fn speak(
    data: &Data,
    guild: GuildId,
    channel: Option<ChannelId>,
    text: &str,
    key: u64,
) -> Result<()> {
    let text: String = text.chars().take(MAX_CHARS).collect();
    let chunks = split::split(&text);
    let total = chunks.len();
    if total == 0 {
        return Ok(());
    }
    data.tracker.stage(key, "vào voice + tạo giọng");
    if total > 1 {
        data.tracker.note(key, format!("{total} đoạn"));
    }

    // Producer: synthesise chunks in order, at most PREFETCH ahead of playback.
    let (tx, mut rx) = mpsc::channel::<Result<Vec<u8>>>(PREFETCH);
    let producer = async {
        for (i, chunk) in chunks.iter().enumerate() {
            let r = synthesize(&data.cfg, &data.http, chunk).await;
            let failed = r.is_err();
            if let Ok((_, note)) = &r {
                let label = if total > 1 {
                    format!("đoạn {}/{total}: ", i + 1)
                } else {
                    String::new()
                };
                data.tracker.note(key, format!("{label}{note}"));
            }
            if tx.send(r.map(|(a, _)| a)).await.is_err() || failed {
                break;
            }
        }
        drop(tx);
    };

    // Consumer: join voice while the first chunk is being made, then play in order.
    let consumer = async {
        let t = Instant::now();
        let (call, first) = tokio::join!(ensure_call(data, guild, channel), rx.recv());
        data.tracker
            .note(key, format!("vào voice {}", secs(t.elapsed())));
        let call = call?;
        let Some(first) = first else { return Ok(()) };
        let first = first?;

        data.tracker.stage(key, "chờ lượt");
        let lock = guild_lock(data, guild);
        let _turn = lock.lock().await; // one speaker at a time
        data.tracker.stage(key, "đang nói");
        if let Some(el) = data.tracker.elapsed(key) {
            data.tracker
                .note(key, format!("⏱ bắt đầu nói sau {}", secs(el)));
        }

        let music = call.lock().await.queue().current();
        if let Some(m) = &music {
            let _ = m.set_volume(data.volume(guild) * DUCK);
        }
        let mut result = Ok(());
        let mut next = Some(Ok(first));
        while let Some(audio) = next {
            match audio {
                Ok(a) => {
                    data.touch(guild);
                    if let Err(e) = play_clip(&call, a).await {
                        result = Err(e);
                        break;
                    }
                }
                Err(e) => {
                    result = Err(e);
                    break;
                }
            }
            next = rx.recv().await;
        }
        if let Some(m) = music {
            let _ = m.set_volume(data.volume(guild));
        }
        rx.close();
        result
    };

    let ((), result) = tokio::join!(producer, consumer);
    result
}

/// Dapan đọc câu bạn nhập trong kênh voice
#[poise::command(slash_command, prefix_command, guild_only)]
async fn say(
    ctx: Context<'_>,
    #[description = "Nội dung (tiếng Việt)"]
    #[rest]
    text: String,
) -> Result<(), Error> {
    let guild = ctx.guild_id().context("guild only")?;
    let data = ctx.data();
    let channel = user_voice_channel(ctx.serenity_context(), guild, ctx.author().id);
    if channel.is_none() && data.songbird.get(guild).is_none() {
        bail!("Bạn vào một kênh voice trước rồi gọi Dapan nha 🥺");
    }
    // Reply right away: Discord gives up on a command after 3 seconds without an answer.
    let shown: String = text.chars().take(1900).collect();
    ctx.say(format!("🗣️ {shown}")).await?;
    speak(data, guild, channel, &text, ctx.id()).await?;
    Ok(())
}

/// Bật/tắt tự đọc tin nhắn trong kênh chat này
#[poise::command(slash_command, guild_only, required_permissions = "MANAGE_CHANNELS")]
async fn ttschannel(
    ctx: Context<'_>,
    #[description = "Bật (true) hoặc tắt (false)"] enable: bool,
) -> Result<(), Error> {
    let guild = ctx.guild_id().context("guild only")?;
    let data = ctx.data();
    let gid = guild.get() as i64;
    if enable {
        let ch = ctx.channel_id();
        sqlx::query("INSERT INTO tts_channels (guild_id, channel_id) VALUES (?, ?) ON CONFLICT(guild_id) DO UPDATE SET channel_id = excluded.channel_id")
            .bind(gid)
            .bind(ch.get() as i64)
            .execute(&data.db)
            .await?;
        data.tts_channels.lock().unwrap().insert(guild, ch);
        ctx.say(format!(
            "Từ giờ Dapan sẽ đọc tin nhắn trong <#{ch}> khi bạn ở kênh voice nha 🎀"
        ))
        .await?;
    } else {
        sqlx::query("DELETE FROM tts_channels WHERE guild_id = ?")
            .bind(gid)
            .execute(&data.db)
            .await?;
        data.tts_channels.lock().unwrap().remove(&guild);
        ctx.say("Đã tắt tự đọc tin nhắn 🤐").await?;
    }
    Ok(())
}

/// Makes a chat message pleasant to hear: drops links, mentions, custom emoji and code.
pub fn clean_for_speech(content: &str) -> String {
    content
        .split_whitespace()
        .filter(|w| !(w.starts_with("http://") || w.starts_with("https://")))
        .filter(|w| !(w.starts_with('<') && w.ends_with('>')))
        .filter(|w| !w.starts_with("```"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Called for every message: reads it aloud if it was posted in the guild's TTS channel.
pub async fn on_message(ctx: &serenity::Context, data: &Data, msg: &Message) {
    if msg.author.bot {
        return;
    }
    let Some(guild) = msg.guild_id else { return };
    let configured: Option<ChannelId> = data.tts_channels.lock().unwrap().get(&guild).copied();
    if configured != Some(msg.channel_id) {
        return;
    }
    // Prefix commands are not read aloud.
    if msg.content.starts_with('/') || msg.content.starts_with('!') {
        return;
    }
    let text = clean_for_speech(&msg.content);
    if text.is_empty() {
        return;
    }
    // Only speak when the author is in voice, so the bot doesn't talk to an empty room.
    let Some(author_vc) = user_voice_channel(ctx, guild, msg.author.id) else {
        return;
    };
    let key = msg.id.get();
    let server = ctx
        .cache
        .guild(guild)
        .map(|g| g.name.clone())
        .unwrap_or_default();
    data.tracker.start(
        key,
        "tự đọc tin nhắn",
        format!("{} @ {server}", msg.author.name),
    );
    let result = speak(data, guild, Some(author_vc), &text, key).await;
    if let Err(e) = &result {
        tracing::warn!("auto-tts failed: {e:#}");
        data.tracker.note(key, format!("lỗi: {e}"));
        let _ = msg.react(ctx, '😵').await;
    }
    data.tracker.finish(key, result.is_ok());
}

#[cfg(test)]
mod tests {
    #[test]
    fn cleans_links_and_mentions() {
        assert_eq!(
            super::clean_for_speech("chào <@123> xem https://x.com nè <:cat:42>"),
            "chào xem nè"
        );
    }

    #[test]
    fn filter_keeps_speed() {
        // +12 semitones doubles the rate, so atempo must halve it back (×1.0 speed).
        assert_eq!(
            super::voice_filter(12.0, 1.0, 24_000),
            "asetrate=48000,aresample=24000,atempo=0.5000"
        );
    }
}

#[cfg(test)]
mod live {
    /// `cargo test local_long_text -- --ignored --nocapture` (needs tts-server running)
    #[tokio::test]
    #[ignore]
    async fn local_long_text() {
        unsafe {
            std::env::set_var("DISCORD_TOKEN", "x");
            std::env::set_var("TTS_LOCAL_URL", "http://127.0.0.1:5005");
            std::env::set_var("TTS_PITCH", "0");
        }
        let cfg = crate::config::Config::from_env().unwrap();
        let http = reqwest::Client::new();
        let text = "Hôm nay trời Hà Nội khá đẹp, nắng nhẹ và gió mát, rất thích hợp để đi dạo quanh hồ Gươm cùng bạn bè. ".repeat(8);
        let chunks = super::split::split(&text);
        let t = std::time::Instant::now();
        let mut total_bytes = 0;
        for (i, c) in chunks.iter().enumerate() {
            let (audio, note) = super::synthesize(&cfg, &http, c).await.unwrap();
            assert!(note.starts_with("local"), "fell back: {note}");
            total_bytes += audio.len();
            println!(
                "chunk {}/{} ({} chars): {note}",
                i + 1,
                chunks.len(),
                c.chars().count()
            );
        }
        println!(
            "total {} chars in {:?}, {} bytes",
            text.chars().count(),
            t.elapsed(),
            total_bytes
        );
    }
}
