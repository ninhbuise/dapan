//! Weather from Open-Meteo (free, no API key).

use anyhow::{Context as _, Result};
use poise::CreateReply;
use poise::serenity_prelude::{CreateEmbed, CreateEmbedFooter};
use serde::Deserialize;

use crate::notify::PINK;
use crate::{Context, Data, Error};

pub fn commands() -> Vec<poise::Command<Data, Error>> {
    vec![weather()]
}

#[derive(Debug, Clone)]
pub struct Place {
    pub name: String,
    pub lat: f64,
    pub lon: f64,
}

#[derive(Deserialize)]
struct GeoResp {
    results: Option<Vec<GeoItem>>,
}
#[derive(Deserialize)]
struct GeoItem {
    name: String,
    latitude: f64,
    longitude: f64,
    admin1: Option<String>,
    country: Option<String>,
}

pub async fn geocode(http: &reqwest::Client, city: &str) -> Result<Place> {
    let resp: GeoResp = http
        .get("https://geocoding-api.open-meteo.com/v1/search")
        .query(&[
            ("name", city),
            ("count", "1"),
            ("language", "vi"),
            ("format", "json"),
        ])
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let g = resp
        .results
        .and_then(|r| r.into_iter().next())
        .with_context(|| format!("Không tìm thấy địa điểm \"{city}\""))?;
    let mut name = g.name;
    for extra in [g.admin1, g.country].into_iter().flatten() {
        if !name.contains(&extra) {
            name.push_str(", ");
            name.push_str(&extra);
        }
    }
    Ok(Place {
        name,
        lat: g.latitude,
        lon: g.longitude,
    })
}

#[derive(Debug, Deserialize)]
pub struct Forecast {
    pub current: Current,
    pub hourly: Hourly,
    pub daily: Daily,
}

#[derive(Debug, Deserialize)]
pub struct Current {
    pub temperature_2m: f64,
    pub apparent_temperature: f64,
    pub relative_humidity_2m: f64,
    pub weather_code: u8,
    pub wind_speed_10m: f64,
}

#[derive(Debug, Deserialize)]
pub struct Hourly {
    pub time: Vec<String>,
    pub temperature_2m: Vec<f64>,
    pub precipitation: Vec<f64>,
    pub weather_code: Vec<u8>,
}

#[derive(Debug, Deserialize)]
pub struct Daily {
    pub time: Vec<String>,
    pub weather_code: Vec<u8>,
    pub temperature_2m_max: Vec<f64>,
    pub temperature_2m_min: Vec<f64>,
    pub precipitation_sum: Vec<f64>,
    pub precipitation_probability_max: Vec<Option<f64>>,
    pub uv_index_max: Vec<Option<f64>>,
}

pub async fn forecast(http: &reqwest::Client, lat: f64, lon: f64, tz: &str) -> Result<Forecast> {
    Ok(http
        .get("https://api.open-meteo.com/v1/forecast")
        .query(&[
            ("latitude", lat.to_string()),
            ("longitude", lon.to_string()),
            ("current", "temperature_2m,apparent_temperature,relative_humidity_2m,weather_code,wind_speed_10m".into()),
            ("hourly", "temperature_2m,precipitation,weather_code".into()),
            ("daily", "weather_code,temperature_2m_max,temperature_2m_min,precipitation_sum,precipitation_probability_max,uv_index_max".into()),
            ("timezone", tz.to_string()),
            ("forecast_days", "2".into()),
        ])
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?)
}

/// WMO weather code → (emoji, Vietnamese description).
pub fn describe(code: u8) -> (&'static str, &'static str) {
    match code {
        0 => ("☀️", "Trời quang"),
        1 => ("🌤️", "Ít mây"),
        2 => ("⛅", "Có mây"),
        3 => ("☁️", "Nhiều mây"),
        45 | 48 => ("🌫️", "Sương mù"),
        51 | 53 | 55 => ("🌦️", "Mưa phùn"),
        56 | 57 => ("🌧️", "Mưa phùn lạnh"),
        61 => ("🌦️", "Mưa nhỏ"),
        63 => ("🌧️", "Mưa vừa"),
        65 => ("🌧️", "Mưa to"),
        66 | 67 => ("🌧️", "Mưa lạnh"),
        71 | 73 | 75 | 77 => ("🌨️", "Tuyết"),
        80 => ("🌦️", "Mưa rào nhẹ"),
        81 => ("🌧️", "Mưa rào"),
        82 => ("⛈️", "Mưa rào rất to"),
        85 | 86 => ("🌨️", "Mưa tuyết"),
        95 => ("⛈️", "Dông"),
        96 | 99 => ("⛈️", "Dông kèm mưa đá"),
        _ => ("🌈", "Không rõ"),
    }
}

/// A cute tip for the day based on the forecast.
fn tip(max: f64, rain_prob: f64, uv: f64) -> &'static str {
    if rain_prob >= 60.0 {
        "Nhớ mang ô nha, kẻo ướt hết đó ☔"
    } else if max >= 35.0 {
        "Nóng lắm luôn, uống nhiều nước nha 🧋"
    } else if uv >= 8.0 {
        "Nắng gắt, bôi kem chống nắng nha 🧴"
    } else if max <= 15.0 {
        "Lạnh đó, mặc ấm vào nha 🧣"
    } else {
        "Trời dễ chịu, chúc bạn một ngày vui vẻ 💖"
    }
}

/// Builds the forecast embed for day `day` (0 = today, 1 = tomorrow).
pub fn forecast_embed(place: &str, f: &Forecast, title: &str) -> CreateEmbed {
    let (emoji, desc) = describe(f.current.weather_code);
    let d = &f.daily;
    let rain_prob = d
        .precipitation_probability_max
        .first()
        .copied()
        .flatten()
        .unwrap_or(0.0);
    let uv = d.uv_index_max.first().copied().flatten().unwrap_or(0.0);
    let max = d
        .temperature_2m_max
        .first()
        .copied()
        .unwrap_or(f.current.temperature_2m);
    let min = d
        .temperature_2m_min
        .first()
        .copied()
        .unwrap_or(f.current.temperature_2m);

    let mut e = CreateEmbed::new()
        .title(format!("{title} · {place}"))
        .description(format!(
            "{emoji} **{desc}**, {:.0}°C (cảm giác như {:.0}°C)\n{}",
            f.current.temperature_2m,
            f.current.apparent_temperature,
            tip(max, rain_prob, uv)
        ))
        .color(PINK)
        .field("🌡️ Hôm nay", format!("{min:.0}° – {max:.0}°C"), true)
        .field(
            "💧 Độ ẩm",
            format!("{:.0}%", f.current.relative_humidity_2m),
            true,
        )
        .field(
            "🌬️ Gió",
            format!("{:.0} km/h", f.current.wind_speed_10m),
            true,
        )
        .field(
            "☔ Mưa",
            format!(
                "{:.0}% · {:.1} mm",
                rain_prob,
                d.precipitation_sum.first().copied().unwrap_or(0.0)
            ),
            true,
        )
        .field("😎 UV", format!("{uv:.0}"), true);

    if d.time.len() > 1 {
        let (te, td) = describe(d.weather_code[1]);
        e = e.field(
            "📅 Ngày mai",
            format!(
                "{te} {td}, {:.0}° – {:.0}°C",
                d.temperature_2m_min[1], d.temperature_2m_max[1]
            ),
            false,
        );
    }
    e.footer(CreateEmbedFooter::new("Dữ liệu: Open-Meteo"))
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum AlertKind {
    HeavyRain,
    Storm,
    Heat,
}

#[derive(Debug, Clone)]
pub struct Alert {
    pub kind: AlertKind,
    /// "HH:00" when it is expected.
    pub at: String,
    pub detail: String,
}

/// Thresholds for severe weather alerts.
pub const HEAVY_RAIN_MM: f64 = 10.0;
pub const HEAT_C: f64 = 37.0;

/// Looks at the next `hours` hourly slots starting at `now_prefix` ("2026-11-26T14").
pub fn find_alerts(h: &Hourly, now_prefix: &str, hours: usize) -> Vec<Alert> {
    let start = h
        .time
        .iter()
        .position(|t| t.starts_with(now_prefix))
        .unwrap_or(0);
    let mut out: Vec<Alert> = Vec::new();
    for i in start..(start + hours).min(h.time.len()) {
        let at = h.time[i].get(11..16).unwrap_or("?").to_string();
        let mut push = |kind: AlertKind, detail: String| {
            if !out.iter().any(|a| a.kind == kind) {
                out.push(Alert {
                    kind,
                    at: at.clone(),
                    detail,
                });
            }
        };
        if (95..=99).contains(&h.weather_code[i]) {
            push(
                AlertKind::Storm,
                "Có dông/sét, hạn chế ra ngoài nha ⛈️".into(),
            );
        }
        if h.precipitation[i] >= HEAVY_RAIN_MM {
            push(
                AlertKind::HeavyRain,
                format!(
                    "Mưa lớn khoảng {:.0} mm/giờ, cẩn thận ngập nha 🌧️",
                    h.precipitation[i]
                ),
            );
        }
        if h.temperature_2m[i] >= HEAT_C {
            push(
                AlertKind::Heat,
                format!(
                    "Nắng nóng {:.0}°C, tránh ra đường buổi trưa nha 🥵",
                    h.temperature_2m[i]
                ),
            );
        }
    }
    out
}

/// Xem thời tiết
#[poise::command(slash_command, prefix_command)]
async fn weather(
    ctx: Context<'_>,
    #[description = "Thành phố (bỏ trống = nhà)"]
    #[rest]
    city: Option<String>,
) -> Result<(), Error> {
    ctx.defer().await?;
    let data = ctx.data();
    let place = match city {
        Some(c) if !c.trim().is_empty() => geocode(&data.http, c.trim()).await?,
        _ => Place {
            name: data.cfg.home_city.clone(),
            lat: data.cfg.home_lat,
            lon: data.cfg.home_lon,
        },
    };
    let f = forecast(&data.http, place.lat, place.lon, data.cfg.tz.name()).await?;
    ctx.send(CreateReply::default().embed(forecast_embed(&place.name, &f, "Thời tiết")))
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hourly() -> Hourly {
        Hourly {
            time: (0..6)
                .map(|h| format!("2026-11-26T{:02}:00", 12 + h))
                .collect(),
            temperature_2m: vec![30.0, 36.0, 38.0, 33.0, 30.0, 29.0],
            precipitation: vec![0.0, 0.0, 0.0, 12.0, 15.0, 0.0],
            weather_code: vec![1, 2, 3, 63, 95, 3],
        }
    }

    #[test]
    fn detects_each_alert_once() {
        let a = find_alerts(&hourly(), "2026-11-26T13", 5);
        let kinds: Vec<_> = a.iter().map(|x| x.kind.clone()).collect();
        assert_eq!(
            kinds,
            vec![AlertKind::Heat, AlertKind::HeavyRain, AlertKind::Storm]
        );
        assert_eq!(a[0].at, "14:00");
    }

    #[test]
    fn window_limits_lookahead() {
        assert!(find_alerts(&hourly(), "2026-11-26T12", 2).is_empty());
    }
}

#[cfg(test)]
mod live {
    #[tokio::test]
    #[ignore]
    async fn open_meteo_roundtrip() {
        let http = reqwest::Client::new();
        let p = super::geocode(&http, "Đà Nẵng").await.unwrap();
        let f = super::forecast(&http, p.lat, p.lon, "Asia/Ho_Chi_Minh")
            .await
            .unwrap();
        assert!(!f.hourly.time.is_empty());
        println!(
            "{} → {:.1}°C {:?}",
            p.name,
            f.current.temperature_2m,
            super::describe(f.current.weather_code)
        );
    }
}
