use sha2::{Digest, Sha256};
pub const ID_RECIPE: &str = "v3";
pub fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
pub fn length_prefixed(parts: &[&str]) -> String {
    parts
        .iter()
        .map(|part| format!("{}:{part}", part.chars().count()))
        .collect()
}
pub fn drawer_id(wing: &str, room: &str, source_file: &str, chunk_index: usize) -> String {
    let hash = sha256(length_prefixed(&[source_file, &chunk_index.to_string()]).as_bytes());
    format!("drawer_{wing}_{room}_{}", &hash[..24])
}
pub fn triple_id(
    subject: &str,
    predicate: &str,
    object: &str,
    valid_from: &str,
    recorded_at: &str,
) -> String {
    let hash = sha256(length_prefixed(&[valid_from, recorded_at]).as_bytes());
    format!("t_{subject}_{predicate}_{object}_{}", &hash[..12])
}
pub fn entity_id(name: &str) -> String {
    name.to_lowercase().replace(' ', "_").replace('\'', "")
}

pub fn predicate_id(value: &str) -> String {
    value.to_lowercase().replace(' ', "_")
}
pub fn temporal_triple_id(
    subject: &str,
    predicate: &str,
    object: &str,
    valid_from: Option<&str>,
    recorded_at: &str,
) -> String {
    triple_id(
        subject,
        predicate,
        object,
        valid_from.unwrap_or("None"),
        recorded_at,
    )
}

/// Independent ids.py outputs recorded in the M3a review, section 3.
pub fn verify_python_golden_ids() -> bool {
    let at = "2026-09-24T12:00:00.000000Z";
    [
        ("a", None, "t_a_supports_b_a4911618c606"),
        ("a", Some(""), "t_a_supports_b_1210fb94c3e5"),
        ("é", Some("2026-01-01"), "t_é_supports_b_2337787991ad"),
    ]
    .into_iter()
    .all(|(subject, from, expected)| {
        temporal_triple_id(subject, "supports", "b", from, at) == expected
    })
}
#[cfg(test)]
mod tests {
    #[test]
    fn real_python_ids_cover_none_empty_and_unicode() {
        assert!(super::verify_python_golden_ids());
    }
}
