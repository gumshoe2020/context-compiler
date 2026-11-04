//! Open-item actions: validate shape and quotes against precisely the cited new line.
use crate::distill::verbatim_at;
use serde_json::Value;
use std::collections::BTreeMap;

pub fn text<'a>(v: &'a Value, key: &str) -> Result<&'a str, String> {
    v[key]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| format!("{key} missing or empty"))
}
pub fn date(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != 10
        || b[4] != b'-'
        || b[7] != b'-'
        || !b
            .iter()
            .enumerate()
            .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
    {
        return false;
    }
    let year: u32 = s[..4].parse().unwrap_or(0);
    let month: u32 = s[5..7].parse().unwrap_or(0);
    let day: u32 = s[8..].parse().unwrap_or(0);
    let leap = year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
    let days = match month {
        2 => {
            if leap {
                29
            } else {
                28
            }
        }
        4 | 6 | 9 | 11 => 30,
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        _ => 0,
    };
    year > 0 && day > 0 && day <= days
}
pub fn check(v: &Value, lines: &BTreeMap<usize, &str>) -> Result<(), String> {
    let action = text(v, "action")?;
    if !["create", "update", "close", "expire"].contains(&action) {
        return Err("unknown action".into());
    }
    let id = text(v, "id")?;
    if id.len() > 64
        || !id.split('-').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        })
    {
        return Err("id must be a short lowercase kebab id (at most 64 bytes)".into());
    }
    for (key, choices) in [
        ("owner", &["assistant", "person", "other"][..]),
        (
            "kind",
            &["task", "decision", "question", "pastoral", "topic"][..],
        ),
    ] {
        if (action == "create" || v.get(key).is_some()) && !choices.contains(&text(v, key)?) {
            return Err(format!("invalid {key}"));
        }
    }
    if action == "create" || v.get("title").is_some() {
        text(v, "title")?;
    }
    if let Some(due) = v.get("due")
        && !due.is_null()
        && !due.as_str().is_some_and(date)
    {
        return Err("due must be a valid YYYY-MM-DD or null".into());
    }
    if action == "close" && !["done", "dropped"].contains(&text(v, "status")?) {
        return Err("close status must be done or dropped".into());
    }
    // Extra mutation fields are refused rather than silently accepted or applied.
    let allowed = [
        "action",
        "id",
        "line",
        "quote",
        "close_line",
        "close_quote",
        "owner",
        "kind",
        "title",
        "due",
        "status",
    ];
    if v.as_object()
        .is_some_and(|o| o.keys().any(|k| !allowed.contains(&k.as_str())))
    {
        return Err("unknown action field".into());
    }
    if action != "close" && v.get("status").is_some() {
        return Err("only close accepts status".into());
    }
    if ["close", "expire"].contains(&action)
        && ["owner", "kind", "title", "due"]
            .iter()
            .any(|k| v.get(*k).is_some())
    {
        return Err("closing action cannot edit item fields".into());
    }
    for key in ["quote", "close_quote"] {
        let required = if ["close", "expire"].contains(&action) {
            key == "close_quote"
        } else {
            key == "quote"
        };
        if !required && v.get(key).is_none() {
            continue;
        }
        let quote = text(v, key)?;
        let line_key = if key == "close_quote" && v.get("close_line").is_some() {
            "close_line"
        } else {
            "line"
        };
        let line = v[line_key]
            .as_u64()
            .and_then(|n| usize::try_from(n).ok())
            .ok_or_else(|| format!("{line_key} must be a line number"))?;
        if !verbatim_at(quote, line, lines) {
            return Err(format!("{key} not verbatim in cited line {line}"));
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn strict_citation_and_calendar_dates() {
        let lines = BTreeMap::from([(1, "I will send it."), (2, "Okay sent.")]);
        let mut v = json!({"action":"close","id":"send-map","line":2,"close_quote":"Okay sent.","status":"done"});
        assert!(check(&v, &lines).is_ok());
        v["line"] = json!(1);
        assert!(check(&v, &lines).unwrap_err().contains("not verbatim"));
        v["line"] = json!(2);
        v["quote"] = json!("fabricated");
        assert!(check(&v, &lines).is_err());
        for good in ["2032-02-29", "2031-12-31"] {
            assert!(date(good));
        }
        for bad in ["2031-02-29", "2031-13-01", "2031-04-31", "0000-01-01"] {
            assert!(!date(bad));
        }
    }
}

/// RFC3339 timestamps, including offsets and fractional seconds, as Unix milliseconds.
/// Replay rejects missing/invalid clocks rather than silently inventing windows.
pub fn timestamp_millis(ts: &str) -> Result<i64, String> {
    let bad = || format!("invalid RFC3339 timestamp {ts:?}");
    if ts.len() < 20 || !ts.is_ascii() || !date(&ts[..10]) || !b"Tt".contains(&ts.as_bytes()[10]) {
        return Err(bad());
    }
    let clock = &ts[11..];
    let b = clock.as_bytes();
    if b.len() < 9
        || b[2] != b':'
        || b[5] != b':'
        || !b[..8]
            .iter()
            .enumerate()
            .all(|(i, c)| i == 2 || i == 5 || c.is_ascii_digit())
    {
        return Err(bad());
    }
    let number = |s: &str| s.parse::<i64>().map_err(|_| bad());
    let hour = number(&clock[..2])?;
    let minute = number(&clock[3..5])?;
    let second = number(&clock[6..8])?;
    if hour > 23 || minute > 59 || second > 59 || hour < 0 || minute < 0 || second < 0 {
        return Err(bad());
    }
    let mut rest = &clock[8..];
    let mut millis = 0;
    if let Some(fraction) = rest.strip_prefix('.') {
        let digits = fraction.bytes().take_while(u8::is_ascii_digit).count();
        if digits == 0 {
            return Err(bad());
        }
        let fraction = &fraction[..digits];
        millis = number(&format!("{fraction:0<3}")[..3])?;
        rest = &rest[digits + 1..];
    }
    let offset = if rest == "Z" || rest == "z" {
        0
    } else {
        let b = rest.as_bytes();
        if b.len() != 6 || !b"+-".contains(&b[0]) || b[3] != b':' {
            return Err(bad());
        }
        let h = number(&rest[1..3])?;
        let m = number(&rest[4..])?;
        if !(0..=23).contains(&h) || !(0..=59).contains(&m) {
            return Err(bad());
        }
        (h * 60 + m) * 60 * if b[0] == b'-' { -1 } else { 1 }
    };
    let year = number(&ts[..4])?;
    let month = number(&ts[5..7])?;
    let day = number(&ts[8..10])?;
    let leap = |y: i64| y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let year_days = |y: i64| 365 * (y - 1) + (y - 1) / 4 - (y - 1) / 100 + (y - 1) / 400;
    let mut days = year_days(year) - year_days(1970) + day - 1;
    for m in 1..month {
        days += match m {
            2 => {
                if leap(year) {
                    29
                } else {
                    28
                }
            }
            4 | 6 | 9 | 11 => 30,
            _ => 31,
        };
    }
    Ok(((days * 24 + hour) * 3600 + minute * 60 + second - offset) * 1000 + millis)
}
/// Inclusive physical-line windows, split at a gap >= the threshold between kept turns.
pub fn windows(
    turns: &[(usize, crate::transcript::Turn)],
    minutes: u64,
) -> Result<Vec<(usize, usize)>, String> {
    let threshold = i64::try_from(minutes)
        .ok()
        .and_then(|m| m.checked_mul(60_000))
        .filter(|m| *m > 0)
        .ok_or("window minutes must be positive and bounded")?;
    let mut windows = Vec::new();
    let mut start = None;
    let mut previous = None;
    let mut last = 0;
    for (line, turn) in turns {
        let at = timestamp_millis(&turn.ts).map_err(|e| format!("line {line}: {e}"))?;
        if previous.is_some_and(|p| at - p >= threshold) {
            windows.push((start.unwrap(), last));
            start = None;
        }
        start.get_or_insert(*line);
        previous = Some(at);
        last = *line;
    }
    if let Some(start) = start {
        windows.push((start, last));
    }
    Ok(windows)
}
#[cfg(test)]
mod replay_tests {
    use super::*;
    #[test]
    fn offsets_fractional_seconds_and_exact_boundary() {
        let parse = |s| timestamp_millis(s).unwrap();
        assert_eq!(parse("1970-01-01T00:00:00Z"), 0);
        assert_eq!(
            parse("2031-04-01T09:00:00Z"),
            parse("2031-04-01T05:00:00-04:00")
        );
        assert_eq!(
            parse("2031-04-01T09:00:00.250Z") - parse("2031-04-01T09:00:00Z"),
            250
        );
        for bad in [
            "",
            "2031-04-31T09:00:00Z",
            "2031-04-01T24:00:00Z",
            "2031-04-01T09:00:00",
        ] {
            assert!(timestamp_millis(bad).is_err());
        }
        let turns: Vec<_> = [
            (1, "2031-04-01T09:00:00.250Z"),
            (3, "2031-04-01T09:30:00Z"),
            (4, "2031-04-01T10:00:00Z"),
        ]
        .into_iter()
        .map(|(n, ts)| {
            (
                n,
                crate::transcript::Turn {
                    ts: ts.into(),
                    role: "user".into(),
                    text: "Synthetic".into(),
                },
            )
        })
        .collect();
        assert_eq!(windows(&turns, 30).unwrap(), [(1, 3), (4, 4)]);
        assert!(windows(&turns, 0).is_err());
    }
}

/// Format Unix milliseconds in UTC, retaining millisecond precision.
pub fn timestamp_rfc3339(ms: i64) -> String {
    let days = ms.div_euclid(86_400_000);
    let seconds = ms.rem_euclid(86_400_000) / 1000;
    // Gregorian civil date from days since the Unix epoch.
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z",
        seconds / 3600,
        seconds / 60 % 60,
        seconds % 60,
        ms.rem_euclid(1000)
    )
}
