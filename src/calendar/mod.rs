pub mod ical;
pub mod time_parse;

use anyhow::Context as _;
use chrono::{DateTime, Duration, NaiveDate, TimeZone, Utc};
use chrono_tz::Tz;
use poise::CreateReply;
use poise::serenity_prelude::CreateEmbed;
use sqlx::SqlitePool;

use crate::notify::PINK;
use crate::{Context, Data, Error};

pub fn commands() -> Vec<poise::Command<Data, Error>> {
    vec![remind(), reminders(), remind_delete(), calendar()]
}

/// Nhắc bạn một việc
#[poise::command(slash_command, prefix_command)]
async fn remind(
    ctx: Context<'_>,
    #[description = "Khi nào? vd: 10m, 2h, 14:30, mai 8:00, 25/12 09:00"] when: String,
    #[description = "Nhắc việc gì?"]
    #[rest]
    text: String,
) -> Result<(), Error> {
    let tz = ctx.data().cfg.tz;
    let at = time_parse::parse_when(&when, Utc::now().with_timezone(&tz))?;
    let res = sqlx::query(
        "INSERT INTO reminders (user_id, channel_id, text, remind_at) VALUES (?, ?, ?, ?)",
    )
    .bind(ctx.author().id.get() as i64)
    .bind(ctx.channel_id().get() as i64)
    .bind(&text)
    .bind(at.timestamp())
    .execute(&ctx.data().db)
    .await?;
    ctx.say(format!(
        "Okiee! Dapan sẽ nhắc bạn **{text}** lúc <t:{ts}:F> (<t:{ts}:R>) ⏰ · mã `#{id}`",
        ts = at.timestamp(),
        id = res.last_insert_rowid()
    ))
    .await?;
    Ok(())
}

/// Danh sách lời nhắc của bạn
#[poise::command(slash_command, prefix_command)]
async fn reminders(ctx: Context<'_>) -> Result<(), Error> {
    let rows: Vec<(i64, String, i64)> = sqlx::query_as(
        "SELECT id, text, remind_at FROM reminders WHERE user_id = ? AND sent = 0 ORDER BY remind_at LIMIT 20",
    )
    .bind(ctx.author().id.get() as i64)
    .fetch_all(&ctx.data().db)
    .await?;
    let desc = if rows.is_empty() {
        "Bạn chưa có lời nhắc nào ✨".to_string()
    } else {
        rows.iter()
            .map(|(id, text, at)| format!("`#{id}` <t:{at}:f> — {text}"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let e = CreateEmbed::new()
        .title("⏰ Lời nhắc")
        .description(desc)
        .color(PINK);
    ctx.send(CreateReply::default().embed(e).ephemeral(true))
        .await?;
    Ok(())
}

/// Xoá một lời nhắc
#[poise::command(slash_command, prefix_command, rename = "remind-delete")]
async fn remind_delete(
    ctx: Context<'_>,
    #[description = "Mã lời nhắc (#)"] id: i64,
) -> Result<(), Error> {
    let res = sqlx::query("DELETE FROM reminders WHERE id = ? AND user_id = ?")
        .bind(id)
        .bind(ctx.author().id.get() as i64)
        .execute(&ctx.data().db)
        .await?;
    if res.rows_affected() == 0 {
        ctx.say(format!("Không thấy lời nhắc `#{id}` của bạn"))
            .await?;
    } else {
        ctx.say(format!("Đã xoá lời nhắc `#{id}` 🗑️")).await?;
    }
    Ok(())
}

/// Lịch hôm nay và ngày mai
#[poise::command(slash_command, prefix_command)]
async fn calendar(ctx: Context<'_>) -> Result<(), Error> {
    let data = ctx.data();
    let today = Utc::now().with_timezone(&data.cfg.tz).date_naive();
    let mut e = CreateEmbed::new().title("📅 Lịch của bạn").color(PINK);
    for (label, day) in [("Hôm nay", today), ("Ngày mai", today + Duration::days(1))] {
        let text = agenda(data, day).await?;
        e = e.field(
            label,
            if text.is_empty() {
                "Trống, thảnh thơi nè 🍵".into()
            } else {
                text
            },
            false,
        );
    }
    if data.cfg.ical_url.is_none() {
        e = e.footer(poise::serenity_prelude::CreateEmbedFooter::new(
            "Chưa kết nối Google Calendar (ICAL_URL), chỉ hiện lời nhắc /remind",
        ));
    }
    ctx.send(CreateReply::default().embed(e)).await?;
    Ok(())
}

fn day_bounds(tz: Tz, day: NaiveDate) -> anyhow::Result<(DateTime<Utc>, DateTime<Utc>)> {
    let start = tz
        .from_local_datetime(&day.and_hms_opt(0, 0, 0).unwrap())
        .earliest()
        .context("bad date")?
        .with_timezone(&Utc);
    Ok((start, start + Duration::days(1)))
}

/// Calendar events + pending reminders on `day`, one per line.
pub async fn agenda(data: &Data, day: NaiveDate) -> anyhow::Result<String> {
    agenda_from(&data.calendar.read().await, &data.db, data.cfg.tz, day).await
}

pub async fn agenda_from(
    events: &[ical::Occurrence],
    db: &SqlitePool,
    tz: Tz,
    day: NaiveDate,
) -> anyhow::Result<String> {
    let (start, end) = day_bounds(tz, day)?;
    let mut lines: Vec<(i64, String)> = Vec::new();
    for o in events.iter().filter(|o| o.start >= start && o.start < end) {
        let when = if o.all_day {
            "Cả ngày".to_string()
        } else {
            o.start.with_timezone(&tz).format("%H:%M").to_string()
        };
        let loc = o
            .location
            .as_ref()
            .map(|l| format!(" 📍 {l}"))
            .unwrap_or_default();
        lines.push((
            o.start.timestamp(),
            format!("🗓️ `{when}` {}{loc}", o.summary),
        ));
    }
    let rows: Vec<(i64, String, i64)> = sqlx::query_as(
        "SELECT user_id, text, remind_at FROM reminders WHERE sent = 0 AND remind_at >= ? AND remind_at < ?",
    )
    .bind(start.timestamp())
    .bind(end.timestamp())
    .fetch_all(db)
    .await?;
    for (user, text, at) in rows {
        let when = Utc
            .timestamp_opt(at, 0)
            .single()
            .unwrap_or(start)
            .with_timezone(&tz)
            .format("%H:%M");
        lines.push((at, format!("⏰ `{when}` {text} (<@{user}>)")));
    }
    lines.sort_by_key(|l| l.0);
    Ok(lines
        .into_iter()
        .map(|l| l.1)
        .collect::<Vec<_>>()
        .join("\n"))
}
