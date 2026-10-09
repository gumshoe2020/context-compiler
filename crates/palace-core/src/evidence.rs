pub const BOOKKEEPING_TAGS: &[&str] = &[
    "kg-audit",
    "kg-precision-audit",
    "graph-write",
    "alias-gap",
    "palace-graph-wiring",
    "kg-bookkeeping",
];
/// The adapter supplies the source entry header, or the hit text when unavailable.
pub fn is_bookkeeping(header: &str) -> bool {
    let head: String = header
        .chars()
        .scan(0, |units, c| {
            *units += c.len_utf16();
            (*units <= 800).then_some(c)
        })
        .collect::<String>()
        .to_lowercase();
    if let Some(title) = head.strip_prefix('#') {
        let title = title.trim_start();
        if [
            "kg precision audit",
            "kg alias gap",
            "kg cleanup",
            "kg seeded",
        ]
        .iter()
        .any(|prefix| title.starts_with(prefix))
            || (title.starts_with("rollup:")
                && title
                    .lines()
                    .next()
                    .unwrap_or("")
                    .contains("graph relations"))
        {
            return true;
        }
    }
    let tag_start = [head.find("tag:"), head.find("tags:")]
        .into_iter()
        .flatten()
        .min();
    let Some(start) = tag_start else {
        return false;
    };
    let tags = head[start..]
        .split_once(':')
        .unwrap()
        .1
        .trim_start()
        .split('\n')
        .next()
        .unwrap_or("");
    tags.split([',', ';'])
        .any(|tag| BOOKKEEPING_TAGS.contains(&tag.trim().strip_suffix('.').unwrap_or(tag.trim())))
}
