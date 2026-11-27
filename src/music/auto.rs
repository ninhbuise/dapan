//! Per-guild music state (loop mode, autoplay, recently played songs) and what happens
//! when a song ends: repeat the queue, or queue a related song from YouTube's "Mix".

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail};
use poise::serenity_prelude::{self as serenity, ChannelId, CreateEmbed, CreateMessage, GuildId};
use songbird::events::{Event, EventContext, EventHandler, TrackEvent};
use songbird::input::YoutubeDl;
use songbird::tracks::{Track, TrackHandle};
use songbird::{Call, Songbird};

use super::{TrackInfo, embed, fmt_duration};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, poise::ChoiceParameter)]
pub enum LoopMode {
    #[default]
    #[name = "Tắt"]
    Off,
    #[name = "Bài hiện tại"]
    Track,
    #[name = "Cả hàng chờ"]
    Queue,
}

impl LoopMode {
    pub fn label(self) -> &'static str {
        match self {
            LoopMode::Off => "tắt",
            LoopMode::Track => "🔂 bài hiện tại",
            LoopMode::Queue => "🔁 cả hàng chờ",
        }
    }
}

#[derive(Default)]
pub struct GuildMusic {
    pub autoplay: bool,
    pub loop_mode: LoopMode,
    /// Set by /stop and /leave so ending tracks don't trigger loop/autoplay.
    pub stopped: bool,
    /// An autoplay lookup is running.
    pub filling: bool,
    /// Tracks removed on purpose (/remove, /clear, /skipto): ignore their end.
    pub suppress: HashSet<u64>,
    /// YouTube ids of recently queued songs, newest last (autoplay avoids repeats).
    pub history: VecDeque<String>,
    /// The last song that finished (seed for autoplay when the queue is empty).
    pub last: Option<Arc<TrackInfo>>,
    /// Where autoplay announces its picks.
    pub channel: Option<ChannelId>,
}

pub type MusicState = Arc<StdMutex<HashMap<GuildId, GuildMusic>>>;

const HISTORY: usize = 100;
/// Skip long compilations / livestream VODs in autoplay picks.
const MAX_AUTO_LEN: Duration = Duration::from_secs(15 * 60);

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

pub fn next_id() -> u64 {
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}

/// Everything needed to queue songs, also from background events (cheap to clone).
#[derive(Clone)]
pub struct Player {
    pub songbird: Arc<Songbird>,
    pub http: reqwest::Client,
    pub discord: Arc<serenity::Http>,
    pub state: MusicState,
    pub volumes: Arc<StdMutex<HashMap<GuildId, f32>>>,
    pub activity: Arc<StdMutex<HashMap<GuildId, Instant>>>,
    /// AUTOPLAY from .env: autoplay state for a server that hasn't used /autoplay.
    pub default_autoplay: bool,
}

impl Player {
    pub fn with<R>(&self, guild: GuildId, f: impl FnOnce(&mut GuildMusic) -> R) -> R {
        let default_autoplay = self.default_autoplay;
        f(self
            .state
            .lock()
            .unwrap()
            .entry(guild)
            .or_insert_with(|| GuildMusic {
                autoplay: default_autoplay,
                ..Default::default()
            }))
    }

    fn volume(&self, guild: GuildId) -> f32 {
        *self.volumes.lock().unwrap().get(&guild).unwrap_or(&0.6)
    }

    /// Queues a song and arranges for loop/autoplay to run when it ends.
    pub async fn enqueue(
        &self,
        guild: GuildId,
        call: &mut Call,
        info: TrackInfo,
        source: YoutubeDl<'static>,
    ) -> TrackHandle {
        if let Some(id) = info.url.as_deref().and_then(video_id) {
            self.with(guild, |g| {
                g.history.retain(|h| h != &id);
                g.history.push_back(id);
                while g.history.len() > HISTORY {
                    g.history.pop_front();
                }
            });
        }
        let info = Arc::new(info);
        let track = Track::new_with_data(source.into(), info.clone()).volume(self.volume(guild));
        let handle = call.enqueue(track).await;
        let _ = handle.add_event(
            Event::Track(TrackEvent::End),
            Ended {
                player: self.clone(),
                guild,
                info,
            },
        );
        self.activity.lock().unwrap().insert(guild, Instant::now());
        handle
    }

    async fn on_end(&self, guild: GuildId, info: Arc<TrackInfo>) {
        // Let songbird's queue drop the finished track and start the next one first.
        tokio::time::sleep(Duration::from_millis(300)).await;
        let (skip, mode) = self.with(guild, |g| {
            if g.suppress.remove(&info.id) || g.stopped {
                return (true, g.loop_mode);
            }
            g.last = Some(info.clone());
            let mode = g.loop_mode;
            if mode == LoopMode::Track {
                // A looping song only ends when skipped: looping is over.
                g.loop_mode = LoopMode::Off;
            }
            (false, mode)
        });
        if skip {
            return;
        }
        if mode == LoopMode::Queue {
            let Some(call) = self.songbird.get(guild) else { return };
            let again = TrackInfo {
                id: next_id(),
                ..(*info).clone()
            };
            let src = source_for(&self.http, &again);
            let mut h = call.lock().await;
            self.enqueue(guild, &mut h, again, src).await;
            return;
        }
        self.fill(guild, Some(info)).await;
    }

    /// Autoplay: when one song or fewer is left, queue a related one. Doing it while the
    /// last song still plays hides the few seconds the lookup takes.
    pub async fn fill(&self, guild: GuildId, fallback: Option<Arc<TrackInfo>>) {
        let go = self.with(guild, |g| {
            if !g.autoplay || g.stopped || g.filling || g.loop_mode == LoopMode::Queue {
                return false;
            }
            g.filling = true;
            true
        });
        if !go {
            return;
        }
        let result = self.fill_inner(guild, fallback).await;
        self.with(guild, |g| g.filling = false);
        if let Err(e) = result {
            tracing::warn!("autoplay: {e:#}");
            self.announce(guild, embed("🤖 Tự phát gặp lỗi", format!("{e:#}")))
                .await;
        }
    }

    async fn fill_inner(&self, guild: GuildId, fallback: Option<Arc<TrackInfo>>) -> Result<()> {
        let Some(call) = self.songbird.get(guild) else {
            return Ok(());
        };
        let seed = {
            let h = call.lock().await;
            if h.current_channel().is_none() {
                return Ok(());
            }
            let q = h.queue().current_queue();
            if q.len() > 1 {
                return Ok(());
            }
            match q.last() {
                Some(t) => t.data::<TrackInfo>(),
                None => match fallback.or_else(|| self.with(guild, |g| g.last.clone())) {
                    Some(f) => f,
                    None => return Ok(()),
                },
            }
        };

        let seed_id = match seed.url.as_deref().and_then(video_id) {
            Some(id) => id,
            None => search_id(&seed.title).await?,
        };
        let history = self.with(guild, |g| g.history.clone());
        let pick = mix(&seed_id)
            .await?
            .into_iter()
            .find(|c| c.id != seed_id && !history.contains(&c.id))
            .with_context(|| format!("hết bài gợi ý mới cho \"{}\"", seed.title))?;

        let mut h = call.lock().await;
        let still_wanted = self.with(guild, |g| g.autoplay && !g.stopped);
        if h.queue().len() > 1 || !still_wanted {
            return Ok(());
        }
        let url = format!("https://www.youtube.com/watch?v={}", pick.id);
        let was_empty = h.queue().is_empty();
        let info = TrackInfo {
            id: next_id(),
            title: pick.title.clone(),
            url: Some(url.clone()),
            duration: pick.duration,
            requested_by: seed.requested_by,
            auto: true,
        };
        let src = YoutubeDl::new(self.http.clone(), url.clone());
        self.enqueue(guild, &mut h, info, src).await;
        drop(h);

        let title = if was_empty {
            "🤖 Tự phát"
        } else {
            "🤖 Bài tiếp theo (tự phát)"
        };
        let desc = format!("[{}]({url}) `{}`", pick.title, fmt_duration(pick.duration));
        self.announce(guild, embed(title, desc)).await;
        Ok(())
    }

    async fn announce(&self, guild: GuildId, e: CreateEmbed) {
        let Some(ch) = self.with(guild, |g| g.channel) else {
            return;
        };
        if let Err(err) = ch
            .send_message(self.discord.as_ref(), CreateMessage::new().embed(e))
            .await
        {
            tracing::warn!("autoplay announce: {err}");
        }
    }
}

struct Ended {
    player: Player,
    guild: GuildId,
    info: Arc<TrackInfo>,
}

#[async_trait::async_trait]
impl EventHandler for Ended {
    async fn act(&self, _ctx: &EventContext<'_>) -> Option<Event> {
        let (p, guild, info) = (self.player.clone(), self.guild, self.info.clone());
        tokio::spawn(async move { p.on_end(guild, info).await });
        None
    }
}

/// A fresh source for a song that was queued before (loop / move back).
pub fn source_for(http: &reqwest::Client, info: &TrackInfo) -> YoutubeDl<'static> {
    match &info.url {
        Some(u) => YoutubeDl::new(http.clone(), u.clone()),
        None => YoutubeDl::new_search(http.clone(), info.title.clone()),
    }
}

/// The 11-character video id of a YouTube link.
pub fn video_id(url: &str) -> Option<String> {
    let rest = url.split_once("://").map(|x| x.1).unwrap_or(url);
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    let id = if host.ends_with("youtu.be") {
        path.split(['?', '#', '/']).next()
    } else if host.ends_with("youtube.com") {
        match ["shorts/", "embed/", "live/"]
            .iter()
            .find_map(|p| path.strip_prefix(p))
        {
            Some(r) => r.split(['?', '#', '/']).next(),
            None => path
                .split_once('?')
                .and_then(|(_, q)| q.split(['&', '#']).find_map(|kv| kv.strip_prefix("v="))),
        }
    } else {
        None
    }?;
    let ok = id.len() == 11
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    ok.then(|| id.to_string())
}

pub struct Candidate {
    pub id: String,
    pub title: String,
    pub duration: Option<Duration>,
}

/// Songs from YouTube's "Mix" for a video (what YouTube itself autoplays next).
async fn mix(seed: &str) -> Result<Vec<Candidate>> {
    let url = format!("https://www.youtube.com/watch?v={seed}&list=RD{seed}");
    let out = yt_dlp(&[
        "--flat-playlist",
        "--playlist-end",
        "30",
        "--print",
        "%(id)s\t%(title)s\t%(duration)s",
        &url,
    ])
    .await?;
    Ok(out
        .lines()
        .filter_map(|line| {
            let mut p = line.split('\t');
            let id = p.next()?.trim().to_string();
            let title = p.next().unwrap_or("?").to_string();
            let duration = p
                .next()
                .and_then(|d| d.trim().parse::<f64>().ok())
                .map(Duration::from_secs_f64);
            Some(Candidate { id, title, duration })
        })
        .filter(|c| c.id.len() == 11 && c.duration.is_none_or(|d| d <= MAX_AUTO_LEN))
        .collect())
}

/// Video id of the first YouTube search result (for Spotify songs, which have no link).
async fn search_id(query: &str) -> Result<String> {
    let q = format!("ytsearch1:{query}");
    let out = yt_dlp(&["--flat-playlist", "--print", "%(id)s", &q]).await?;
    match out.lines().next().map(str::trim) {
        Some(id) if !id.is_empty() => Ok(id.to_string()),
        _ => bail!("không tìm thấy \"{query}\" trên YouTube"),
    }
}

async fn yt_dlp(args: &[&str]) -> Result<String> {
    let run = tokio::process::Command::new("yt-dlp")
        .args(args)
        .kill_on_drop(true)
        .output();
    let out = tokio::time::timeout(Duration::from_secs(30), run)
        .await
        .context("yt-dlp quá 30 giây")??;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        bail!("yt-dlp lỗi: {}", err.lines().last().unwrap_or("").trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::video_id;

    #[test]
    fn video_ids() {
        let id = Some("dQw4w9WgXcQ".to_string());
        assert_eq!(video_id("https://www.youtube.com/watch?v=dQw4w9WgXcQ"), id);
        assert_eq!(video_id("https://youtube.com/watch?list=PL1&v=dQw4w9WgXcQ&t=3"), id);
        assert_eq!(video_id("https://youtu.be/dQw4w9WgXcQ?si=x"), id);
        assert_eq!(video_id("https://www.youtube.com/shorts/dQw4w9WgXcQ"), id);
        assert_eq!(video_id("https://music.youtube.com/watch?v=dQw4w9WgXcQ"), id);
        assert_eq!(video_id("https://example.com/watch?v=dQw4w9WgXcQ"), None);
        assert_eq!(video_id("https://www.youtube.com/playlist?list=PL1"), None);
    }
}
