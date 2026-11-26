//! Background jobs: reminders, calendar notifications, morning brief, weather alerts,
//! iCal refresh and idle voice disconnect.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

use chrono::{DateTime, NaiveDate, Timelike, Utc};
use poise::serenity_prelude::{ChannelId, CreateEmbed, CreateMessage, GuildId, Http};
use songbird::Songbird;
use sqlx::SqlitePool;
use tokio::sync::RwLock;

use crate::Data;
use crate::calendar::{self, ical::Occurrence};
use crate::config::Config;
use crate::notify::{PINK, broadcast};
use crate::weather;

#[derive(Clone)]
struct Shared {
    http: Arc<Http>,
    cfg: Arc<Config>,
    client: reqwest::Client,
    db: SqlitePool,
    calendar: Arc<RwLock<Vec<Occurrence>>>,
    songbird: Arc<Songbird>,
    last_activity: Arc<StdMutex<HashMap<GuildId, Instant>>>,
}

pub fn spawn_all(http: Arc<Http>, data: &Data) {
    let s = Shared {
        http,
        cfg: data.cfg.clone(),
        client: data.http.clone(),
        db: data.db.clone(),
        calendar: data.calendar.clone(),
        songbird: data.songbird.clone(),
        last_activity: data.last_activity.clone(),
    };
    tokio::spawn(ical_refresher(s.clone()));
    tokio::spawn(clock(s.clone()));
    tokio::spawn(idle_disconnect(s));
}

async fn ical_refresher(s: Shared) {
    let Some(url) = s.cfg.ical_url.clone() else {
        return;
    };
    let mut tick = tokio::time::interval(Duration::from_secs(300));
    loop {
        tick.tick().await;
        match calendar::ical::fetch(&s.client, &url, s.cfg.tz).await {
            Ok(events) => {
                tracing::info!("iCal: {} upcoming occurrences", events.len());
                *s.calendar.write().await = events;
            }
            Err(e) => tracing::warn!("iCal fetch failed: {e:#}"),
        }
    }
}

/// Runs every 30 s and fires whatever is due.
async fn clock(s: Shared) {
    let mut tick = tokio::time::interval(Duration::from_secs(30));
    let mut sent_events: HashSet<(String, i64, &'static str)> = HashSet::new();
    let mut last_morning: Option<NaiveDate> = None;
    let mut last_alert_hour: Option<(NaiveDate, u32)> = None;
    let mut sent_alerts: HashSet<(NaiveDate, weather::AlertKind)> = HashSet::new();

    loop {
        tick.tick().await;
        let now = Utc::now();
        let local = now.with_timezone(&s.cfg.tz);
        let today = local.date_naive();

        if let Err(e) = due_reminders(&s, now).await {
            tracing::warn!("reminders: {e:#}");
        }
        calendar_events(&s, now, &mut sent_events).await;

        // Morning brief: once a day, within an hour after MORNING_TIME.
        let t = local.time();
        let late = t.signed_duration_since(s.cfg.morning_time);
        if last_morning != Some(today)
            && late >= chrono::Duration::zero()
            && late < chrono::Duration::hours(1)
        {
            last_morning = Some(today);
            morning_brief(&s, today).await;
        }

        // Severe weather: check once per hour.
        if last_alert_hour != Some((today, local.hour())) {
            last_alert_hour = Some((today, local.hour()));
            sent_alerts.retain(|(d, _)| *d == today);
            weather_alerts(
                &s,
                &local.format("%Y-%m-%dT%H").to_string(),
                today,
                &mut sent_alerts,
            )
            .await;
        }

        sent_events.retain(|(_, ts, _)| *ts > now.timestamp() - 86_400);
    }
}

async fn due_reminders(s: &Shared, now: DateTime<Utc>) -> anyhow::Result<()> {
    let rows: Vec<(i64, i64, i64, String)> = sqlx::query_as(
        "SELECT id, user_id, channel_id, text FROM reminders WHERE sent = 0 AND remind_at <= ?",
    )
    .bind(now.timestamp())
    .fetch_all(&s.db)
    .await?;
    for (id, user, channel, text) in rows {
        let msg = CreateMessage::new().content(format!(
            "⏰ <@{user}> ơi, đến giờ rồi nè: **{text}** (｡•̀ᴗ-)✧"
        ));
        if let Err(e) = ChannelId::new(channel as u64)
            .send_message(&s.http, msg)
            .await
        {
            tracing::warn!("reminder #{id} delivery failed: {e}");
        }
        sqlx::query("UPDATE reminders SET sent = 1 WHERE id = ?")
            .bind(id)
            .execute(&s.db)
            .await?;
    }
    Ok(())
}

async fn calendar_events(
    s: &Shared,
    now: DateTime<Utc>,
    sent: &mut HashSet<(String, i64, &'static str)>,
) {
    let before = chrono::Duration::minutes(s.cfg.remind_before_min);
    let due: Vec<(Occurrence, &'static str)> = {
        let events = s.calendar.read().await;
        events
            .iter()
            .filter(|o| !o.all_day)
            .filter_map(|o| {
                let ts = o.start.timestamp();
                if now >= o.start - before
                    && now < o.start
                    && !sent.contains(&(o.uid.clone(), ts, "pre"))
                {
                    Some((o.clone(), "pre"))
                } else if now >= o.start
                    && now < o.start + chrono::Duration::minutes(5)
                    && !sent.contains(&(o.uid.clone(), ts, "start"))
                {
                    Some((o.clone(), "start"))
                } else {
                    None
                }
            })
            .collect()
    };
    for (o, kind) in due {
        sent.insert((o.uid.clone(), o.start.timestamp(), kind));
        // Don't send both when we only noticed the event after it started.
        if kind == "start" {
            sent.insert((o.uid.clone(), o.start.timestamp(), "pre"));
        }
        let ts = o.start.timestamp();
        let title = if kind == "pre" {
            "📅 Sắp đến lịch rồi nè"
        } else {
            "🔔 Đến giờ rồi!"
        };
        let mut e = CreateEmbed::new()
            .title(title)
            .description(format!("**{}**\n<t:{ts}:t> (<t:{ts}:R>)", o.summary))
            .color(PINK);
        if let Some(l) = &o.location {
            e = e.field("📍 Địa điểm", l, false);
        }
        broadcast(&s.http, &s.cfg, e).await;
    }
}

async fn morning_brief(s: &Shared, today: NaiveDate) {
    match weather::forecast(&s.client, s.cfg.home_lat, s.cfg.home_lon, s.cfg.tz.name()).await {
        Ok(f) => {
            let e = weather::forecast_embed(&s.cfg.home_city, &f, "🌅 Chào buổi sáng");
            broadcast(&s.http, &s.cfg, e).await;
        }
        Err(e) => tracing::warn!("morning forecast failed: {e:#}"),
    }
    let events = s.calendar.read().await.clone();
    // Only the owner's own reminders go into the brief, never other users'.
    match calendar::agenda_from(&events, &s.db, s.cfg.tz, today, s.cfg.owner_user_id).await {
        Ok(text) if !text.is_empty() => {
            let e = CreateEmbed::new()
                .title("📅 Lịch hôm nay")
                .description(text)
                .color(PINK);
            broadcast(&s.http, &s.cfg, e).await;
        }
        Ok(_) => {}
        Err(e) => tracing::warn!("morning agenda failed: {e:#}"),
    }
}

async fn weather_alerts(
    s: &Shared,
    hour_prefix: &str,
    today: NaiveDate,
    sent: &mut HashSet<(NaiveDate, weather::AlertKind)>,
) {
    let f =
        match weather::forecast(&s.client, s.cfg.home_lat, s.cfg.home_lon, s.cfg.tz.name()).await {
            Ok(f) => f,
            Err(e) => {
                tracing::warn!("weather alert check failed: {e:#}");
                return;
            }
        };
    for a in weather::find_alerts(&f.hourly, hour_prefix, 3) {
        if !sent.insert((today, a.kind.clone())) {
            continue;
        }
        let e = CreateEmbed::new()
            .title(format!("⚠️ Cảnh báo thời tiết · {}", s.cfg.home_city))
            .description(format!("Khoảng **{}**: {}", a.at, a.detail))
            .color(0xFF6B6B);
        broadcast(&s.http, &s.cfg, e).await;
    }
}

/// Leaves voice after 5 minutes with an empty queue and no speech.
async fn idle_disconnect(s: Shared) {
    const IDLE: Duration = Duration::from_secs(300);
    let mut tick = tokio::time::interval(Duration::from_secs(60));
    loop {
        tick.tick().await;
        let guilds: Vec<(GuildId, Instant)> = s
            .last_activity
            .lock()
            .unwrap()
            .iter()
            .map(|(g, t)| (*g, *t))
            .collect();
        for (guild, last) in guilds {
            let Some(call) = s.songbird.get(guild) else {
                s.last_activity.lock().unwrap().remove(&guild);
                continue;
            };
            let empty = call.lock().await.queue().is_empty();
            if !empty {
                s.last_activity
                    .lock()
                    .unwrap()
                    .insert(guild, Instant::now());
            } else if last.elapsed() > IDLE {
                tracing::info!("leaving idle voice in guild {guild}");
                let _ = s.songbird.remove(guild).await;
                s.last_activity.lock().unwrap().remove(&guild);
            }
        }
    }
}
