//! Client for the local VieNeu-TTS server in `tts-server/` (runs on your GPU).

use std::time::Duration;

use anyhow::{Context as _, Result, bail};
use serde::Serialize;

#[derive(Serialize)]
struct Req<'a> {
    text: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    voice: Option<&'a str>,
}

/// Synthesises one chunk of text. Returns WAV bytes.
pub async fn synthesize(
    http: &reqwest::Client,
    base_url: &str,
    voice: Option<&str>,
    text: &str,
) -> Result<Vec<u8>> {
    let resp = http
        .post(format!("{}/tts", base_url.trim_end_matches('/')))
        .json(&Req { text, voice })
        .timeout(Duration::from_secs(60))
        .send()
        .await
        .context("không kết nối được máy chủ giọng nói local (tts-server)")?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        bail!("tts-server lỗi {status}: {body}");
    }
    let bytes = resp.bytes().await?.to_vec();
    if bytes.len() < 100 {
        bail!("tts-server trả về âm thanh rỗng");
    }
    Ok(bytes)
}
