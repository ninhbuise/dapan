//! Microsoft Edge "Read aloud" neural voices (free, no API key).
//!
//! The Edge service rejects SSML pitch changes (it drops the connection), so the voice is
//! requested at its natural pitch and made cuter afterwards with FFmpeg.

use std::process::Stdio;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail};
use msedge_tts::tts::SpeechConfig;
use msedge_tts::tts::client::tokio_runtime::connect_async;
use tokio::io::AsyncWriteExt;

use crate::config::Config;

const SAMPLE_RATE: u32 = 24_000;

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

/// FFmpeg filter that shifts pitch by `semitones` and sets overall speed to `speed`.
fn voice_filter(semitones: f32, speed: f32) -> String {
    let f = 2f32.powf(semitones / 12.0);
    // asetrate raises pitch *and* speed by f; atempo then corrects speed to the target.
    format!(
        "asetrate={rate},aresample={SAMPLE_RATE},atempo={tempo:.4}",
        rate = (SAMPLE_RATE as f32 * f).round() as u32,
        tempo = speed / f,
    )
}

/// Result of [`synthesize`], with timings for /tasks and the logs.
pub struct Synth {
    pub mp3: Vec<u8>,
    pub attempts: u32,
    pub edge_time: Duration,
    pub pitch_time: Option<Duration>,
}

/// Synthesises `text` and returns MP3 bytes.
pub async fn synthesize(cfg: &Config, text: &str) -> Result<Synth> {
    let config = SpeechConfig {
        voice_name: cfg.tts_voice.clone(),
        audio_format: "audio-24khz-48kbitrate-mono-mp3".into(),
        pitch: 0,
        rate: 0,
        volume: 0,
    };
    let t = Instant::now();
    let (mp3, attempts) = synth_with_retry(&xml_escape(text), &config).await?;
    let edge_time = t.elapsed();

    if cfg.tts_pitch == 0.0 && (cfg.tts_speed - 1.0).abs() < f32::EPSILON {
        return Ok(Synth {
            mp3,
            attempts,
            edge_time,
            pitch_time: None,
        });
    }
    let t = Instant::now();
    let mp3 = match cutify(&mp3, cfg.tts_pitch, cfg.tts_speed).await {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("ffmpeg pitch shift failed, using the plain voice: {e:#}");
            mp3
        }
    };
    Ok(Synth {
        mp3,
        attempts,
        edge_time,
        pitch_time: Some(t.elapsed()),
    })
}

/// The Edge service occasionally resets or stalls a connection, so each attempt uses a
/// fresh one and is cut off after a few seconds. Returns the audio and attempts used.
async fn synth_with_retry(ssml_text: &str, config: &SpeechConfig) -> Result<(Vec<u8>, u32)> {
    const ATTEMPTS: u32 = 4;
    const PER_ATTEMPT: Duration = Duration::from_secs(8);
    let mut last_err = None;
    for attempt in 1..=ATTEMPTS {
        let result = tokio::time::timeout(PER_ATTEMPT, async {
            let mut client = connect_async()
                .await
                .context("không kết nối được Edge TTS")?;
            let audio = client.synthesize(ssml_text, config).await?;
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
                tokio::time::sleep(Duration::from_millis(250 * attempt as u64)).await;
            }
        }
    }
    Err(last_err.unwrap()).context("Edge TTS không phản hồi, thử lại sau nha")
}

async fn cutify(mp3: &[u8], semitones: f32, speed: f32) -> Result<Vec<u8>> {
    let mut child = tokio::process::Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "mp3",
            "-i",
            "pipe:0",
            "-af",
        ])
        .arg(voice_filter(semitones, speed))
        .args(["-f", "mp3", "-b:a", "64k", "pipe:1"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .context("không chạy được ffmpeg (đã cài và có trong PATH chưa?)")?;

    let mut stdin = child.stdin.take().context("no stdin")?;
    let input = mp3.to_vec();
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

#[cfg(test)]
mod tests {
    #[test]
    fn escapes_ssml() {
        assert_eq!(super::xml_escape("a<b & 'c'"), "a&lt;b &amp; &apos;c&apos;");
    }

    #[test]
    fn filter_keeps_speed() {
        // +12 semitones doubles the rate, so atempo must halve it back (×1.0 speed).
        assert_eq!(
            super::voice_filter(12.0, 1.0),
            "asetrate=48000,aresample=24000,atempo=0.5000"
        );
    }
}

#[cfg(test)]
mod live {
    /// `cargo test -- --ignored` (needs internet + ffmpeg)
    #[tokio::test]
    #[ignore]
    async fn hoaimy_speaks_vietnamese() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        unsafe { std::env::set_var("DISCORD_TOKEN", "x") };
        let cfg = crate::config::Config::from_env().unwrap();
        let mp3 = super::synthesize(&cfg, "Xin chào, mình là Dapan nè! Hôm nay bạn thế nào?")
            .await
            .unwrap();
        println!(
            "edge {:?} ({} attempts), ffmpeg {:?}",
            mp3.edge_time, mp3.attempts, mp3.pitch_time
        );
        let mp3 = mp3.mp3;
        assert!(mp3.len() > 5_000, "audio too short: {} bytes", mp3.len());
        let path = std::env::temp_dir().join("dapan_tts_test.mp3");
        std::fs::write(&path, &mp3).unwrap();
        println!("saved sample to {}", path.display());
    }
}
