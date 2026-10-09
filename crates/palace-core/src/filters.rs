fn is_word(c: char) -> bool {
    (c.is_alphanumeric() && !matches!(c, '\u{300}'..='\u{36f}')) || c == '_'
}

pub fn optional_name(value: Option<&str>) -> Result<Option<String>, &'static str> {
    let Some(value) = trimmed(value) else {
        return Ok(None);
    };
    let edge = |c: char| is_word(c) && c != '_';
    if value.chars().count() > 128
        || value.contains("..")
        || !value.starts_with(edge)
        || !value.ends_with(edge)
        || !value.chars().all(|c| is_word(c) || " .'-".contains(c))
    {
        return Err("invalid wing/room name");
    }
    Ok(Some(value.into()))
}
pub fn optional_source(value: Option<&str>) -> Result<Option<String>, &'static str> {
    let Some(value) = trimmed(value) else {
        return Ok(None);
    };
    if value.chars().count() > 4096
        || value.contains('\0')
        || value.split(['/', '\\']).any(|part| part == "..")
    {
        return Err("invalid source_file filter");
    }
    Ok(Some(value.into()))
}
fn trimmed(value: Option<&str>) -> Option<&str> {
    value
        .map(|v| v.trim_matches(|c: char| c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c)))
        .filter(|v| !v.is_empty())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn names_follow_python_safe_unicode_boundaries() {
        for value in [
            "../x",
            "a/b",
            "a\\b",
            "_a",
            "a_",
            "a..b",
            "a\0b",
            "a:b",
            "a\u{301}b",
        ] {
            assert!(optional_name(Some(value)).is_err(), "{value}");
        }
        assert_eq!(
            optional_name(Some("  Étage 2  ")).unwrap().as_deref(),
            Some("Étage 2")
        );
        assert!(optional_name(Some(&"a".repeat(129))).is_err());
        assert_eq!(optional_name(Some(" \n ")).unwrap(), None);
    }
    #[test]
    fn sources_are_strings_not_filesystem_lookups() {
        assert_eq!(
            optional_source(Some(" /not/present/a.md "))
                .unwrap()
                .as_deref(),
            Some("/not/present/a.md")
        );
        for value in ["a/../b", "a\\..\\b", "..", "a\0b"] {
            assert!(optional_source(Some(value)).is_err());
        }
        assert!(optional_source(Some(&"界".repeat(4097))).is_err());
        assert!(optional_source(Some(&"界".repeat(4096))).is_ok());
        assert_eq!(optional_source(Some(" ")).unwrap(), None);
    }
}
