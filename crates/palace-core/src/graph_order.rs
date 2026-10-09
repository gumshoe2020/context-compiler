use serde_json::Value;
use std::cmp::Ordering;
/// Default en-US ICU ordering for the ASCII/Latin-1 graph vocabulary.
/// Controls are ignorable; punctuation has primary weights, accents secondary, case tertiary.
pub fn locale_compare(a: &str, b: &str) -> Ordering {
    let key = |s: &str| {
        let mut primary = Vec::new();
        let mut accents = Vec::new();
        let mut case = Vec::new();
        for c in s.chars().filter(|c| !c.is_control()) {
            let lower = c.to_lowercase().next().unwrap_or(c);
            let (base, accent) = latin(lower);
            for ch in base.chars() {
                let punctuation = " _-,;:!?.'\"()[]{}@*/\\&#%`^+<=>|~$";
                let weight = punctuation.find(ch).map_or(1000 + ch as u32, |i| i as u32);
                primary.push(weight);
                accents.push(accent);
                case.push(u8::from(c.is_uppercase()));
            }
        }
        (primary, accents, case)
    };
    key(a).cmp(&key(b))
}
fn latin(c: char) -> (String, u8) {
    let mapped = match c {
        'à' => ("a", 2),
        'á' => ("a", 1),
        'â' => ("a", 3),
        'ã' => ("a", 4),
        'ä' => ("a", 5),
        'å' => ("a", 6),
        'è' => ("e", 2),
        'é' => ("e", 1),
        'ê' => ("e", 3),
        'ë' => ("e", 5),
        'ì' => ("i", 2),
        'í' => ("i", 1),
        'î' => ("i", 3),
        'ï' => ("i", 5),
        'ò' => ("o", 2),
        'ó' => ("o", 1),
        'ô' => ("o", 3),
        'õ' => ("o", 4),
        'ö' => ("o", 5),
        'ø' => ("o", 7),
        'ù' => ("u", 2),
        'ú' => ("u", 1),
        'û' => ("u", 3),
        'ü' => ("u", 5),
        'ç' => ("c", 7),
        'ñ' => ("n", 4),
        'ý' => ("y", 1),
        'ÿ' => ("y", 5),
        'æ' => ("ae", 7),
        'œ' => ("oe", 7),
        'ß' => ("ss", 7),
        _ => return (c.to_string(), 0),
    };
    (mapped.0.into(), mapped.1)
}
pub fn utf16_compare(a: &str, b: &str) -> Ordering {
    a.encode_utf16().cmp(b.encode_utf16())
}

/// JSON.stringify length, including null fields and JS shortest number formatting.
pub fn estimate_tokens(value: &impl serde::Serialize) -> usize {
    json_length(&serde_json::to_value(value).expect("finite graph JSON"))
        .div_ceil(4)
        .max(1)
}
fn json_length(value: &Value) -> usize {
    match value {
        Value::Number(n) => {
            let f = n.as_f64().unwrap_or(0.0);
            let text = if f == 0.0 {
                "0".into()
            } else if f.abs() >= 1e21 || f.abs() < 1e-6 {
                let s = format!("{f:e}");
                if s.contains("e-") {
                    s
                } else {
                    s.replace('e', "e+")
                }
            } else {
                f.to_string()
            };
            text.len()
        }
        Value::Array(a) => 2 + a.iter().map(json_length).sum::<usize>() + a.len().saturating_sub(1),
        Value::Object(o) => {
            2 + o
                .iter()
                .map(|(k, v)| json_length(&Value::String(k.clone())) + 1 + json_length(v))
                .sum::<usize>()
                + o.len().saturating_sub(1)
        }
        _ => value.to_string().encode_utf16().count(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn node_locale_order_and_json_stringify_utf16_goldens() {
        let mut names = [
            "a-b", "ab", "a_b", "a b", "a\0b", "aB", "Ab", "áb", "a!b", "a.b",
        ];
        names.sort_by(|a, b| locale_compare(a, b));
        assert_eq!(
            names,
            [
                "a b", "a_b", "a-b", "a!b", "a.b", "ab", "a\0b", "aB", "Ab", "áb"
            ]
        );
        let row = serde_json::json!({"subject":"é😀","predicate":"p","object":"b","confidence":1.0,"source_closet":null,"confidence_source":"column"});
        assert_eq!(json_length(&row), 111);
        assert_eq!(estimate_tokens(&row), 28);
        for (value, length) in [(1e-7, 4), (1e21, 5), (-0.0, 1)] {
            assert_eq!(json_length(&serde_json::json!(value)), length);
        }
        assert_eq!(json_length(&serde_json::json!("😀")), 4);
    }
}
