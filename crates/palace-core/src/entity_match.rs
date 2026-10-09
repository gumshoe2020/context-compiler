//! Per-turn node selection (M5c item 1): the palace-context-hook's entity match, ported. Message
//! tokens (lowercase ASCII alphanumeric runs, at least three characters, a stop-list dropped, a
//! trailing `s` folded off tokens longer than three) are matched against each live node's id and
//! its curated aliases (`entities.properties.aliases`). Alias words skip the stop-list; alias
//! words under three characters (acronyms such as "pb") match the message's raw words exactly.
//! A node scores one hit per matching token; nodes are ordered by hits (ties keep their input
//! order) and at most `MAX_MATCHED_NODES` are returned. Pure.
use std::collections::BTreeSet;

pub const MAX_MATCHED_NODES: usize = 10;

/// Words frequent enough to fire on most messages; they never match a node name on their own.
pub const STOPWORDS: &[&str] = &[
    "the", "and", "for", "not", "was", "are", "but", "you", "can", "all", "has", "had", "have",
    "this", "that", "with", "from", "what", "when", "where", "which", "will", "would", "could",
    "should", "there", "their", "them", "then", "than", "some", "more", "over", "into", "only",
    "also", "very", "much", "time", "good", "need", "want", "make", "just", "like", "been",
    "being", "does", "get", "got", "out", "now", "how", "why", "who", "its", "one", "two", "new",
    "old", "day", "way", "see", "say", "said", "use", "used", "using", "off", "per", "each",
    "still", "here", "about", "after", "before", "other", "while", "right", "work", "working",
    "thing", "things", "know", "think", "yes", "yeah", "okay", "let", "back", "next", "last",
    "first", "week", "today", "tomorrow", "well", "any", "may", "might",
];

/// A live graph node: its id and its curated aliases (none when absent or unreadable).
#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub id: String,
    pub aliases: Vec<String>,
}

/// The ids of the nodes the message touches, best first, at most `MAX_MATCHED_NODES`.
pub fn match_node_ids(message: &str, nodes: &[Node]) -> Vec<String> {
    let raw: BTreeSet<String> = words(message).collect();
    if raw.is_empty() {
        return Vec::new();
    }
    let tokens = canon_tokens(message);
    let mut scored: Vec<(&str, usize)> = nodes
        .iter()
        .map(|node| (node.id.as_str(), hits(node, &tokens, &raw)))
        .filter(|(_, hits)| *hits > 0)
        .collect();
    scored.sort_by_key(|s| std::cmp::Reverse(s.1));
    scored
        .into_iter()
        .take(MAX_MATCHED_NODES)
        .map(|(id, _)| id.to_owned())
        .collect()
}

/// The canonical tokens of `text`: words of three or more characters, not stop-words, folded.
pub fn canon_tokens(text: &str) -> BTreeSet<String> {
    words(text)
        .filter(|w| w.len() >= 3 && !STOPWORDS.contains(&w.as_str()))
        .map(|w| fold(&w))
        .collect()
}

/// One hit per short alias word found among the message's raw words, plus one per node token
/// (id tokens and long alias words, as a set) among the message's canonical tokens.
fn hits(node: &Node, tokens: &BTreeSet<String>, raw: &BTreeSet<String>) -> usize {
    let mut node_tokens = canon_tokens(&node.id);
    let mut hits = 0;
    for word in node.aliases.iter().flat_map(|alias| words(alias)) {
        if word.len() < 3 {
            hits += usize::from(raw.contains(&word));
        } else {
            node_tokens.insert(fold(&word));
        }
    }
    hits + node_tokens.intersection(tokens).count()
}

/// Lowercased runs of ASCII letters and digits (every other character separates).
fn words(text: &str) -> impl Iterator<Item = String> + '_ {
    let lower = text.to_lowercase();
    lower
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>()
        .into_iter()
}

/// The naive de-plural: a trailing `s` off a word longer than three characters.
fn fold(word: &str) -> String {
    match word.strip_suffix('s') {
        Some(stem) if word.len() > 3 => stem.to_owned(),
        _ => word.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: &str, aliases: &[&str]) -> Node {
        Node {
            id: id.into(),
            aliases: aliases.iter().map(|a| (*a).to_owned()).collect(),
        }
    }

    #[test]
    fn tokens_fold_plurals_and_drop_stop_words_and_short_words() {
        let tokens: Vec<String> = canon_tokens("The Guilds and their KG, at 9am: pass")
            .into_iter()
            .collect();
        assert_eq!(tokens, ["9am", "guild", "pas"]);
        assert!(canon_tokens("is it ok?").is_empty());
    }

    #[test]
    fn aliases_score_and_short_aliases_match_raw_words() {
        let nodes = [
            node(
                "palace-graph-substrate",
                &["kg", "knowledge graph", "write gate"],
            ),
            node("prayer-breakfast", &["pb"]),
            node("guild-system", &[]),
        ];
        assert_eq!(
            match_node_ids("the KG write gate for the guilds", &nodes),
            ["palace-graph-substrate", "guild-system"]
        );
        assert_eq!(
            match_node_ids("PB on Friday?", &nodes),
            ["prayer-breakfast"]
        );
        assert!(match_node_ids("...", &nodes).is_empty());
    }

    #[test]
    fn ties_keep_input_order_and_at_most_ten_nodes_match() {
        let nodes: Vec<Node> = (0..12).map(|i| node(&format!("orbit-{i}"), &[])).collect();
        let matched = match_node_ids("orbit", &nodes);
        assert_eq!(matched.len(), MAX_MATCHED_NODES);
        assert_eq!(matched[0], "orbit-0");
        assert_eq!(matched[9], "orbit-9");
    }
}
