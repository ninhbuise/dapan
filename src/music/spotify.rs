//! Minimal Spotify Web API client (client-credentials flow).
//! Spotify audio cannot be streamed by bots, so we only read track names here and
//! let YouTube supply the audio.

use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;
use tokio::sync::Mutex;

use super::resolve::SpotifyKind;

#[derive(Debug, Clone)]
pub struct SpotifyTrack {
    pub title: String,
    pub artist: String,
    pub duration: Option<Duration>,
}

impl SpotifyTrack {
    pub fn search_query(&self) -> String {
        if self.artist.is_empty() {
            format!("{} audio", self.title)
        } else {
            format!("{} - {} audio", self.artist, self.title)
        }
    }
}

pub struct Spotify {
    http: reqwest::Client,
    creds: Option<(String, String)>,
    token: Mutex<Option<(String, Instant)>>,
}

#[derive(Deserialize)]
struct TokenResp {
    access_token: String,
    expires_in: u64,
}

#[derive(Deserialize)]
struct Artist {
    name: String,
}

#[derive(Deserialize)]
struct Track {
    name: String,
    #[serde(default)]
    artists: Vec<Artist>,
    duration_ms: Option<u64>,
}

#[derive(Deserialize)]
struct Page<T> {
    items: Vec<T>,
    next: Option<String>,
}

#[derive(Deserialize)]
struct PlaylistItem {
    #[serde(alias = "item")]
    track: Option<Track>,
}

#[derive(Deserialize)]
struct OEmbed {
    title: String,
}

/// Upper bound on how many tracks we take from one album/playlist.
const MAX_TRACKS: usize = 100;

impl From<Track> for SpotifyTrack {
    fn from(t: Track) -> Self {
        SpotifyTrack {
            title: t.name,
            artist: t
                .artists
                .into_iter()
                .map(|a| a.name)
                .collect::<Vec<_>>()
                .join(", "),
            duration: t.duration_ms.map(Duration::from_millis),
        }
    }
}

impl Spotify {
    pub fn new(http: reqwest::Client, id: Option<String>, secret: Option<String>) -> Self {
        Self {
            http,
            creds: id.zip(secret),
            token: Mutex::new(None),
        }
    }

    async fn token(&self) -> Result<String> {
        let Some((id, secret)) = &self.creds else {
            bail!("chưa cấu hình SPOTIFY_CLIENT_ID / SPOTIFY_CLIENT_SECRET");
        };
        let mut guard = self.token.lock().await;
        if let Some((tok, exp)) = guard.as_ref()
            && Instant::now() < *exp
        {
            return Ok(tok.clone());
        }
        let resp: TokenResp = self
            .http
            .post("https://accounts.spotify.com/api/token")
            .basic_auth(id, Some(secret))
            .form(&[("grant_type", "client_credentials")])
            .send()
            .await?
            .error_for_status()
            .context("Spotify từ chối client id/secret")?
            .json()
            .await?;
        let exp = Instant::now() + Duration::from_secs(resp.expires_in.saturating_sub(60));
        *guard = Some((resp.access_token.clone(), exp));
        Ok(resp.access_token)
    }

    async fn get<T: for<'de> Deserialize<'de>>(&self, url: &str) -> Result<T> {
        let token = self.token().await?;
        Ok(self
            .http
            .get(url)
            .bearer_auth(token)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?)
    }

    /// Track name via the public oEmbed endpoint (works without credentials).
    async fn oembed(&self, kind: SpotifyKind, id: &str) -> Result<SpotifyTrack> {
        let url = format!("https://open.spotify.com/{}/{id}", kind.as_str());
        let o: OEmbed = self
            .http
            .get("https://open.spotify.com/oembed")
            .query(&[("url", url.as_str())])
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(SpotifyTrack {
            title: o.title,
            artist: String::new(),
            duration: None,
        })
    }

    pub async fn tracks(&self, kind: SpotifyKind, id: &str) -> Result<Vec<SpotifyTrack>> {
        match kind {
            SpotifyKind::Track => {
                match self
                    .get::<Track>(&format!("https://api.spotify.com/v1/tracks/{id}"))
                    .await
                {
                    Ok(t) => Ok(vec![t.into()]),
                    Err(e) => {
                        tracing::info!("Spotify API failed ({e}); falling back to oEmbed");
                        Ok(vec![self.oembed(kind, id).await?])
                    }
                }
            }
            SpotifyKind::Album => {
                let first = format!("https://api.spotify.com/v1/albums/{id}/tracks?limit=50");
                self.paged::<Track>(first, Some).await
            }
            SpotifyKind::Playlist => {
                let tracks_url =
                    format!("https://api.spotify.com/v1/playlists/{id}/tracks?limit=100");
                match self.paged::<PlaylistItem>(tracks_url, |i| i.track).await {
                    Ok(v) => Ok(v),
                    // Newer API versions renamed the endpoint to /items.
                    Err(_) => {
                        let items_url =
                            format!("https://api.spotify.com/v1/playlists/{id}/items?limit=100");
                        self.paged::<PlaylistItem>(items_url, |i| i.track)
                            .await
                            .context(
                                "không đọc được playlist (playlist riêng tư hoặc do Spotify tạo?)",
                            )
                    }
                }
            }
        }
    }

    async fn paged<I: for<'de> Deserialize<'de>>(
        &self,
        first: String,
        pick: impl Fn(I) -> Option<Track>,
    ) -> Result<Vec<SpotifyTrack>> {
        let mut out = Vec::new();
        let mut next = Some(first);
        while let Some(url) = next.take() {
            let page: Page<I> = self.get(&url).await?;
            out.extend(
                page.items
                    .into_iter()
                    .filter_map(&pick)
                    .map(SpotifyTrack::from),
            );
            if out.len() >= MAX_TRACKS {
                out.truncate(MAX_TRACKS);
                break;
            }
            next = page.next;
        }
        Ok(out)
    }
}
