//! Reads a Google Calendar iCal feed and expands events into concrete occurrences.
//!
//! Supported recurrence: FREQ=DAILY/WEEKLY/MONTHLY/YEARLY with INTERVAL, COUNT, UNTIL,
//! BYDAY (weekly only), EXDATE and per-occurrence overrides (RECURRENCE-ID).
//! Rules beyond that keep only their first occurrence.

use std::collections::HashSet;
use std::io::BufReader;

use chrono::{
    DateTime, Datelike, Duration, Months, NaiveDate, NaiveDateTime, TimeZone, Utc, Weekday,
};
use chrono_tz::Tz;
use ical::parser::ical::component::IcalEvent;
use ical::property::Property;

#[derive(Debug, Clone, PartialEq)]
pub struct Occurrence {
    pub uid: String,
    pub summary: String,
    pub location: Option<String>,
    pub start: DateTime<Utc>,
    pub all_day: bool,
}

pub async fn fetch(http: &reqwest::Client, url: &str, tz: Tz) -> anyhow::Result<Vec<Occurrence>> {
    let body = http
        .get(url)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    let now = Utc::now();
    Ok(parse(
        &body,
        tz,
        now - Duration::days(1),
        now + Duration::days(8),
    ))
}

fn prop<'a>(ev: &'a IcalEvent, name: &str) -> Option<&'a Property> {
    ev.properties.iter().find(|p| p.name == name)
}

fn param<'a>(p: &'a Property, name: &str) -> Option<&'a str> {
    p.params
        .as_ref()?
        .iter()
        .find(|(k, _)| k == name)
        .and_then(|(_, v)| v.first())
        .map(|s| s.as_str())
}

fn unescape(s: &str) -> String {
    s.replace("\\n", " ")
        .replace("\\N", " ")
        .replace("\\,", ",")
        .replace("\\;", ";")
        .replace("\\\\", "\\")
}

/// Parses a DTSTART-like value. Returns (local wall time, zone, all_day).
fn parse_dt(value: &str, tzid: Option<&str>, default_tz: Tz) -> Option<(NaiveDateTime, Tz, bool)> {
    let v = value.trim();
    if v.len() == 8 {
        let d = NaiveDate::parse_from_str(v, "%Y%m%d").ok()?;
        return Some((d.and_hms_opt(0, 0, 0)?, default_tz, true));
    }
    if let Some(utc) = v.strip_suffix('Z') {
        let n = NaiveDateTime::parse_from_str(utc, "%Y%m%dT%H%M%S").ok()?;
        // Express UTC instants as wall time in the default zone so recurrences keep the local hour.
        let local = Utc
            .from_utc_datetime(&n)
            .with_timezone(&default_tz)
            .naive_local();
        return Some((local, default_tz, false));
    }
    let n = NaiveDateTime::parse_from_str(v, "%Y%m%dT%H%M%S").ok()?;
    let tz = tzid
        .and_then(|t| t.parse::<Tz>().ok())
        .unwrap_or(default_tz);
    Some((n, tz, false))
}

fn to_utc(n: NaiveDateTime, tz: Tz) -> Option<DateTime<Utc>> {
    tz.from_local_datetime(&n)
        .earliest()
        .map(|d| d.with_timezone(&Utc))
}

fn prop_dt(p: &Property, tz: Tz) -> Option<(NaiveDateTime, Tz, bool)> {
    parse_dt(p.value.as_deref()?, param(p, "TZID"), tz)
}

struct Rule {
    freq: String,
    interval: u32,
    count: Option<u32>,
    until: Option<DateTime<Utc>>,
    byday: Vec<Weekday>,
    unsupported: bool,
}

fn parse_rule(s: &str, tz: Tz) -> Rule {
    let mut r = Rule {
        freq: String::new(),
        interval: 1,
        count: None,
        until: None,
        byday: vec![],
        unsupported: false,
    };
    for kv in s.split(';') {
        let Some((k, v)) = kv.split_once('=') else {
            continue;
        };
        match k {
            "FREQ" => r.freq = v.to_string(),
            "INTERVAL" => r.interval = v.parse().unwrap_or(1).max(1),
            "COUNT" => r.count = v.parse().ok(),
            "UNTIL" => r.until = parse_dt(v, None, tz).and_then(|(n, z, _)| to_utc(n, z)),
            "BYDAY" => {
                for d in v.split(',') {
                    match weekday(d) {
                        Some(w) => r.byday.push(w),
                        None => r.unsupported = true, // e.g. "2TU" (second Tuesday)
                    }
                }
            }
            "WKST" => {}
            _ => r.unsupported = true,
        }
    }
    if !r.byday.is_empty() && r.freq != "WEEKLY" {
        r.unsupported = true;
    }
    r
}

fn weekday(s: &str) -> Option<Weekday> {
    Some(match s {
        "MO" => Weekday::Mon,
        "TU" => Weekday::Tue,
        "WE" => Weekday::Wed,
        "TH" => Weekday::Thu,
        "FR" => Weekday::Fri,
        "SA" => Weekday::Sat,
        "SU" => Weekday::Sun,
        _ => return None,
    })
}

/// Local start times of every occurrence of a rule, in order, until `to`.
fn expand(start: NaiveDateTime, rule: &Rule, tz: Tz, to: DateTime<Utc>) -> Vec<NaiveDateTime> {
    let mut out = Vec::new();
    let limit_reached = |n: NaiveDateTime, out: &Vec<NaiveDateTime>| -> bool {
        let utc = to_utc(n, tz);
        rule.count.is_some_and(|c| out.len() as u32 >= c)
            || utc.is_none_or(|u| u > to || rule.until.is_some_and(|until| u > until))
    };

    if rule.unsupported
        || !matches!(
            rule.freq.as_str(),
            "DAILY" | "WEEKLY" | "MONTHLY" | "YEARLY"
        )
    {
        return vec![start];
    }

    // Old daily events can need many steps to reach today; the cap only guards against bad data.
    for k in 0..100_000u32 {
        let step = k * rule.interval;
        let candidates: Vec<NaiveDateTime> = match rule.freq.as_str() {
            "DAILY" => vec![start + Duration::days(step as i64)],
            "WEEKLY" if rule.byday.is_empty() => vec![start + Duration::weeks(step as i64)],
            "WEEKLY" => {
                let monday = start.date()
                    - Duration::days(start.weekday().num_days_from_monday() as i64)
                    + Duration::weeks(step as i64);
                let mut days: Vec<NaiveDateTime> = rule
                    .byday
                    .iter()
                    .map(|w| {
                        (monday + Duration::days(w.num_days_from_monday() as i64))
                            .and_time(start.time())
                    })
                    .filter(|d| *d >= start)
                    .collect();
                days.sort();
                days
            }
            "MONTHLY" => start
                .checked_add_months(Months::new(step))
                .into_iter()
                .collect(),
            _ => start
                .checked_add_months(Months::new(step * 12))
                .into_iter()
                .collect(),
        };
        for c in candidates {
            if limit_reached(c, &out) {
                return out;
            }
            out.push(c);
        }
    }
    out
}

pub fn parse(ics: &str, tz: Tz, from: DateTime<Utc>, to: DateTime<Utc>) -> Vec<Occurrence> {
    let mut events: Vec<IcalEvent> = Vec::new();
    for cal in ical::IcalParser::new(BufReader::new(ics.as_bytes())).flatten() {
        events.extend(cal.events);
    }

    // Occurrences replaced by an override (RECURRENCE-ID) must not come from the master rule.
    let mut overridden: HashSet<(String, i64)> = HashSet::new();
    for ev in &events {
        if let (Some(uid), Some(rid)) = (prop(ev, "UID"), prop(ev, "RECURRENCE-ID"))
            && let Some(t) = prop_dt(rid, tz).and_then(|(n, z, _)| to_utc(n, z))
        {
            overridden.insert((uid.value.clone().unwrap_or_default(), t.timestamp()));
        }
    }

    let mut out = Vec::new();
    for ev in &events {
        if prop(ev, "STATUS").and_then(|p| p.value.as_deref()) == Some("CANCELLED") {
            continue;
        }
        let Some((start, ev_tz, all_day)) = prop(ev, "DTSTART").and_then(|p| prop_dt(p, tz)) else {
            continue;
        };
        let uid = prop(ev, "UID")
            .and_then(|p| p.value.clone())
            .unwrap_or_default();
        let summary = prop(ev, "SUMMARY")
            .and_then(|p| p.value.as_deref())
            .map(unescape)
            .unwrap_or_else(|| "(Không có tiêu đề)".into());
        let location = prop(ev, "LOCATION")
            .and_then(|p| p.value.as_deref())
            .map(unescape)
            .filter(|s| !s.is_empty());

        let mut exdates: HashSet<i64> = HashSet::new();
        for p in ev.properties.iter().filter(|p| p.name == "EXDATE") {
            for v in p.value.as_deref().unwrap_or("").split(',') {
                if let Some(t) =
                    parse_dt(v, param(p, "TZID"), tz).and_then(|(n, z, _)| to_utc(n, z))
                {
                    exdates.insert(t.timestamp());
                }
            }
        }

        let is_override = prop(ev, "RECURRENCE-ID").is_some();
        let starts = match prop(ev, "RRULE").and_then(|p| p.value.as_deref()) {
            Some(r) if !is_override => expand(start, &parse_rule(r, tz), ev_tz, to),
            _ => vec![start],
        };

        for s in starts {
            let Some(utc) = to_utc(s, ev_tz) else {
                continue;
            };
            let ts = utc.timestamp();
            if utc < from || utc > to || exdates.contains(&ts) {
                continue;
            }
            if !is_override && overridden.contains(&(uid.clone(), ts)) {
                continue;
            }
            out.push(Occurrence {
                uid: uid.clone(),
                summary: summary.clone(),
                location: location.clone(),
                start: utc,
                all_day,
            });
        }
    }
    out.sort_by_key(|o| o.start);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono_tz::Asia::Ho_Chi_Minh as VN;

    const ICS: &str = "BEGIN:VCALENDAR\r
VERSION:2.0\r
BEGIN:VEVENT\r
UID:once@x\r
DTSTART:20261126T020000Z\r
SUMMARY:Họp nhóm\\, phòng A\r
END:VEVENT\r
BEGIN:VEVENT\r
UID:weekly@x\r
DTSTART;TZID=Asia/Ho_Chi_Minh:20261102T190000\r
RRULE:FREQ=WEEKLY;BYDAY=MO,TH\r
EXDATE;TZID=Asia/Ho_Chi_Minh:20261130T190000\r
SUMMARY:Gym\r
END:VEVENT\r
BEGIN:VEVENT\r
UID:weekly@x\r
RECURRENCE-ID;TZID=Asia/Ho_Chi_Minh:20261126T190000\r
DTSTART;TZID=Asia/Ho_Chi_Minh:20261126T200000\r
SUMMARY:Gym (dời giờ)\r
END:VEVENT\r
BEGIN:VEVENT\r
UID:bday@x\r
DTSTART;VALUE=DATE:20261127\r
RRULE:FREQ=YEARLY\r
SUMMARY:Sinh nhật\r
END:VEVENT\r
END:VCALENDAR\r
";

    #[test]
    fn expands_week() {
        let from = VN
            .with_ymd_and_hms(2026, 11, 26, 0, 0, 0)
            .unwrap()
            .with_timezone(&Utc);
        let to = from + Duration::days(8);
        let occ = parse(ICS, VN, from, to);
        let got: Vec<(String, String)> = occ
            .iter()
            .map(|o| {
                (
                    o.summary.clone(),
                    o.start.with_timezone(&VN).format("%d %H:%M").to_string(),
                )
            })
            .collect();
        assert_eq!(
            got,
            vec![
                ("Họp nhóm, phòng A".into(), "26 09:00".into()),
                ("Gym (dời giờ)".into(), "26 20:00".into()),
                ("Sinh nhật".into(), "27 00:00".into()),
                // 30/11 is excluded by EXDATE
                ("Gym".into(), "03 19:00".into()),
            ]
        );
        assert!(occ[2].all_day);
    }
}
