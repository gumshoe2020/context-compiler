use serde::{Deserialize, Serialize};
/// MemPalace owns ranking; the compiler consumes text and provenance only.
#[derive(Debug, Deserialize, Serialize)]
pub struct SearchHit {
    pub text: String,
    pub source_path: String,
    pub source_file: String,
    pub wing: String,
    pub room: String,
    #[serde(flatten)]
    pub metadata: serde_json::Map<String, serde_json::Value>,
}

/// Admit each hit before deduplication; the first visible chunk represents its entry.
/// Preserve backend rank, including when distances disagree with that rank.
pub fn distinct_entries(
    hits: impl IntoIterator<Item = SearchHit>,
    limit: usize,
    mut allowed: impl FnMut(&SearchHit) -> bool,
) -> Vec<SearchHit> {
    let mut seen = std::collections::BTreeSet::new();
    hits.into_iter()
        .filter(|hit| {
            if !allowed(hit) {
                return false;
            }
            let basename = hit
                .source_file
                .rsplit(['/', '\\'])
                .next()
                .unwrap_or_default();
            seen.insert(basename.strip_suffix(".md").unwrap_or(basename).to_owned())
        })
        .take(limit)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(file: &str, text: &str) -> SearchHit {
        SearchHit {
            text: text.into(),
            source_file: file.into(),
            source_path: file.into(),
            wing: "openclaw".into(),
            room: "general".into(),
            metadata: Default::default(),
        }
    }

    #[test]
    fn five_entries_in_backend_order_after_three_top_chunks() {
        let hits = [
            "b.md", "b.md", "b.md", "z.md", "a.md", "d.md", "c.md", "f.md",
        ];
        let actual = distinct_entries(hits.map(|s| hit(s, "visible")), 5, |_| true);
        assert_eq!(
            actual
                .iter()
                .map(|h| h.source_file.as_str())
                .collect::<Vec<_>>(),
            ["b.md", "z.md", "a.md", "d.md", "c.md"]
        );
    }

    #[test]
    fn privacy_precedes_dedupe_and_limit() {
        let actual = distinct_entries(
            [
                hit("hidden.md", "hidden"),
                hit("a.md", "hidden"),
                hit("a.md", "visible"),
                hit("b.md", "visible"),
            ],
            1,
            |h| h.text == "visible",
        );
        assert_eq!(actual.len(), 1);
        assert_eq!(actual[0].source_file, "a.md");
        assert_eq!(actual[0].text, "visible");
    }

    #[test]
    fn duplicate_window_can_return_fewer_and_basename_is_the_identity() {
        let actual = distinct_entries(
            [hit("/one/a.md", "1"), hit("a.md", "2"), hit("a", "3")],
            5,
            |_| true,
        );
        assert_eq!(actual.len(), 1);
        assert_eq!(actual[0].text, "1");
        assert!(distinct_entries([hit("a.md", "1")], 0, |_| true).is_empty());
    }
}
