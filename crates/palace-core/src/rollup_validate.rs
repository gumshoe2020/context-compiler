//! Rollup stages 4–5: the write prompt and the Rust validator (the parts that need no IO).
//! The CLI adds the gate dry-run and floor/depth/canary on the candidate text.
use crate::{
    batch::parse_batch,
    claim::{self, Claim},
    proposal::Proposal,
    quote_relevance::distinctive_anchors,
    rollup::{
        Entry, REFERENCED_ONLY_LABEL, superseded_sources, unlabelled_paragraphs, unpinned_labels,
    },
    rollup_cluster::Cluster,
};

pub const WRITE_MARKER: &str = "=== TASK: WRITE THE ROLLUP BODY ===";
pub const MIN_CHARS: usize = 2_000;
pub const MAX_CHARS: usize = 40_000;
pub const MIN_CLAIMS: usize = 3;
const REQUIRED: &[&str] = &[
    "## Claims",
    "## Current consolidated truth",
    "## Supersedes",
    "## Open items",
    "## Graph relations (proposed)",
];

pub struct Target<'a> {
    pub thread: &'a str,
    pub as_of: &'a str,
}
impl Target<'_> {
    pub fn title(&self) -> String {
        format!("ROLLUP: {} — state as of {}", self.thread, self.as_of)
    }
}
pub fn write_prompt(
    policy: &str,
    target: &Target,
    cluster: &Cluster,
    retry: Option<&str>,
) -> String {
    let mut text = format!(
        "{policy}\n\n{WRITE_MARKER}\nReturn ONLY the rollup entry, first line exactly:\n# {}\nThen, in order, these sections: {}. Claims use the pinned grammar; cite only PRIMARY or REFERENCE ONLY sources below, never BOOKKEEPING; Supersedes lists only PRIMARY sources below, one \"- <file>\" line each, then any entry it points at without superseding in a later paragraph labelled exactly \"{REFERENCED_ONLY_LABEL}\", one \"- <file>\" line each under the label line (the policy's SUPERSEDES LAYOUT); each relation carries \"claim\": \"Cn\". 2,000–40,000 characters.\n\n=== CLUSTER ({} entries) ===\n",
        target.title(),
        REQUIRED.join(", "),
        cluster.members.len()
    );
    for member in &cluster.members {
        let voided = cluster.voided.contains(&member.entry.name);
        let role = match member.label() {
            "primary" if voided => {
                "PRIMARY (an earlier rollup's supersession of it was voided: not yet compressed)"
            }
            "primary" => "PRIMARY",
            "reference-only" if cluster.referenced.contains(&member.entry.name) => {
                "REFERENCE ONLY (an earlier rollup referenced it without superseding: cite, never supersede)"
            }
            "reference-only" => "REFERENCE ONLY (no-compress: cite, never supersede)",
            _ => "BOOKKEEPING (graph bookkeeping: never cite or supersede)",
        };
        text.push_str(&format!(
            "\n--- {}.md [{role}] ---\n{}\n",
            member.entry.name, member.entry.text
        ));
    }
    if let Some(report) = retry {
        text.push_str(&format!(
            "\n=== YOUR PREVIOUS ATTEMPT FAILED VALIDATION; fix every problem ===\n{report}\n"
        ));
    }
    text
}
/// A live specific (spec §4.1) that a rollup's sources state: its claim id and exact value.
#[derive(Clone, Debug)]
pub struct Specific {
    pub id: String,
    pub subject: String,
    pub predicate: String,
    pub object: String,
    pub date: String,
    /// The entries that share its provenance (see [`lineage`]), bare names, its source first.
    pub lineage: Vec<String>,
}
/// SETTLE-1g §1: the entries that share the provenance of a specific stated by `source`: the
/// source itself; each entry the source's `## Supersedes` names (superseded or referenced only);
/// and each of `entries` whose `## Supersedes` names the source (the rollup that carries it).
/// `entries` are the texts at hand (the cluster's members and its prior rollups). Bare names,
/// the source first.
pub fn lineage(source: &str, entries: &[&Entry], names: &[String]) -> Vec<String> {
    let source = bare(source);
    let named = |e: &Entry| {
        let refs = superseded_sources(e, names);
        refs.resolved.into_iter().chain(refs.referenced_only)
    };
    let mut found = vec![source.to_owned()];
    for e in entries {
        let named: Vec<String> = named(e).collect();
        let shared = match e.name == source {
            true => named,
            false if named.iter().any(|n| n == source) => vec![e.name.clone()],
            false => continue,
        };
        for name in shared {
            if !found.contains(&name) {
                found.push(name);
            }
        }
    }
    found
}
fn bare(name: &str) -> &str {
    name.trim().strip_suffix(".md").unwrap_or(name.trim())
}
/// The write prompt's list of the live specifics the cluster states, with the rule (spec §5);
/// empty when there are none, so a rollup without distilled specifics gets the prompt as before.
pub fn specifics_section(specifics: &[Specific]) -> String {
    if specifics.is_empty() {
        return String::new();
    }
    let mut text = String::from(
        "\n=== LIVE SPECIFICS (exact values the cluster states) ===\nA rollup may not paraphrase a specific: cite it by its claim id and copy its value exactly (a claim line with the same subject and predicate that cites the source named after the value, or an entry that source supersedes or a rollup that carries it, carries this exact object; a line naming the id carries the exact value). A rollup that states a different value fails validation.\n",
    );
    for s in specifics {
        text.push_str(&format!(
            "- {}: {} | {} | {} ({}) — from {}.md\n",
            s.id,
            s.subject,
            s.predicate,
            s.object,
            s.date,
            s.lineage.first().map_or("?", String::as_str)
        ));
    }
    text
}
/// SETTLE-1b (spec §5, test 13), deterministic, no judge. A rollup may not paraphrase a live
/// specific its sources state:
/// 1. a claim line not marked superseded or corrected, with the specific's subject and predicate
///    (normalised as claim ids normalise them) and a source in the specific's lineage (SETTLE-1g
///    §1: shared provenance), must carry the specific's value as its object, exactly (outer
///    space ignored); a claim sharing only subject and predicate is a claim of its own;
/// 2. a line that names the specific's claim id must carry the value verbatim.
///
/// Each breach is a problem, so the rollup fails validation and, after its attempts, is ROLLUP
/// INVALID (exit 4: page, keep the attempts, the night goes on).
pub fn check_specifics(v: &mut Validation, body: &str, specifics: &[Specific]) {
    for s in specifics {
        let (subject, predicate) = (
            claim::normalise_entity(&s.subject),
            claim::normalise_predicate(&s.predicate),
        );
        for c in v.claims.iter().filter(|c| c.superseded_by.is_none()) {
            let same = claim::normalise_entity(&c.subject) == subject
                && claim::normalise_predicate(&c.predicate) == predicate;
            let shared = s.lineage.iter().any(|e| bare(e) == bare(&c.source));
            if same && shared && c.object.trim() != s.object.trim() {
                v.problems.push(format!(
                    "{}: restates the live specific {} ({} | {} | {}) as \"{}\": copy its value exactly and cite {}",
                    c.number, s.id, s.subject, s.predicate, s.object, c.object, s.id
                ));
            }
        }
        for (i, line) in body.lines().enumerate() {
            if line.contains(&s.id) && !line.contains(s.object.trim()) {
                v.problems.push(format!(
                    "line {}{CITES}{}{WITHOUT_VALUE}\"{}\"",
                    i + 1,
                    s.id,
                    s.object
                ));
            }
        }
    }
}
const CITES: &str = ": cites the live specific ";
const WITHOUT_VALUE: &str = " without its exact value ";
/// `(line, handle)` of a "line <n>: cites the live specific <handle> without its exact value …"
/// problem of [`check_specifics`], else `None` (SETTLE-1h: the problem repair mends).
pub fn missing_value_of(problem: &str) -> Option<(usize, &str)> {
    let (line, rest) = problem.strip_prefix("line ")?.split_once(CITES)?;
    let (id, _) = rest.split_once(WITHOUT_VALUE)?;
    Some((line.parse().ok()?, id))
}
#[derive(Debug, Default)]
pub struct Validation {
    pub problems: Vec<String>,
    pub claims: Vec<Claim>,
    pub relations: Vec<Proposal>,
    pub superseded: Vec<String>,
}
impl Validation {
    pub fn passed(&self) -> bool {
        self.problems.is_empty()
    }
}
pub fn validate(body: &str, target: &Target, cluster: &Cluster, names: &[String]) -> Validation {
    let mut v = Validation::default();
    let chars = body.chars().count();
    if !(MIN_CHARS..=MAX_CHARS).contains(&chars) {
        v.problems.push(format!(
            "size {chars} chars; must be {MIN_CHARS}–{MAX_CHARS}"
        ));
    }
    let first = body.lines().next().unwrap_or_default().trim();
    if first != format!("# {}", target.title()) {
        v.problems.push(format!(
            "first line must be `# {}`, got `{first}`",
            target.title()
        ));
    }
    for heading in REQUIRED {
        if !body.lines().any(|l| l.trim_end().starts_with(heading)) {
            v.problems.push(format!("missing section `{heading}`"));
        }
    }
    check_claims(&mut v, body, cluster);
    check_supersedes(&mut v, body, cluster, names);
    check_relations(&mut v, body);
    v
}
fn check_claims(v: &mut Validation, body: &str, cluster: &Cluster) {
    let parsed = claim::parse(body);
    for m in &parsed.malformed {
        v.problems.push(format!("line {}: {}", m.line, m.reason));
    }
    if parsed.claims.len() < MIN_CLAIMS {
        v.problems.push(format!(
            "{} claims; at least {MIN_CLAIMS} required",
            parsed.claims.len()
        ));
    }
    for c in &parsed.claims {
        match cluster.member(&c.source) {
            Some(m) if m.citable => check_quote(v, c, &m.entry.text),
            Some(_) => v.problems.push(format!(
                "{}: source {} is graph bookkeeping, not citable",
                c.number, c.source
            )),
            None => v.problems.push(format!(
                "{}: source {} is not a citable member of the cluster",
                c.number, c.source
            )),
        }
    }
    v.claims = parsed.claims;
}
/// The problem of a quoted claim whose object holds no name, number or content word: no quote can
/// support it, so it is not asked of a repair (M5b-11 item 5).
pub const NO_WORD: &str = "the object has no word a quote could carry";
/// A quote must be verbatim in the source and support the claim (M4b.1 S-2).
fn check_quote(v: &mut Validation, c: &Claim, source: &str) {
    let Some(quote) = &c.quote else {
        return;
    };
    if distinctive_anchors(c).is_empty() {
        v.problems.push(format!("{}: {NO_WORD}", c.number));
    } else if !source.contains(quote.as_str()) {
        v.problems
            .push(format!("{}: quote not verbatim in {}", c.number, c.source));
    } else if !claim::quote_supports(c, quote, source) {
        v.problems
            .push(format!("{}: quote does not support claim", c.number));
    }
}
fn check_supersedes(v: &mut Validation, body: &str, cluster: &Cluster, names: &[String]) {
    let entry = Entry::parse("candidate", body.into());
    for named in unlabelled_paragraphs(&entry, names) {
        v.problems.push(format!(
            "Supersedes: a later paragraph names {} without a label; list each referenced entry as \"- <file>\" under the line \"{REFERENCED_ONLY_LABEL}\" and name no entry elsewhere in the section (a reason goes on the entry's own line)",
            named.join(", ")
        ));
    }
    for label in unpinned_labels(&entry) {
        v.problems.push(format!(
            "Supersedes: referenced-only label \"{label}\" is not the pinned one; start its paragraph with \"{REFERENCED_ONLY_LABEL}\""
        ));
    }
    let refs = superseded_sources(&entry, names);
    if refs.resolved.is_empty() {
        v.problems.push("`## Supersedes` resolves no source".into());
    }
    for name in &refs.dangling {
        v.problems
            .push(format!("Supersedes names unknown entry {name}"));
    }
    for name in &refs.resolved {
        if !cluster.member(name).is_some_and(|m| m.supersedable) {
            v.problems.push(format!(
                "Supersedes {name}: not a non-no-compress, non-ROLLUP cluster entry"
            ));
        }
    }
    v.superseded = refs.resolved;
}
fn check_relations(v: &mut Validation, body: &str) {
    let Some(block) = relations_block(body) else {
        v.problems.push("missing ```json relations block".into());
        return;
    };
    match parse_batch(block, None) {
        Err(e) => v.problems.push(format!("relations block: {e}")),
        Ok(relations) => {
            if !(1..=3).contains(&relations.len()) {
                v.problems
                    .push(format!("{} relations; 1–3 required", relations.len()));
            }
            for (i, r) in relations.iter().enumerate() {
                let cited = r
                    .claim
                    .as_deref()
                    .and_then(|n| v.claims.iter().find(|c| c.number == n));
                if cited.is_none() {
                    v.problems.push(format!(
                        "relation {i} must cite an existing claim (\"claim\": \"Cn\")"
                    ));
                }
            }
            v.problems
                .extend(crate::pairing::problems(&v.claims, &relations));
            v.relations = relations;
        }
    }
}
/// Body of the first ```` ```json relations ```` fence.
pub fn relations_block(body: &str) -> Option<&str> {
    let start = body.find("```json relations")?;
    let rest = &body[start..];
    let open = rest.find('\n')? + 1;
    let close = rest[open..].find("```")?;
    Some(&rest[open..open + close])
}
/// Resolve each relation's `claim` number to the claim's stable id.
pub fn cite_claims(relations: &mut [Proposal], claims: &[Claim]) {
    for r in relations {
        r.claim_id = r
            .claim
            .as_deref()
            .and_then(|n| claims.iter().find(|c| c.number == n))
            .map(|c| c.id.clone());
    }
}
/// Title line without `# ` and the text after it, as the diary plugin takes them.
pub fn split_title(body: &str) -> (String, String) {
    let (first, rest) = body.split_once('\n').unwrap_or((body, ""));
    (
        first.trim().trim_start_matches('#').trim().to_owned(),
        rest.to_owned(),
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::rollup_cluster::Member;

    const PRIMARY: &str = "2026-09-20-aaaaaaaaaaaa";
    const KEEP: &str = "2026-09-20-bbbbbbbbbbbb";
    const AUDIT: &str = "2026-09-20-cccccccccccc";

    fn cluster() -> Cluster {
        let primary = format!(
            "# Alice likes Rust\nTags: t.\nAlice said she likes Rust a lot.\n{}\nUnrelated weather note.",
            "-".repeat(700)
        );
        let members = [
            (PRIMARY, primary.as_str()),
            (
                KEEP,
                "# Callum verbatim\nTags: t, no-compress.\nAlice uses Rust daily.",
            ),
            (
                AUDIT,
                "# KG precision audit 2026-09-20\nTags: t.\nAlice likes Rust (audited).",
            ),
        ]
        .into_iter()
        .map(|(name, text)| Member::new(Entry::parse(name, text.into())))
        .collect();
        Cluster {
            thread: "t".into(),
            members,
            ..Default::default()
        }
    }
    fn body(c3_source: &str) -> String {
        let claims = format!(
            "## Claims\n- C1. Alice | likes | Rust — 2026-09-20 — {PRIMARY}.md — quote: \"likes Rust\"\n- C2. Alice | likes | Rust a lot — 2026-09-20 — {PRIMARY}.md\n- C3. Alice | uses | Rust — 2026-09-20 — {c3_source}.md\n"
        );
        format!(
            "# ROLLUP: t — state as of 2026-09-23\n{claims}## Current consolidated truth\nx\n## Supersedes\n- {PRIMARY}.md\n## Open items\n- none\n## Graph relations (proposed)\n"
        )
    }
    fn problems(body: &str) -> Vec<String> {
        let target = Target {
            thread: "t",
            as_of: "2026-09-22",
        };
        let names = [PRIMARY, KEEP, AUDIT].map(String::from);
        validate(body, &target, &cluster(), &names).problems
    }
    fn claim_problems(body: &str) -> Vec<String> {
        problems(body)
            .into_iter()
            .filter(|p| p.starts_with('C'))
            .collect::<Vec<_>>()
    }
    #[test]
    fn irrelevant_verbatim_quote_is_a_named_problem() {
        let text =
            body(PRIMARY).replace("quote: \"likes Rust\"", "quote: \"Unrelated weather note\"");
        assert_eq!(claim_problems(&text), ["C1: quote does not support claim"]);
    }
    #[test]
    fn an_object_no_quote_can_carry_is_named_and_not_repaired() {
        let text = body(PRIMARY).replace(
            "- C1. Alice | likes | Rust — ",
            "- C1. Alice | will | do so if she can — ",
        );
        let problems = claim_problems(&text);
        assert_eq!(problems, [format!("C1: {NO_WORD}")]);
        assert!(crate::rollup_repair::claim_problems(&text, &problems).is_none());
    }
    #[test]
    fn claim_may_cite_a_no_compress_entry() {
        assert!(claim_problems(&body(KEEP)).is_empty());
    }
    #[test]
    fn claim_citing_bookkeeping_is_a_named_problem() {
        assert_eq!(
            claim_problems(&body(AUDIT)),
            [format!(
                "C3: source {AUDIT}.md is graph bookkeeping, not citable"
            )]
        );
    }
    fn supersedes_problems(section: &str) -> Vec<String> {
        let text = body(PRIMARY).replace(&format!("## Supersedes\n- {PRIMARY}.md\n"), section);
        problems(&text)
            .into_iter()
            .filter(|p| p.starts_with("Supersedes"))
            .collect()
    }
    /// M5b item 0: a later paragraph naming files needs a label, or it would silently become
    /// superseded sources.
    #[test]
    fn a_later_supersedes_paragraph_without_a_label_is_a_problem() {
        let labelled = format!(
            "## Supersedes\n- {PRIMARY}.md\n\n{REFERENCED_ONLY_LABEL} {KEEP}.md\n\nNo INVALIDATE CANDIDATE.\n"
        );
        assert!(supersedes_problems(&labelled).is_empty());
        let variant = labelled.replace(REFERENCED_ONLY_LABEL, "Referenced, NOT superseded:");
        let found = supersedes_problems(&variant);
        assert!(
            found.len() == 1 && found[0].contains("is not the pinned one"),
            "{found:?}"
        );
        let bare = labelled.replace(REFERENCED_ONLY_LABEL, "Kept (no-compress):");
        let found = supersedes_problems(&bare);
        assert!(
            found
                .iter()
                .any(|p| p.starts_with("Supersedes: a later paragraph names")
                    && p.contains(KEEP)
                    && p.contains(REFERENCED_ONLY_LABEL)),
            "{found:?}"
        );
    }
    /// M5b item 0: the policy and the write prompt pin exactly one referenced-only label.
    #[test]
    fn the_prompts_pin_the_referenced_only_label() {
        let policy = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../prompts/rollup.md"
        ))
        .unwrap();
        assert_eq!(policy.matches(REFERENCED_ONLY_LABEL).count(), 1);
        assert!(!policy.contains("Referenced-only") && !policy.contains("NOT superseded"));
        let target = Target {
            thread: "t",
            as_of: "2026-09-22",
        };
        let prompt = write_prompt("policy", &target, &cluster(), None);
        assert!(prompt.contains(&format!(
            "labelled exactly \"{REFERENCED_ONLY_LABEL}\", one \"- <file>\" line each"
        )));
    }
    #[test]
    fn supersedes_rejects_no_compress_and_bookkeeping() {
        for name in [KEEP, AUDIT] {
            let text = body(PRIMARY).replace(
                &format!("## Supersedes\n- {PRIMARY}.md"),
                &format!("## Supersedes\n- {name}.md"),
            );
            assert!(
                problems(&text)
                    .iter()
                    .any(|p| p.starts_with(&format!("Supersedes {name}"))),
                "{name}"
            );
        }
    }
    #[test]
    fn a_rollup_that_restates_a_live_specific_differently_fails() {
        let specific = Specific {
            id: "c_0123456789ab".into(),
            subject: "Larkspur ferry inspection".into(),
            predicate: "is scheduled on".into(),
            object: "May 21".into(),
            date: "2031-04-03".into(),
            lineage: vec![PRIMARY.into()],
        };
        let line = |object: &str| {
            format!(
                "## Claims\n- C1. larkspur ferry  inspection | is scheduled on | {object} — 2031-04-03 — {PRIMARY}.md\n"
            )
        };
        let check = |body: &str| {
            let mut v = Validation {
                claims: claim::parse(body).claims,
                ..Default::default()
            };
            check_specifics(&mut v, body, std::slice::from_ref(&specific));
            v.problems
        };
        assert!(check(&line("May 21")).is_empty());
        let problems = check(&line("late May"));
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].starts_with("C1: restates the live specific c_0123456789ab"));
        assert!(
            check(&format!(
                "{}\nThe date stays c_0123456789ab: May 21.",
                line("May 21")
            ))
            .is_empty()
        );
        let cited = check(&format!(
            "{}\nThe date stays c_0123456789ab: the 21st.",
            line("May 21")
        ));
        assert_eq!(cited.len(), 1, "{cited:?}");
        let superseded = "## Claims\n- C2. (superseded by C1) Larkspur ferry inspection | is scheduled on | May 14 — 2031-04-02 — x.md\n";
        assert!(
            check(superseded).is_empty(),
            "a marked old value is history, not a restatement"
        );
        assert!(specifics_section(&[]).is_empty());
        assert!(specifics_section(std::slice::from_ref(&specific)).contains(
            "- c_0123456789ab: Larkspur ferry inspection | is scheduled on | May 21 (2031-04-03)"
        ));
    }
    const ROLLUP: &str = "2031-04-09-dddddddddddd";
    fn found_by_mira(source: &str) -> Vec<String> {
        let entries = [
            Entry::parse(PRIMARY, "# Mira at the quarry\nMira found a fossil.".into()),
            Entry::parse(
                ROLLUP,
                format!(
                    "# ROLLUP: quarry — state as of 2031-04-09\n## Supersedes\n- {PRIMARY}.md\n"
                ),
            ),
        ];
        let names = [PRIMARY, KEEP, ROLLUP].map(String::from);
        let specific = Specific {
            id: "c_aaaaaaaaaaaa".into(),
            subject: "Mira".into(),
            predicate: "found".into(),
            object: "a fossil".into(),
            date: "2031-04-03".into(),
            lineage: lineage(
                &format!("{PRIMARY}.md"),
                &entries.iter().collect::<Vec<_>>(),
                &names,
            ),
        };
        let body =
            format!("## Claims\n- C1. Mira | found | a lost key — 2031-04-04 — {source}.md\n");
        let mut v = Validation {
            claims: claim::parse(&body).claims,
            ..Default::default()
        };
        check_specifics(&mut v, &body, &[specific]);
        v.problems
    }
    /// SETTLE-1g §1: same subject and predicate pair a claim with a specific only on shared
    /// provenance: the specific's source, or an entry in its lineage.
    #[test]
    fn a_claim_pairs_with_a_specific_only_on_shared_provenance() {
        assert!(
            found_by_mira(KEEP).is_empty(),
            "a collision, not a restatement"
        );
        for source in [PRIMARY, ROLLUP] {
            let problems = found_by_mira(source);
            assert_eq!(problems.len(), 1, "{source}: {problems:?}");
            assert!(problems[0].starts_with("C1: restates the live specific c_aaaaaaaaaaaa"));
        }
    }
    #[test]
    fn a_specifics_lineage_is_its_source_what_it_names_and_what_names_it() {
        let names = [PRIMARY, KEEP, AUDIT, ROLLUP].map(String::from);
        let source = Entry::parse(
            KEEP,
            format!(
                "# Keep\n## Supersedes\n- {PRIMARY}.md\n\n{REFERENCED_ONLY_LABEL}\n- {AUDIT}.md\n"
            ),
        );
        let carrier = Entry::parse(ROLLUP, format!("# ROLLUP: t\n## Supersedes\n- {KEEP}.md\n"));
        let other = Entry::parse(PRIMARY, "# Other\nNo supersedes.".into());
        let entries = [&source, &carrier, &other];
        assert_eq!(
            lineage(&format!("{KEEP}.md"), &entries, &names),
            [KEEP, PRIMARY, AUDIT, ROLLUP]
        );
        assert_eq!(lineage(PRIMARY, &entries[2..], &names), [PRIMARY]);
    }
}
