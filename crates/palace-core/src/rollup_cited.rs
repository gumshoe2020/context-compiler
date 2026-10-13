//! SETTLE-1h §1: a prose line whose only problem is that it cites a live specific's handle
//! without the specific's exact value is mended before re-validation, with no model call: the
//! value is quoted right after the handle, `(c_0123456789ab)` becoming
//! `(c_0123456789ab: "May 21")`. Only that problem kind, only for handles among the live
//! specifics, only once per handle per line; every other byte is unchanged. Prose is any line
//! outside a `## Claims` section and outside a code fence.
use crate::{
    claim,
    rollup_validate::{Specific, missing_value_of},
};

/// One mended citation: the 1-based line and the handle.
#[derive(Clone, Debug, PartialEq)]
pub struct Mend {
    pub line: usize,
    pub id: String,
}

/// `body` with the exact value quoted after each mendable handle, and the mends made.
pub fn mend(body: &str, problems: &[String], specifics: &[Specific]) -> (String, Vec<Mend>) {
    let wanted = mendable(body, problems, specifics);
    if wanted.is_empty() {
        return (body.to_owned(), Vec::new());
    }
    let (mut out, mut mends) = (String::with_capacity(body.len()), Vec::new());
    for (i, line) in body.split_inclusive('\n').enumerate() {
        let mut line = line.to_owned();
        for (_, s) in wanted.iter().filter(|(n, _)| *n == i + 1) {
            if let Some(mended) = quote_after_handle(&line, s) {
                line = mended;
                mends.push(Mend {
                    line: i + 1,
                    id: s.id.clone(),
                });
            }
        }
        out.push_str(&line);
    }
    (out, mends)
}

/// Each `(line, specific)` to mend, once: a missing-value problem on a prose line whose problems
/// are all of that kind, for a handle among the live specifics.
fn mendable<'s>(
    body: &str,
    problems: &[String],
    specifics: &'s [Specific],
) -> Vec<(usize, &'s Specific)> {
    let prose = prose_lines(body);
    let mut found: Vec<(usize, &Specific)> = Vec::new();
    for (line, id) in problems.iter().filter_map(|p| missing_value_of(p)) {
        let Some(s) = specifics.iter().find(|s| s.id == id) else {
            continue;
        };
        let new = !found.iter().any(|(n, f)| *n == line && f.id == id);
        if new && prose.contains(&line) && only_missing_values(problems, line) {
            found.push((line, s));
        }
    }
    found
}

fn only_missing_values(problems: &[String], line: usize) -> bool {
    let prefix = format!("line {line}: ");
    problems
        .iter()
        .filter(|p| p.starts_with(&prefix))
        .all(|p| missing_value_of(p).is_some())
}

/// The 1-based numbers of the prose lines: outside a `## Claims` section and a code fence.
fn prose_lines(body: &str) -> Vec<usize> {
    let (mut claims, mut fenced) = (false, false);
    let mut prose = Vec::new();
    for (i, line) in body.lines().enumerate() {
        if line.starts_with("## ") {
            claims = claim::is_claims_heading(line);
        } else if line.trim_start().starts_with("```") {
            fenced = !fenced;
        } else if !claims && !fenced {
            prose.push(i + 1);
        }
    }
    prose
}

/// `line` with `: "<value>"` right after the first occurrence of the handle; `None` when the line
/// already holds the value or not the handle.
fn quote_after_handle(line: &str, s: &Specific) -> Option<String> {
    let value = s.object.trim();
    if line.contains(value) {
        return None;
    }
    let at = line.find(&s.id)? + s.id.len();
    Some(format!("{}: \"{value}\"{}", &line[..at], &line[at..]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rollup_validate::{Validation, check_specifics};

    fn specific(id: &str, object: &str) -> Specific {
        Specific {
            id: id.into(),
            subject: "Mira".into(),
            predicate: "found".into(),
            object: object.into(),
            date: "2031-05-02".into(),
            lineage: vec!["2031-05-02-aaaaaaaaaaaa".into()],
        }
    }
    fn specifics() -> Vec<Specific> {
        vec![
            specific("c_aaaaaaaaaaaa", "the brass compass"),
            specific("c_cccccccccccc", "a silver thimble"),
        ]
    }
    fn problems(body: &str, specifics: &[Specific]) -> Vec<String> {
        let mut v = Validation::default();
        check_specifics(&mut v, body, specifics);
        v.problems
    }
    /// Mends as `palace rollup` does: validate, mend, validate again.
    fn mended(body: &str) -> (String, Vec<Mend>, Vec<String>) {
        let (body, mends) = mend(body, &problems(body, &specifics()), &specifics());
        let left = problems(&body, &specifics());
        (body, mends, left)
    }
    const HEAD: &str =
        "# ROLLUP: quarry — state as of 2031-05-09\n\n## Current consolidated truth\n";

    #[test]
    fn a_paraphrase_is_mended_then_passes() {
        let body =
            format!("{HEAD}Mira dug up the old compass (c_aaaaaaaaaaaa) by the gate.\nNext.\n");
        let (out, mends, left) = mended(&body);
        assert_eq!(
            out,
            body.replace(
                "(c_aaaaaaaaaaaa)",
                "(c_aaaaaaaaaaaa: \"the brass compass\")"
            )
        );
        assert_eq!(
            mends,
            [Mend {
                line: 4,
                id: "c_aaaaaaaaaaaa".into()
            }]
        );
        assert!(left.is_empty(), "{left:?}");
    }
    #[test]
    fn a_line_holding_the_value_is_untouched() {
        let body = format!("{HEAD}Mira found the brass compass (c_aaaaaaaaaaaa).\n");
        assert!(problems(&body, &specifics()).is_empty());
        let (out, mends, _) = mended(&body);
        assert_eq!(out, body);
        assert!(mends.is_empty());
    }
    #[test]
    fn an_unknown_handle_is_not_mended_and_its_problem_stays() {
        let body = format!("{HEAD}Mira dug up the old compass (c_aaaaaaaaaaaa).\n");
        let found = problems(&body, &specifics());
        let unknown = [specific("c_cccccccccccc", "a silver thimble")];
        let (out, mends) = mend(&body, &found, &unknown);
        assert_eq!(out, body);
        assert!(mends.is_empty());
        assert_eq!(problems(&out, &specifics()), found);
    }
    #[test]
    fn two_missing_values_on_one_line_are_both_mended_once_each() {
        let body = format!(
            "{HEAD}The compass (c_aaaaaaaaaaaa) and the thimble (c_cccccccccccc), again c_aaaaaaaaaaaa.\n"
        );
        let (out, mends, left) = mended(&body);
        assert_eq!(
            out,
            format!(
                "{HEAD}The compass (c_aaaaaaaaaaaa: \"the brass compass\") and the thimble (c_cccccccccccc: \"a silver thimble\"), again c_aaaaaaaaaaaa.\n"
            )
        );
        assert_eq!(mends.len(), 2);
        assert!(left.is_empty(), "{left:?}");
    }
    #[test]
    fn only_prose_lines_whose_only_problem_it_is_are_mended() {
        let claims = "## Claims\n- C1. Mira | found | it (c_aaaaaaaaaaaa) — 2031-05-02 — a.md\n";
        let fenced = "```json relations\n[{\"claim\": \"c_aaaaaaaaaaaa\"}]\n```\n";
        for body in [format!("{HEAD}{claims}"), format!("{HEAD}{fenced}")] {
            let (out, mends, _) = mended(&body);
            assert_eq!(out, body);
            assert!(mends.is_empty());
        }
        let body = format!("{HEAD}The compass (c_aaaaaaaaaaaa).\n");
        let mut found = problems(&body, &specifics());
        found.push("line 4: another problem".into());
        assert_eq!(mend(&body, &found, &specifics()).0, body);
    }
}
