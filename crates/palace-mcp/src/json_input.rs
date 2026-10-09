use serde_json::Value;

pub fn parse(line: &str) -> Result<Value, serde_json::Error> {
    serde_json::from_str(line).or_else(|_| serde_json::from_str(&repair(line)))
}
fn code(bytes: &[u8], at: usize) -> Option<u16> {
    let escape = bytes.get(at..at + 6)?;
    if &escape[..2] != b"\\u" {
        return None;
    }
    u16::from_str_radix(std::str::from_utf8(&escape[2..]).ok()?, 16).ok()
}
fn repair(line: &str) -> String {
    let bytes = line.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        if let Some(unit) = code(bytes, at).filter(|u| (0xd800..=0xdfff).contains(u)) {
            if unit <= 0xdbff && code(bytes, at + 6).is_some_and(|u| (0xdc00..=0xdfff).contains(&u))
            {
                output.extend_from_slice(&bytes[at..at + 12]);
                at += 12;
            } else {
                output.extend_from_slice(b"\\ufffd");
                at += 6;
            }
        } else {
            let count = if bytes[at] == b'\\' && at + 1 < bytes.len() {
                2
            } else {
                1
            };
            output.extend_from_slice(&bytes[at..at + count]);
            at += count;
        }
    }
    String::from_utf8(output).expect("repair preserves UTF-8")
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recovers_surrogates_without_changing_pairs_or_literal_escapes() {
        let value = parse(r#"{"id":7,"text":"\ud83d/\udc00/\ud83d\ude00/\\ud83d/é"}"#).unwrap();
        assert_eq!(value["id"], 7);
        assert_eq!(value["text"], "�/�/😀/\\ud83d/é");
        assert!(parse(r#"{"id":7,"text":"\ud83d",}"#).is_err());
    }
}
