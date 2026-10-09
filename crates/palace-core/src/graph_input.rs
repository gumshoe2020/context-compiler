use serde_json::Value;
pub fn optional<'a>(args: &'a Value, key: &str, cap: usize) -> Result<Option<&'a str>, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) if s.chars().count() <= cap && !s.contains('\0') => Ok(Some(s)),
        _ => Err(format!(
            "{key} must be a string without NUL, at most {cap} characters"
        )),
    }
}
pub fn name(args: &Value, key: &str) -> Result<String, String> {
    let s = optional(args, key, 4096)?
        .ok_or_else(|| format!("{key} must be a non-empty string"))?
        .trim();
    if s.is_empty() || s.chars().count() > 128 {
        return Err(format!(
            "{key} must be a non-empty string of at most 128 characters"
        ));
    }
    Ok(s.into())
}
pub fn predicate(args: &Value) -> Result<String, String> {
    let value = name(args, "predicate")?;
    crate::filters::optional_name(Some(&value))
        .map_err(|_| "predicate contains invalid characters".to_string())?;
    Ok(crate::ids::predicate_id(&value))
}
pub fn temporal(args: &Value, key: &str) -> Result<Option<String>, String> {
    optional(args, key, 64)?
        .map(|s| temporal_value(s, key))
        .transpose()
}
pub fn temporal_value(s: &str, key: &str) -> Result<String, String> {
    if s.is_empty() {
        return Ok(String::new());
    }
    let s = s.trim();
    let normalized = s
        .strip_suffix("+00:00")
        .map(|s| format!("{s}Z"))
        .unwrap_or_else(|| s.into());
    let b = normalized.as_bytes();
    let number =
        |range: std::ops::Range<usize>| normalized.get(range).and_then(|s| s.parse::<u32>().ok());
    let valid_shape = (b.len() == 10 || b.len() == 20)
        && b[4] == b'-'
        && b[7] == b'-'
        && b.iter()
            .enumerate()
            .all(|(i, c)| matches!(i, 4 | 7 | 10 | 13 | 16 | 19) || c.is_ascii_digit());
    let (year, month, day) = (
        number(0..4).unwrap_or(0),
        number(5..7).unwrap_or(0),
        number(8..10).unwrap_or(0),
    );
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
    let time_valid = b.len() == 10
        || (b.len() == 20
            && b[10] == b'T'
            && b[13] == b':'
            && b[16] == b':'
            && b[19] == b'Z'
            && number(11..13).is_some_and(|n| n < 24)
            && number(14..16).is_some_and(|n| n < 60)
            && number(17..19).is_some_and(|n| n < 60));
    if !valid_shape || year == 0 || day == 0 || day > days || !time_valid {
        return Err(format!(
            "{key}={s:?} is not a valid ISO-8601 date or UTC datetime (expected YYYY-MM-DD or YYYY-MM-DDTHH:MM:SSZ)"
        ));
    }
    Ok(normalized)
}
pub fn temporal_key(value: &str, end: bool) -> String {
    if value.len() == 10 {
        format!("{value}T{}Z", if end { "23:59:59" } else { "00:00:00" })
    } else {
        value.into()
    }
}
pub fn interval(start: Option<&str>, end: Option<&str>) -> Result<(), String> {
    if let (Some(start), Some(end)) = (start, end)
        && temporal_key(end, true) < temporal_key(start, false)
    {
        return Err(format!(
            "valid_to={end:?} is before valid_from={start:?}; an inverted interval would be invisible to every KG query"
        ));
    }
    Ok(())
}
