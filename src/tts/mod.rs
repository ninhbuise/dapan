pub mod edge;

use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use anyhow::{Context as _, Result};
use poise::serenity_prelude::{self as serenity, ChannelId, GuildId, Message};
use songbird::Call;
use songbird::events::{Event, EventContext, EventHandler, TrackEvent};
use songbird::input::Input;
use tokio::sync::{Mutex, oneshot};

use crate::music::{ensure_call, user_voice_channel};
use crate::{Context, Data, Error};

const MAX_CHARS: usize = 300;
/// Music volume multiplier while Dapan is talking.
const DUCK: f32 = 0.3;

pub fn commands() -> Vec<poise::Command<Data, Error>> {
    vec![say(), ttschannel()]
}

/// Fires a oneshot when the TTS track ends (or errors).
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

/// Speaks `text` in the guild's current call, lowering the music while talking.
pub async fn speak(data: &Data, guild: GuildId, call: Arc<Mutex<Call>>, text: &str) -> Result<()> {
    let text: String = text.chars().take(MAX_CHARS).collect();
    let lock = guild_lock(data, guild);
    let _turn = lock.lock().await; // one sentence at a time

    let audio = edge::synthesize(&data.cfg, &text).await?;
    let base_vol = data.volume(guild);

    let (tx, rx) = oneshot::channel();
    let done = Arc::new(StdMutex::new(Some(tx)));
    let music = {
        let mut handler = call.lock().await;
        let music = handler.queue().current();
        if let Some(m) = &music {
            let _ = m.set_volume(base_vol * DUCK);
        }
        let voice = handler.play_input(Input::from(audio));
        let _ = voice.set_volume(1.0);
        voice.add_event(Event::Track(TrackEvent::End), Done(done.clone()))?;
        voice.add_event(Event::Track(TrackEvent::Error), Done(done))?;
        music
    };
    data.touch(guild);

    let _ = tokio::time::timeout(Duration::from_secs(90), rx).await;

    if let Some(m) = music {
        let _ = m.set_volume(data.volume(guild));
    }
    Ok(())
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
    let channel = user_voice_channel(ctx.serenity_context(), guild, ctx.author().id);
    let call = ensure_call(ctx.data(), guild, channel).await?;
    ctx.say(format!("🗣️ {text}")).await?;
    speak(ctx.data(), guild, call, &text).await?;
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
    let call = match ensure_call(data, guild, Some(author_vc)).await {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("auto-tts join failed: {e}");
            return;
        }
    };
    if let Err(e) = speak(data, guild, call, &text).await {
        tracing::warn!("auto-tts failed: {e:#}");
        let _ = msg.react(ctx, '😵').await;
    }
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
}
