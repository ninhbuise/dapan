//! Turns whatever the user typed into a list of playable tracks.

use std::time::Duration;

use anyhow::{Result, bail};
use songbird::input::{Compose, YoutubeDl};

use super::spotify::Spotify;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpotifyKind {
    Track,
    Album,
    Playlist,
}

impl SpotifyKind {
    pub fn as_str(self) -> &'static str {
        match self {
            SpotifyKind::Track => "track",
            SpotifyKind::Album => "album",
            SpotifyKind::Playlist => "playlist",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Query {
    Spotify(SpotifyKind, String),
    YoutubePlaylist(String),
    Url(String),
    Search(String),
}

/// Classifies user input. Pure function, unit tested below.
pub fn classify(input: &str) -> Query {
    let s = input.trim();

    // spotify:track:ID
    if let Some(rest) = s.strip_prefix("spotify:") {
        let mut parts = rest.split(':');
        if let (Some(kind), Some(id)) = (parts.next(), parts.next())
            && let Some(k) = spotify_kind(kind)
        {
            return Query::Spotify(k, id.to_string());
        }
    }

    if s.starts_with("http://") || s.starts_with("https://") {
        let no_scheme = s.split_once("://").map(|x| x.1).unwrap_or(s);
        let (host, path_q) = no_scheme.split_once('/').unwrap_or((no_scheme, ""));
        let path = path_q.split(['?', '#']).next().unwrap_or("");

        if host.ends_with("spotify.com") {
            // open.spotify.com/(intl-vi/)?track/ID
            let segs: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
            for w in segs.windows(2) {
                if let Some(k) = spotify_kind(w[0]) {
                    return Query::Spotify(k, w[1].to_string());
                }
            }
        }

        let is_youtube = host.ends_with("youtube.com") || host.ends_with("youtu.be");
        if is_youtube && path.trim_end_matches('/') == "playlist" {
            return Query::YoutubePlaylist(s.to_string());
        }
        if is_youtube {
            // Drop "&list=..." so a watch link plays just that song.
            return Query::Url(strip_param(s, "list"));
        }
        return Query::Url(s.to_string());
    }

    Query::Search(s.to_string())
}

fn spotify_kind(s: &str) -> Option<SpotifyKind> {
    match s {
        "track" => Some(SpotifyKind::Track),
        "album" => Some(SpotifyKind::Album),
        "playlist" => Some(SpotifyKind::Playlist),
        _ => None,
    }
}

fn strip_param(url: &str, key: &str) -> String {
    let Some((base, query)) = url.split_once('?') else {
        return url.to_string();
    };
    let kept: Vec<&str> = query
        .split('&')
        .filter(|kv| kv.split('=').next() != Some(key))
        .collect();
    if kept.is_empty() {
        base.to_string()
    } else {
        format!("{base}?{}", kept.join("&"))
    }
}

/// A track ready to be queued. `source` is created lazily by songbird when it is its turn.
pub struct Resolved {
    pub title: String,
    pub url: Option<String>,
    pub duration: Option<Duration>,
    pub source: YoutubeDl<'static>,
}

pub async fn resolve(
    http: &reqwest::Client,
    spotify: &Spotify,
    input: &str,
) -> Result<Vec<Resolved>> {
    match classify(input) {
        Query::Spotify(kind, id) => {
            let tracks = spotify.tracks(kind, &id).await?;
            if tracks.is_empty() {
                bail!("không tìm thấy bài nào trong link Spotify này");
            }
            Ok(tracks
                .into_iter()
                .map(|t| Resolved {
                    title: if t.artist.is_empty() {
                        t.title.clone()
                    } else {
                        format!("{} - {}", t.artist, t.title)
                    },
                    url: None,
                    duration: t.duration,
                    source: YoutubeDl::new_search(http.clone(), t.search_query()),
                })
                .collect())
        }
        Query::YoutubePlaylist(url) => youtube_playlist(http, &url).await,
        Query::Url(url) => {
            let mut src = YoutubeDl::new(http.clone(), url.clone());
            let meta = src.aux_metadata().await?;
            Ok(vec![Resolved {
                title: meta.title.unwrap_or_else(|| url.clone()),
                url: Some(meta.source_url.unwrap_or(url)),
                duration: meta.duration,
                source: src,
            }])
        }
        Query::Search(q) => {
            let mut search = YoutubeDl::new_search(http.clone(), q.clone());
            let meta = search
                .search(Some(1))
                .await?
                .next()
                .ok_or_else(|| anyhow::anyhow!("không tìm thấy bài nào cho \"{q}\""))?;
            let url = meta.source_url.clone().unwrap_or_default();
            Ok(vec![Resolved {
                title: meta.title.clone().unwrap_or(q),
                url: Some(url.clone()),
                duration: meta.duration,
                source: YoutubeDl::new(http.clone(), url),
            }])
        }
    }
}

/// Lists a YouTube playlist with `yt-dlp --flat-playlist` (fast, no per-video lookups).
async fn youtube_playlist(http: &reqwest::Client, url: &str) -> Result<Vec<Resolved>> {
    let out = tokio::process::Command::new("yt-dlp")
        .args([
            "--flat-playlist",
            "--playlist-end",
            "100",
            "--print",
            "%(url)s\t%(title)s\t%(duration)s",
            url,
        ])
        .output()
        .await?;
    if !out.status.success() {
        bail!("yt-dlp lỗi: {}", String::from_utf8_lossy(&out.stderr));
    }
    let list: Vec<Resolved> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|line| {
            let mut p = line.split('\t');
            let u = p.next()?.to_string();
            let title = p.next().unwrap_or("?").to_string();
            let duration = p
                .next()
                .and_then(|d| d.parse::<f64>().ok())
                .map(Duration::from_secs_f64);
            Some(Resolved {
                title,
                url: Some(u.clone()),
                duration,
                source: YoutubeDl::new(http.clone(), u),
            })
        })
        .collect();
    if list.is_empty() {
        bail!("playlist trống hoặc riêng tư");
    }
    Ok(list)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spotify_links() {
        assert_eq!(
            classify("https://open.spotify.com/track/4uLU6hMCjMI75M1A2tKUQC?si=abc"),
            Query::Spotify(SpotifyKind::Track, "4uLU6hMCjMI75M1A2tKUQC".into())
        );
        assert_eq!(
            classify("https://open.spotify.com/intl-vi/playlist/37i9dQZF1DXcBWIGoYBM5M"),
            Query::Spotify(SpotifyKind::Playlist, "37i9dQZF1DXcBWIGoYBM5M".into())
        );
        assert_eq!(
            classify("spotify:album:abc"),
            Query::Spotify(SpotifyKind::Album, "abc".into())
        );
    }

    #[test]
    fn youtube_links() {
        assert_eq!(
            classify("https://www.youtube.com/watch?v=dQw4w9WgXcQ&list=PL123&index=2"),
            Query::Url("https://www.youtube.com/watch?v=dQw4w9WgXcQ&index=2".into())
        );
        assert_eq!(
            classify("https://youtube.com/playlist?list=PL123"),
            Query::YoutubePlaylist("https://youtube.com/playlist?list=PL123".into())
        );
        assert_eq!(
            classify("https://youtu.be/dQw4w9WgXcQ"),
            Query::Url("https://youtu.be/dQw4w9WgXcQ".into())
        );
    }

    #[test]
    fn plain_search() {
        assert_eq!(
            classify("  sơn tùng mtp  "),
            Query::Search("sơn tùng mtp".into())
        );
    }
}
