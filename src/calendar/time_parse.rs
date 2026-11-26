//! Parses the `when` argument of /remind.
//!
//! Accepted forms (all in the bot's timezone):
//! - relative: `10m`, `2h`, `1h30m`, `1d`, also Vietnamese `30p` (phút), `2g`/`2gio` (giờ), `1ngay`
//! - time today (or tomorrow if already passed): `14:30`, `7h30`, `7h`
//! - tomorrow: `mai 8:00`, `tomorrow 8:00`
//! - date: `25/12 08:00`, `25/12/2026 08:00`, `2026-12-25 08:00` (time defaults to 08:00)

use anyhow::{Result, bail};
use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveTime, TimeZone};
use chrono_tz::Tz;

pub fn parse_when(input: &str, now: DateTime<Tz>) -> Result<DateTime<Tz>> {
    let s = input.trim().to_lowercase();
    if s.is_empty() {
        bail!("Bạn chưa nhập thời gian");
    }

    if let Some(d) = parse_relative(&s) {
        if d <= Duration::zero() {
            bail!("Thời gian phải lớn hơn 0");
        }
        return Ok(now + d);
    }

    let tz = now.timezone();
    let parts: Vec<&str> = s.split_whitespace().collect();

    // "mai 8:00" / "tomorrow 8:00"
    if let [day, time] = parts.as_slice()
        && matches!(*day, "mai" | "tomorrow" | "ngaymai" | "ngày-mai")
    {
        let t = parse_time(time).ok_or_else(|| bad(input))?;
        return local(tz, now.date_naive() + Duration::days(1), t);
    }

    match parts.as_slice() {
        [time] if parse_time(time).is_some() => {
            let t = parse_time(time).unwrap();
            let today = local(tz, now.date_naive(), t)?;
            if today > now {
                Ok(today)
            } else {
                local(tz, now.date_naive() + Duration::days(1), t)
            }
        }
        [date] => {
            let d = parse_date(date, now).ok_or_else(|| bad(input))?;
            local(tz, d, NaiveTime::from_hms_opt(8, 0, 0).unwrap())
        }
        [date, time] => {
            let d = parse_date(date, now).ok_or_else(|| bad(input))?;
            let t = parse_time(time).ok_or_else(|| bad(input))?;
            local(tz, d, t)
        }
        _ => Err(bad(input)),
    }
}

fn bad(input: &str) -> anyhow::Error {
    anyhow::anyhow!(
        "Dapan không hiểu \"{input}\" (｡•́︿•̀｡) Thử: `10m`, `2h`, `14:30`, `mai 8:00`, `25/12 09:00`"
    )
}

fn local(tz: Tz, d: NaiveDate, t: NaiveTime) -> Result<DateTime<Tz>> {
    tz.from_local_datetime(&d.and_time(t))
        .earliest()
        .ok_or_else(|| anyhow::anyhow!("thời gian không hợp lệ"))
}

fn parse_relative(s: &str) -> Option<Duration> {
    let s: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    let mut total = Duration::zero();
    let mut num = String::new();
    let mut unit = String::new();

    let mut flush = |num: &mut String, unit: &mut String| -> Option<()> {
        if num.is_empty() {
            return None;
        }
        let n: i64 = num.parse().ok()?;
        let d = match unit.as_str() {
            "m" | "min" | "mins" | "p" | "ph" | "phut" | "phút" => Duration::minutes(n),
            "h" | "hr" | "hrs" | "g" | "gio" | "giờ" | "tieng" | "tiếng" => Duration::hours(n),
            "d" | "day" | "days" | "ngay" | "ngày" => Duration::days(n),
            "w" | "tuan" | "tuần" => Duration::weeks(n),
            _ => return None,
        };
        total += d;
        num.clear();
        unit.clear();
        Some(())
    };

    for c in s.chars() {
        if c.is_ascii_digit() {
            if !unit.is_empty() {
                flush(&mut num, &mut unit)?;
            }
            num.push(c);
        } else {
            unit.push(c);
        }
    }
    // "7h30" is a clock time, not a duration: a trailing number without unit is not relative.
    if unit.is_empty() {
        return None;
    }
    flush(&mut num, &mut unit)?;
    Some(total)
}

fn parse_time(s: &str) -> Option<NaiveTime> {
    // 14:30, 7h30, 7h
    let s = s.replace(['h', 'g'], ":");
    let (h, m) = match s.split_once(':') {
        Some((h, m)) => (h, if m.is_empty() { "0" } else { m }),
        None => return None,
    };
    NaiveTime::from_hms_opt(h.parse().ok()?, m.parse().ok()?, 0)
}

fn parse_date(s: &str, now: DateTime<Tz>) -> Option<NaiveDate> {
    if let Ok(d) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        return Some(d);
    }
    if let Ok(d) = NaiveDate::parse_from_str(s, "%d/%m/%Y") {
        return Some(d);
    }
    // dd/mm → this year, or next year if already passed.
    let (d, m) = s.split_once('/')?;
    let (d, m): (u32, u32) = (d.parse().ok()?, m.parse().ok()?);
    let this_year = NaiveDate::from_ymd_opt(now.year(), m, d)?;
    if this_year >= now.date_naive() {
        Some(this_year)
    } else {
        NaiveDate::from_ymd_opt(now.year() + 1, m, d)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono_tz::Asia::Ho_Chi_Minh as VN;

    fn now() -> DateTime<Tz> {
        VN.with_ymd_and_hms(2026, 11, 26, 10, 0, 0).unwrap()
    }

    #[test]
    fn relative() {
        assert_eq!(
            parse_when("10m", now()).unwrap(),
            now() + Duration::minutes(10)
        );
        assert_eq!(
            parse_when("1h30m", now()).unwrap(),
            now() + Duration::minutes(90)
        );
        assert_eq!(
            parse_when("2 giờ", now()).unwrap(),
            now() + Duration::hours(2)
        );
        assert_eq!(
            parse_when("30p", now()).unwrap(),
            now() + Duration::minutes(30)
        );
    }

    #[test]
    fn clock_times() {
        assert_eq!(
            parse_when("14:30", now()).unwrap(),
            VN.with_ymd_and_hms(2026, 11, 26, 14, 30, 0).unwrap()
        );
        // already passed → tomorrow
        assert_eq!(
            parse_when("7h30", now()).unwrap(),
            VN.with_ymd_and_hms(2026, 11, 27, 7, 30, 0).unwrap()
        );
        assert_eq!(
            parse_when("mai 8:00", now()).unwrap(),
            VN.with_ymd_and_hms(2026, 11, 27, 8, 0, 0).unwrap()
        );
    }

    #[test]
    fn dates() {
        assert_eq!(
            parse_when("25/12 09:00", now()).unwrap(),
            VN.with_ymd_and_hms(2026, 12, 25, 9, 0, 0).unwrap()
        );
        assert_eq!(
            parse_when("1/1", now()).unwrap(),
            VN.with_ymd_and_hms(2027, 1, 1, 8, 0, 0).unwrap()
        );
        assert_eq!(
            parse_when("2027-02-03 18:15", now()).unwrap(),
            VN.with_ymd_and_hms(2027, 2, 3, 18, 15, 0).unwrap()
        );
    }

    #[test]
    fn garbage() {
        assert!(parse_when("blah", now()).is_err());
    }
}
