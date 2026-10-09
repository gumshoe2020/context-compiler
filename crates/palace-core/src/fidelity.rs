use crate::{
    rollup::{Entry, superseded_live, superseded_sources, voided},
    traits::DiaryReader,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub fn floor<D: DiaryReader>(entry: &Entry, diary: &D) -> Value
where
    D::Error: std::fmt::Display,
{
    let mut violations = Vec::new();
    let mut dangling = Vec::new();
    let result = (|| -> Result<(), String> {
        let names = diary.names().map_err(|e| e.to_string())?;
        let refs = superseded_sources(entry, &names);
        dangling = refs.dangling;
        for name in superseded_live(entry, &names, &voided(diary, &names)?) {
            match diary
                .read(&name)
                .map_err(|e| format!("loading {name}: {e}"))?
            {
                Some(text) => {
                    if Entry::parse(&name, text).tags.contains("no-compress") {
                        violations.push(name);
                    }
                }
                None => dangling.push(name),
            }
        }
        Ok(())
    })();
    let mut report = json!({"check":"floor","rollup":entry.name,"status":if violations.is_empty(){"ok"}else{"fail"},"violations":violations,"dangling":dangling});
    if let Err(reason) = result {
        report["status"] = json!("error");
        report["reason"] = json!(reason);
    }
    report
}
#[derive(Clone)]
struct Depth {
    depth: usize,
    chain: Vec<String>,
    primary: usize,
    total: usize,
}
pub fn depth<D: DiaryReader>(entry: &Entry, diary: &D) -> Value
where
    D::Error: std::fmt::Display,
{
    let result = diary.names().map_err(|e| e.to_string()).and_then(|names| {
        let voided = voided(diary, &names)?;
        Walk {
            diary,
            names: &names,
            voided: &voided,
            cache: BTreeMap::new(),
            stack: Vec::new(),
        }
        .descend(&entry.name)
    });
    match result {
        Ok(d) => {
            json!({"check":"depth","rollup":entry.name,"status":if d.depth<=1 {"ok"} else if d.primary>0 {"warn"}else{"fail"},"depth":d.depth,"chain":d.chain,"primary_share":if d.total==0 {1.0} else {python_round4(d.primary as f64/d.total as f64)}})
        }
        Err(reason) => {
            json!({"check":"depth","rollup":entry.name,"status":"error","reason":reason,"depth":null,"chain":[],"primary_share":null})
        }
    }
}
/// Python `round(x, 4)`: the exact binary value rounded half-to-even (1/32 → 0.0312).
pub fn python_round4(x: f64) -> f64 {
    format!("{x:.4}").parse().unwrap_or(x)
}
/// Depth traversal state: rollups' live (non-voided) superseded sources, memoised, cycle-checked.
struct Walk<'a, D> {
    diary: &'a D,
    names: &'a [String],
    voided: &'a crate::rollup::Voids,
    cache: BTreeMap<String, Depth>,
    stack: Vec<String>,
}
impl<D: DiaryReader> Walk<'_, D>
where
    D::Error: std::fmt::Display,
{
    fn descend(&mut self, name: &str) -> Result<Depth, String> {
        if self.stack.iter().any(|n| n == name) {
            let mut chain = self.stack.clone();
            chain.push(name.into());
            return Err(format!("cycle detected: {}", chain.join(" -> ")));
        }
        if let Some(d) = self.cache.get(name) {
            return Ok(d.clone());
        }
        if self.stack.len() >= 512 {
            return Err("depth traversal exceeds 512 entries".into());
        }
        let entry = self
            .diary
            .read(name)
            .map_err(|e| e.to_string())?
            .map(|text| Entry::parse(name, text));
        let result = match entry.filter(|e| e.is_rollup) {
            Some(entry) => self.rollup_depth(&entry)?,
            None => Depth {
                depth: 0,
                chain: vec![name.into()],
                primary: 1,
                total: 1,
            },
        };
        self.cache.insert(name.into(), result.clone());
        Ok(result)
    }
    fn rollup_depth(&mut self, entry: &Entry) -> Result<Depth, String> {
        let sources = superseded_live(entry, self.names, self.voided);
        let mut result = Depth {
            depth: 1,
            chain: vec![entry.name.clone()],
            primary: 0,
            total: sources.len(),
        };
        self.stack.push(entry.name.clone());
        for source in sources {
            let child = self.descend(&source)?;
            if child.depth == 0 {
                result.primary += 1;
            }
            if child.depth + 1 > result.depth {
                result.depth = child.depth + 1;
                result.chain = vec![entry.name.clone()];
                result.chain.extend(child.chain);
            }
        }
        self.stack.pop();
        Ok(result)
    }
}
/// Archived results re-based by `rebase_referenced_only`.
#[derive(Debug, Default, PartialEq)]
pub struct Rebased {
    pub floor: usize,
    pub depth: usize,
}
/// An archived report re-based for the referenced-only label (M5b item 0), which the Python
/// watcher misread as superseded sources: a floor `fail` whose violations all sit in a
/// referenced-only paragraph of its rollup becomes `ok` with no violations; a depth result whose
/// chain descends into a referenced-only entry of its rollup, when every source the rollup does
/// supersede is a primary (non-rollup) entry, becomes `ok`, depth 1, chain [rollup], share 1.0.
/// Every other result is kept as archived and the summary is recounted.
pub fn rebase_referenced_only<D: DiaryReader>(
    archived: &Value,
    entries: &[Entry],
    diary: &D,
) -> Result<(Value, Rebased), String>
where
    D::Error: std::fmt::Display,
{
    let names = diary.names().map_err(|e| e.to_string())?;
    let mut rebased = Rebased::default();
    let mut results = archived["results"].as_array().cloned().unwrap_or_default();
    for result in &mut results {
        let Some(entry) = entries.iter().find(|e| result["rollup"] == e.name.as_str()) else {
            continue;
        };
        let refs = superseded_sources(entry, &names);
        if result["check"] == "floor"
            && result["status"] == "fail"
            && all_in(&result["violations"], &refs.referenced_only)
        {
            result["status"] = json!("ok");
            result["violations"] = json!([]);
            rebased.floor += 1;
        } else if result["check"] == "depth"
            && result["status"] != "ok"
            && all_in(&json!([result["chain"][1]]), &refs.referenced_only)
            && all_primary(&refs.resolved, diary)?
        {
            result["status"] = json!("ok");
            result["depth"] = json!(1);
            result["chain"] = json!([entry.name]);
            result["primary_share"] = json!(1.0);
            rebased.depth += 1;
        }
    }
    let rollups = archived["summary"]["rollups"].as_u64().unwrap_or(0) as usize;
    Ok((report(rollups, results), rebased))
}
/// A non-empty array of names, every one of them in `referenced`.
fn all_in(names: &Value, referenced: &[String]) -> bool {
    names.as_array().is_some_and(|names| {
        !names.is_empty()
            && names
                .iter()
                .all(|n| referenced.iter().any(|r| n == r.as_str()))
    })
}
/// Every source exists and is not a rollup.
fn all_primary<D: DiaryReader>(sources: &[String], diary: &D) -> Result<bool, String>
where
    D::Error: std::fmt::Display,
{
    for name in sources {
        let text = diary.read(name).map_err(|e| e.to_string())?;
        if text.is_none_or(|text| Entry::parse(name, text).is_rollup) {
            return Ok(false);
        }
    }
    Ok(true)
}
pub fn report(rollups: usize, results: Vec<Value>) -> Value {
    let count = |status: &str| results.iter().filter(|r| r["status"] == status).count();
    let claims_fail = results
        .iter()
        .filter(|r| {
            r["check"] == "claim"
                && matches!(
                    r["status"].as_str(),
                    Some("reversal" | "unsupported" | "stale" | "absent-source" | "malformed")
                )
        })
        .count();
    json!({"summary":{"rollups":rollups,"fail":count("fail")+claims_fail,"warn":count("warn"),"error":count("error")},"results":results})
}
pub fn exit_code(report: &Value) -> i32 {
    if ["fail", "warn", "error"]
        .iter()
        .any(|k| report["summary"][k].as_u64().unwrap_or(0) > 0)
    {
        1
    } else {
        0
    }
}
#[cfg(test)]
mod tests {
    #[test]
    fn only_results_misread_through_a_referenced_only_paragraph_are_rebased() {
        use super::{Rebased, rebase_referenced_only};
        use crate::{rollup::Entry, test_support::Memory};
        use serde_json::json;
        let kept = "2026-09-16-bbbbbbbbbbbb";
        let listed = "2026-09-16-cccccccccccc";
        let old = "2026-09-10-dddddddddddd";
        let text = |label: &str| {
            format!("# ROLLUP: t\n## Supersedes\n- {listed}.md\n\n{label} {kept}.md, {old}.md\n")
        };
        let mut diary = Memory::default();
        diary.0.insert(listed.into(), "# L\nTags: t.".into());
        diary
            .0
            .insert(kept.into(), "# K\nTags: t, no-compress.".into());
        diary.0.insert(
            old.into(),
            "# ROLLUP: t — state as of 2026-09-11\nTags: t.".into(),
        );
        let archived = |violation: &str, child: &str| {
            json!({"summary":{"rollups":1,"fail":1,"warn":1,"error":0},"results":[
                {"check":"floor","rollup":"r","status":"fail","violations":[violation],"dangling":[]},
                {"check":"depth","rollup":"r","status":"warn","depth":2,"chain":["r",child],"primary_share":0.5},
                {"check":"canary","rollup":"r","status":"absent"}]})
        };
        let entries = [Entry::parse("r", text("Referenced, NOT superseded:"))];
        let (report, n) = rebase_referenced_only(&archived(kept, old), &entries, &diary).unwrap();
        assert_eq!(n, Rebased { floor: 1, depth: 1 });
        assert_eq!(report["results"][0]["status"], "ok");
        assert_eq!(report["results"][0]["violations"], json!([]));
        assert_eq!(
            report["results"][1],
            json!({"check":"depth","rollup":"r","status":"ok","depth":1,"chain":["r"],"primary_share":1.0})
        );
        assert_eq!(report["results"][2], archived(kept, old)["results"][2]);
        assert_eq!(
            report["summary"],
            json!({"rollups":1,"fail":0,"warn":0,"error":0})
        );
        for (violation, child, entries) in [
            (listed, listed, entries.clone()),
            (kept, old, [Entry::parse("r", text("Kept:"))]),
        ] {
            let archive = archived(violation, child);
            let (report, n) = rebase_referenced_only(&archive, &entries, &diary).unwrap();
            assert_eq!((report, n), (archive, Rebased::default()), "{violation}");
        }
        diary.0.insert(
            listed.into(),
            "# ROLLUP: t — state as of 2026-09-15\nTags: t.".into(),
        );
        let (_, n) = rebase_referenced_only(&archived(kept, old), &entries, &diary).unwrap();
        assert_eq!(n.depth, 0, "a superseded rollup keeps the archived depth");
    }
    #[test]
    fn primary_share_rounds_like_python() {
        assert_eq!(super::python_round4(1.0 / 32.0), 0.0312);
        assert_eq!(super::python_round4(0.03135), 0.0314);
        assert_eq!(super::python_round4(2.0 / 3.0), 0.6667);
    }
}
