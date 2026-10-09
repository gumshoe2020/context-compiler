//! Diary file recipe, pinned from the OpenClaw mempalace-memory plugin (index.js:15-18, 158-168):
//! `stringParam` trims `title` and `text` (a blank title becomes "OpenClaw continuity note");
//! then `<day>-<sha256("{iso}\n{title}\n{text}")[:12]>.md` over the trimmed values, day =
//! America/New_York calendar date, iso = `Date.toISOString()` (UTC, milliseconds), content
//! `"# {title}\n\n{text}\n"`. The plugin rejects a blank text; callers never pass one.
use crate::ids::sha256;

pub const DEFAULT_TITLE: &str = "OpenClaw continuity note";
pub struct DiaryFile {
    pub name: String,
    pub content: String,
    /// The trimmed title and text actually hashed and written (M4b.2 L-2: audit fields use these).
    pub title: String,
    pub text: String,
}
/// Render a diary entry exactly as the plugin writes it (M4b.1 L1: render owns the trim).
pub fn render(epoch_millis: i64, title: &str, text: &str) -> DiaryFile {
    render_on(&new_york_date(epoch_millis), epoch_millis, title, text)
}
/// The same recipe with the file's calendar `day` given (PKG-1.2 item 2: palace-mcp's
/// `palace_diary_write` names the file by the local day).
pub fn render_on(day: &str, epoch_millis: i64, title: &str, text: &str) -> DiaryFile {
    let title = match js_trim(title) {
        "" => DEFAULT_TITLE,
        title => title,
    };
    let text = js_trim(text);
    let iso = iso_millis(epoch_millis);
    let hash = sha256(format!("{iso}\n{title}\n{text}").as_bytes());
    DiaryFile {
        name: format!("{day}-{}.md", &hash[..12]),
        content: format!("# {title}\n\n{text}\n"),
        title: title.to_owned(),
        text: text.to_owned(),
    }
}
/// JavaScript `String.prototype.trim`: Unicode White_Space except U+0085, plus U+FEFF.
fn js_trim(value: &str) -> &str {
    value.trim_matches(|c: char| (c.is_whitespace() && c != '\u{85}') || c == '\u{feff}')
}
/// `2026-09-25T23:05:12.345Z`, as JavaScript's `Date.prototype.toISOString`.
pub fn iso_millis(epoch_millis: i64) -> String {
    let seconds = epoch_millis.div_euclid(1000);
    let (year, month, day) = civil_from_days(seconds.div_euclid(86400));
    let second_of_day = seconds.rem_euclid(86400);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z",
        second_of_day / 3600,
        second_of_day / 60 % 60,
        second_of_day % 60,
        epoch_millis.rem_euclid(1000)
    )
}
/// America/New_York calendar date (US rules since 2007: EDT from the second Sunday of March
/// 02:00 EST to the first Sunday of November 02:00 EDT).
pub fn new_york_date(epoch_millis: i64) -> String {
    let seconds = epoch_millis.div_euclid(1000);
    let offset = if is_eastern_daylight(seconds) { 4 } else { 5 };
    let (year, month, day) = civil_from_days((seconds - offset * 3600).div_euclid(86400));
    format!("{year:04}-{month:02}-{day:02}")
}
fn is_eastern_daylight(utc_seconds: i64) -> bool {
    let (year, _, _) = civil_from_days(utc_seconds.div_euclid(86400));
    let start = nth_sunday(year, 3, 2) * 86400 + 7 * 3600; // 02:00 EST
    let end = nth_sunday(year, 11, 1) * 86400 + 6 * 3600; // 02:00 EDT
    (start..end).contains(&utc_seconds)
}
/// Days since the Unix epoch of the n-th Sunday of a month.
fn nth_sunday(year: i64, month: i64, n: i64) -> i64 {
    let first = days_from_civil(year, month, 1);
    let weekday = (first + 4).rem_euclid(7); // 1970-01-01 was a Thursday (4); Sunday = 0
    first + (7 - weekday) % 7 + (n - 1) * 7
}
/// Howard Hinnant's days_from_civil.
pub fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let yoe = year - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}
/// Howard Hinnant's civil_from_days.
pub fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(month <= 2), month, day)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn millis(y: i64, mo: i64, d: i64, h: i64, mi: i64) -> i64 {
        (days_from_civil(y, mo, d) * 86400 + h * 3600 + mi * 60) * 1000
    }
    #[test]
    fn iso_matches_javascript() {
        assert_eq!(iso_millis(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(
            iso_millis(millis(2026, 9, 26, 3, 5) + 12_345),
            "2026-09-26T03:05:12.345Z"
        );
    }
    #[test]
    fn new_york_day_follows_dst() {
        // 23:05 EDT on 2026-09-25 is 03:05Z on the 26th.
        assert_eq!(new_york_date(millis(2026, 9, 26, 3, 5)), "2026-09-25");
        assert_eq!(new_york_date(millis(2026, 9, 26, 4, 0)), "2026-09-26");
        // EST in January: 04:59Z is still the previous day, 05:00Z is not.
        assert_eq!(new_york_date(millis(2026, 1, 15, 4, 59)), "2026-01-14");
        assert_eq!(new_york_date(millis(2026, 1, 15, 5, 0)), "2026-01-15");
        // 2026 DST: starts Sun 8 March 07:00Z, ends Sun 1 November 06:00Z.
        assert!(!is_eastern_daylight(millis(2026, 3, 8, 6, 59) / 1000));
        assert!(is_eastern_daylight(millis(2026, 3, 8, 7, 0) / 1000));
        assert!(is_eastern_daylight(millis(2026, 11, 1, 5, 59) / 1000));
        assert!(!is_eastern_daylight(millis(2026, 11, 1, 6, 0) / 1000));
    }
    #[test]
    fn recipe_trims_title_and_text_like_the_plugin() {
        let at = millis(2026, 9, 26, 3, 5) + 12_345;
        let file = render(at, " T\n", "\u{feff}  body\n");
        let expected = sha256("2026-09-26T03:05:12.345Z\nT\nbody".as_bytes());
        assert_eq!(file.name, format!("2026-09-25-{}.md", &expected[..12]));
        assert_eq!(file.content, "# T\n\nbody\n");
        assert_eq!((file.title.as_str(), file.text.as_str()), ("T", "body"));
        let untitled = render(at, "  ", "body");
        assert_eq!(untitled.content, "# OpenClaw continuity note\n\nbody\n");
        assert_eq!(js_trim("\u{85}x\u{2028}"), "\u{85}x");
    }
    #[test]
    fn render_on_names_the_given_day_and_hashes_as_render() {
        let at = millis(2026, 9, 26, 3, 5) + 12_345;
        let (local, new_york) = (
            render_on("2026-09-26", at, "T", "body"),
            render(at, "T", "body"),
        );
        assert_eq!(local.name[..10], *"2026-09-26");
        assert_eq!(local.name[10..], new_york.name[10..]);
        assert_eq!(local.content, new_york.content);
    }
}
