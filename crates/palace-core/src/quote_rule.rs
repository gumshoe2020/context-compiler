//! M5b-10 item 1: the quote rule as `prompts/rollup.md` states it, pinned to the code. The prompt
//! carries a marked block (a source, a claim that passes, a claim that fails, and, M5b-11 item 5,
//! a claim that passes by the text around its quote alone); `check_examples`
//! gives each claim the validator's own verdict (`quote_relevance::quote_supports`), so editing the
//! prompt without the rule, or the rule without the prompt, fails. Pure: the caller reads the file.
use crate::claim::{self, Claim};
use crate::quote_relevance::quote_supports;

pub const OPEN: &str = "<!-- quote-rule-examples";
pub const CLOSE: &str = "<!-- /quote-rule-examples -->";

/// The pinned examples: the source text and the three claims.
pub struct Examples {
    pub source: String,
    pub passes: Claim,
    pub fails: Claim,
    /// Passes although its quote names only the subject: the text around it carries the object.
    pub around: Claim,
}
pub const AROUND: &str = "passes by the text around its quote:";

/// The marked block of `policy`, parsed; an error names what is missing.
pub fn examples(policy: &str) -> Result<Examples, String> {
    let start = policy.find(OPEN).ok_or("no quote-rule-examples block")?;
    let block = &policy[start..];
    let end = block
        .find(CLOSE)
        .ok_or("quote-rule-examples block not closed")?;
    let field = |key: &str| {
        block[..end]
            .lines()
            .find_map(|l| l.trim().strip_prefix(key).map(str::trim))
            .ok_or(format!("quote-rule-examples: no `{key}` line"))
    };
    let source = field("source:")?.trim_matches('"').to_owned();
    Ok(Examples {
        source,
        passes: one_claim(field("passes:")?)?,
        fails: one_claim(field("fails:")?)?,
        around: one_claim(field(AROUND)?)?,
    })
}

fn one_claim(line: &str) -> Result<Claim, String> {
    let mut parsed = claim::parse(&format!("## Claims\n{line}\n"));
    match (parsed.claims.len(), parsed.malformed.first()) {
        (1, None) => Ok(parsed.claims.remove(0)),
        (_, Some(m)) => Err(format!("quote-rule-examples: {}", m.reason)),
        _ => Err(format!("quote-rule-examples: not one claim: {line}")),
    }
}

/// `Ok` when the passing claims' quotes are verbatim and support them, the failing claim's quote
/// is verbatim and does not, and the `around` claim's quote does not support it on its own: the
/// prompt's words agree with `quote_supports`.
pub fn check_examples(policy: &str) -> Result<(), String> {
    let e = examples(policy)?;
    let around = e.around.quote.as_deref().unwrap_or_default();
    if quote_supports(&e.around, around, around) {
        return Err(format!(
            "quote-rule-examples: {} passes by its quote alone",
            e.around.number
        ));
    }
    for (claim, expected) in [(&e.passes, true), (&e.fails, false), (&e.around, true)] {
        let quote = claim
            .quote
            .as_deref()
            .ok_or("quote-rule-examples: a claim has no quote")?;
        if !e.source.contains(quote) {
            return Err(format!(
                "quote-rule-examples: {} quote not in the source",
                claim.number
            ));
        }
        if quote_supports(claim, quote, &e.source) != expected {
            let verdict = if expected { "fails" } else { "passes" };
            return Err(format!(
                "quote-rule-examples: {} {verdict} the rule",
                claim.number
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn policy() -> String {
        std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../prompts/rollup.md"
        ))
        .unwrap()
    }
    #[test]
    fn the_prompt_examples_agree_with_the_rule() {
        assert_eq!(check_examples(&policy()), Ok(()));
        let e = examples(&policy()).unwrap();
        assert_eq!(
            (e.passes.number.as_str(), e.fails.number.as_str()),
            ("C1", "C2")
        );
    }
    #[test]
    fn the_text_around_the_quote_is_pinned() {
        let p = policy();
        assert_eq!(examples(&p).unwrap().around.number, "C3");
        let far = p.replace(
            "Sam replied under the Maplewood post.",
            &format!("Sam replied under the Maplewood post.{}", " x".repeat(400)),
        );
        let err = "quote-rule-examples: C3 fails the rule";
        assert_eq!(check_examples(&far), Err(err.into()));
        let alone = p.replace(
            "| the Maplewood post — 2026-09-21 — 2026-09-21-aaaaaaaaaaaa.md — quote: \"Sam will report",
            "| riverbend — 2026-09-21 — 2026-09-21-aaaaaaaaaaaa.md — quote: \"Sam will report",
        );
        let err = "quote-rule-examples: C3 passes by its quote alone";
        assert_eq!(check_examples(&alone), Err(err.into()));
    }
    #[test]
    fn a_drifted_prompt_fails() {
        let p = policy();
        let drifted = p.replace(
            "quoted | Article 12:4 in the first-round comment",
            "quoted | the Maplewood post",
        );
        assert_eq!(
            check_examples(&drifted),
            Err("quote-rule-examples: C2 passes the rule".into())
        );
        let start = p.find(OPEN).unwrap();
        let without = &p[..start];
        assert_eq!(
            check_examples(without).unwrap_err(),
            "no quote-rule-examples block"
        );
    }
}
