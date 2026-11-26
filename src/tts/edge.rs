//! Microsoft Edge "Read aloud" neural voices (free, no API key). Used when the local
//! VieNeu server isn't configured or isn't reachable.
//!
//! The Edge service drops the connection when SSML pitch is set, so the voice is
//! requested at its natural pitch; pitch is applied afterwards by [`super::pitch_shift`].

use std::time::Duration;

use anyhow::{Context as _, Result, bail};
use msedge_tts::tts::SpeechConfig;
use msedge_tts::tts::client::tokio_runtime::connect_async;

use crate::config::Config;

fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c => out.push(c),
        }
    }
    out
}

/// Synthesises one chunk of text. Returns MP3 bytes and the number of attempts used.
///
/// The service occasionally resets or stalls a connection, so each attempt uses a fresh
/// one and is cut off after a while. It produces audio at roughly playback speed, so the
/// timeout leaves room for a full ~200 character chunk.
pub async fn synthesize(cfg: &Config, text: &str) -> Result<(Vec<u8>, u32)> {
    const ATTEMPTS: u32 = 3;
    const PER_ATTEMPT: Duration = Duration::from_secs(30);
    let config = SpeechConfig {
        voice_name: cfg.tts_voice.clone(),
        audio_format: "audio-24khz-48kbitrate-mono-mp3".into(),
        pitch: 0,
        rate: 0,
        volume: 0,
    };
    let ssml_text = xml_escape(text);
    let mut last_err = None;
    for attempt in 1..=ATTEMPTS {
        let result = tokio::time::timeout(PER_ATTEMPT, async {
            let mut client = connect_async()
                .await
                .context("không kết nối được Edge TTS")?;
            let audio = client.synthesize(&ssml_text, &config).await?;
            if audio.audio_bytes.is_empty() {
                bail!("Edge TTS trả về âm thanh rỗng");
            }
            Ok(audio.audio_bytes)
        })
        .await
        .unwrap_or_else(|_| {
            Err(anyhow::anyhow!(
                "Edge TTS quá {}s không trả lời",
                PER_ATTEMPT.as_secs()
            ))
        });
        match result {
            Ok(v) => return Ok((v, attempt)),
            Err(e) => {
                tracing::info!("Edge TTS attempt {attempt}/{ATTEMPTS} failed: {e:#}");
                last_err = Some(e);
                tokio::time::sleep(Duration::from_millis(300 * attempt as u64)).await;
            }
        }
    }
    Err(last_err.unwrap()).context("Edge TTS không phản hồi, thử lại sau nha")
}

#[cfg(test)]
mod tests {
    #[test]
    fn escapes_ssml() {
        assert_eq!(super::xml_escape("a<b & 'c'"), "a&lt;b &amp; &apos;c&apos;");
    }
}

#[cfg(test)]
mod live {
    /// `cargo test -- --ignored` (needs internet)
    #[tokio::test]
    #[ignore]
    async fn hoaimy_speaks_vietnamese() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        unsafe { std::env::set_var("DISCORD_TOKEN", "x") };
        let cfg = crate::config::Config::from_env().unwrap();
        let (mp3, attempts) = super::synthesize(&cfg, "Xin chào, mình là Dapan nè!")
            .await
            .unwrap();
        println!("{} bytes after {attempts} attempt(s)", mp3.len());
        assert!(mp3.len() > 5_000, "audio too short: {} bytes", mp3.len());
    }
}
