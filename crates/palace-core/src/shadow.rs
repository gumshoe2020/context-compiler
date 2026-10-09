//! The M5 shadow comparison (M5a item 1): old path against new, all pure. Fidelity: for each
//! rollup the newest Python watcher report covers, its `floor`, `depth` and per-canary statuses
//! against `palace watch`'s rows for the same rollup. Gate decisions: the substrate's recorded
//! accept/reject per relation against `palace propose-batch --dry-run`'s `ok`. Graph: every row
//! `palace doctor --shadow-diff` names. Then the night's outcome, its one line and REPORT.md.
use serde_json::{Value, json};

/// One place where old and new disagree, with both sides.
#[derive(Clone, Debug, PartialEq)]
pub struct Disagreement {
    pub kind: &'static str,
    pub subject: String,
    pub check: String,
    pub old: Value,
    pub new: Value,
}

impl Disagreement {
    /// One sentence naming the subject, the check and both sides.
    pub fn sentence(&self) -> String {
        format!(
            "{} {}: old {}, new {}",
            self.subject,
            self.check,
            side(&self.old),
            side(&self.new)
        )
    }
    pub fn json(&self) -> Value {
        json!({"kind":self.kind,"subject":self.subject,"check":self.check,"old":self.old,"new":self.new,"sentence":self.sentence()})
    }
}

fn side(value: &Value) -> String {
    match value {
        Value::Null => "missing".into(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// The fidelity comparison: the rollups compared, every disagreement, and the canary checks of a
/// compared rollup that the Python report holds no row for (`unverdicted`, `<rollup> canary <id>`:
/// canaries are replayed from that report, so these were not compared, M5b-2 item 4).
pub struct Fidelity {
    pub rollups: Vec<String>,
    pub disagreements: Vec<Disagreement>,
    pub unverdicted: Vec<String>,
}

impl Fidelity {
    /// Rollups on which every check agrees.
    pub fn agree(&self) -> usize {
        self.rollups
            .iter()
            .filter(|r| !self.disagreements.iter().any(|d| &d.subject == *r))
            .count()
    }
}

/// The Python watcher's side of a night: its newest report, if any, and whether that report was
/// written after tonight's old-path run began (only a fresh report is compared, M5b-1 item 3).
pub enum Python<'a> {
    Missing,
    Stale(&'a str),
    Fresh(&'a str, &'a Value),
}

impl Python<'_> {
    /// The report to compare: a fresh one, else nothing.
    pub fn compared(&self) -> &Value {
        match self {
            Python::Fresh(_, report) => report,
            _ => &Value::Null,
        }
    }
    pub fn state(&self) -> &'static str {
        match self {
            Python::Missing => "missing",
            Python::Stale(_) => "stale",
            Python::Fresh(..) => "fresh",
        }
    }
}

/// Whether a report named `<YYYYMMDD>T<HHMMSS>Z.json` was written at or after `old_run`
/// (`YYYY-MM-DDTHH:MM:SSZ`); a name that is not such a stamp is never fresh.
pub fn fresh_report(name: &str, old_run: &str) -> bool {
    let stamp = name.strip_suffix(".json").unwrap_or(name);
    let compact: String = old_run
        .chars()
        .filter(|c| !matches!(c, '-' | ':'))
        .collect();
    report_stamp(stamp) && stamp >= compact.as_str()
}

/// Whether `old_run` (`YYYY-MM-DDTHH:MM:SSZ`) falls on the night's `date` or the day before
/// (M5b-2 item 6): the old chain runs in the small hours, a few hours before the shadow.
pub fn old_run_on_night(date: &str, old_run: &str) -> bool {
    crate::rollup_prep::valid_date(date)
        && old_run
            .get(..10)
            .is_some_and(crate::rollup_prep::valid_date)
        && matches!(
            crate::terms::days_between(&old_run[..10], date),
            Some(0 | 1)
        )
}

/// Whether `old_run` (`YYYY-MM-DDTHH:MM:SSZ`) is no later than `now` (`YYYYMMDDTHHMMSSZ`, UTC):
/// a run that has not happened yet would make tonight's real rollup look old (M5b-3 F-3).
pub fn old_run_not_future(old_run: &str, now: &str) -> bool {
    let compact: String = old_run
        .chars()
        .filter(|c| !matches!(c, '-' | ':'))
        .collect();
    report_stamp(&compact) && report_stamp(now) && compact.as_str() <= now
}

/// `YYYYMMDDTHHMMSSZ`.
pub fn report_stamp(stamp: &str) -> bool {
    stamp.len() == 16
        && stamp.bytes().enumerate().all(|(i, b)| match i {
            8 => b == b'T',
            15 => b == b'Z',
            _ => b.is_ascii_digit(),
        })
}

/// What the night compared: the rollups `palace watch` covered that the Python report did not
/// (`not_compared`), those dated on or after the old-path run's day (`fresh`), the fresh ones
/// that were compared (`fresh_compared`), the names a fresh rollup's Supersedes section gives
/// outside both of its lists (`unclassified`, `{rollup, names}` from `palace watch`, M5b-3 F-1),
/// and, when any fresh rollup was not compared or names such files, why (`uncompared`, M5b-2
/// item 3: agreement only where established).
pub struct Coverage {
    pub not_compared: Vec<String>,
    pub fresh: Vec<String>,
    pub fresh_compared: usize,
    pub unclassified: Vec<Value>,
    pub uncompared: Option<String>,
}

pub fn coverage(python: &Python, palace: &Value, fidelity: &Fidelity, old_run: &str) -> Coverage {
    let compared = &fidelity.rollups;
    let covered = rollups(palace);
    let day = old_run.get(..10).unwrap_or(old_run);
    let fresh: Vec<String> = covered
        .iter()
        .filter(|r| crate::rollup_prep::entry_date(r).is_some_and(|d| d >= day))
        .cloned()
        .collect();
    let unclassified = fresh_unclassified(palace, &fresh);
    let missing = fresh.iter().find(|r| !compared.contains(r));
    let uncompared = missing
        .map(|rollup| uncompared_reason(python, compared, rollup, old_run))
        .or_else(|| unclassified.first().map(unclassified_reason))
        .or_else(|| fidelity.unverdicted.first().map(|check| format!(
            "tonight's Python report has no verdict for {check}, which the replay judge answers from it"
        )));
    Coverage {
        not_compared: covered
            .into_iter()
            .filter(|r| !compared.contains(r))
            .collect(),
        fresh_compared: fresh.iter().filter(|r| compared.contains(r)).count(),
        fresh,
        unclassified,
        uncompared,
    }
}

/// The watch report's `unclassified` entries for fresh rollups only: an old rollup's would
/// otherwise fire every night under `--all`.
fn fresh_unclassified(palace: &Value, fresh: &[String]) -> Vec<Value> {
    palace["unclassified"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|u| {
            u["rollup"]
                .as_str()
                .is_some_and(|r| fresh.iter().any(|f| f == r))
        })
        .cloned()
        .collect()
}

/// `<rollup> names <files> outside both the superseded list and every referenced-only region`.
fn unclassified_reason(entry: &Value) -> String {
    let names: Vec<&str> = entry["names"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    format!(
        "{} names {} outside both the superseded list and every referenced-only region",
        entry["rollup"].as_str().unwrap_or("?"),
        names.join(", ")
    )
}

fn uncompared_reason(python: &Python, compared: &[String], rollup: &str, old_run: &str) -> String {
    if !compared.is_empty() {
        return format!("tonight's Python report does not cover {rollup}");
    }
    match python {
        Python::Missing => format!("no Python report, while {rollup} is new tonight"),
        Python::Stale(name) => format!(
            "the newest Python report {name} is older than tonight's old-path run ({old_run}), while {rollup} is new tonight"
        ),
        Python::Fresh(name, _) => {
            format!(
                "tonight's Python report {name} has no result rows, while {rollup} is new tonight"
            )
        }
    }
}

/// `python` (the newest watcher report) against `palace` (the watch report of the same night).
pub fn fidelity(python: &Value, palace: &Value) -> Fidelity {
    let rollups = rollups(python);
    let (mut disagreements, mut unverdicted) = (Vec::new(), Vec::new());
    for rollup in &rollups {
        for key in keys(python, rollup).into_iter().chain(keys(palace, rollup)) {
            let (old, new) = (status(python, rollup, &key), status(palace, rollup, &key));
            let seen = disagreements
                .iter()
                .any(|d: &Disagreement| &d.subject == rollup && d.check == key);
            if old.is_null() && key.starts_with("canary ") {
                unverdicted.push(format!("{rollup} {key}"));
            } else if old != new && !seen {
                disagreements.push(Disagreement {
                    kind: "fidelity",
                    subject: rollup.clone(),
                    check: key,
                    old,
                    new,
                });
            }
        }
    }
    Fidelity {
        rollups,
        disagreements,
        unverdicted,
    }
}

/// The rollups a report covers, in first-seen order.
fn rollups(report: &Value) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for row in rows(report) {
        if let Some(name) = row["rollup"]
            .as_str()
            .filter(|n| !names.iter().any(|x| x == n))
        {
            names.push(name.to_owned());
        }
    }
    names
}

fn rows(report: &Value) -> impl Iterator<Item = &Value> {
    report["results"].as_array().into_iter().flatten()
}

/// The compared checks of one rollup: `floor`, `depth`, `canary <id>`.
fn keys(report: &Value, rollup: &str) -> Vec<String> {
    rows(report)
        .filter(|r| r["rollup"] == rollup)
        .filter_map(key)
        .collect()
}

fn key(row: &Value) -> Option<String> {
    match row["check"].as_str()? {
        check @ ("floor" | "depth") => Some(check.to_owned()),
        "canary" => Some(format!(
            "canary {}",
            row["canary_id"].as_str().unwrap_or("?")
        )),
        _ => None,
    }
}

fn status(report: &Value, rollup: &str, key: &str) -> Value {
    rows(report)
        .find(|r| r["rollup"] == rollup && self::key(r).as_deref() == Some(key))
        .map_or(Value::Null, |r| r["status"].clone())
}

/// The substrate's recorded result per relation (`old`, each with a boolean `ok` or `accepted`)
/// against the dry run's verdicts (`new`, each `{ok, reason}`), in file order.
pub fn relations(file: &str, old: &[Value], new: &[Value]) -> Vec<Disagreement> {
    let verdict = |v: Option<&Value>| {
        v.and_then(|v| v.get("ok").or_else(|| v.get("accepted")))
            .and_then(Value::as_bool)
            .map_or(Value::Null, |ok| {
                json!(if ok { "accepted" } else { "rejected" })
            })
    };
    (0..old.len().max(new.len()))
        .filter_map(|i| {
            let (before, after) = (verdict(old.get(i)), verdict(new.get(i)));
            (before != after).then(|| Disagreement {
                kind: "relation",
                subject: format!("{file} relation {}", i + 1),
                check: "gate".into(),
                old: before,
                new: after,
            })
        })
        .collect()
}

/// Every row `palace doctor --shadow-diff --json` names as different.
pub fn graph(diff: &Value) -> Vec<Disagreement> {
    diff["differences"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|d| Disagreement {
            kind: "graph",
            subject: format!("triple {}", d["triple"].as_str().unwrap_or("?")),
            check: d["field"].as_str().unwrap_or("?").to_owned(),
            old: d["logs"].clone(),
            new: d["graph"].clone(),
        })
        .collect()
}

/// A step's name and exit code, and the codes that mean it ran as intended.
pub struct Step {
    pub name: String,
    pub exit: i32,
}

impl Step {
    /// `watch` exits 1 on findings and `shadow-diff` on a difference: both ran as intended.
    fn finished(&self) -> bool {
        self.exit == 0 || (self.exit == 1 && (self.name == "watch" || self.name == "shadow-diff"))
    }
}

pub const EXIT_AGREE: i32 = 0;
pub const EXIT_DISAGREE: i32 = 1;
pub const EXIT_BROKEN: i32 = 2;
pub const EXIT_UNAVAILABLE: i32 = 3;

/// The night's exit: 2 when any step broke, else 3 when any was unavailable (retry later), else
/// 1 on any disagreement, else 0.
pub fn exit(steps: &[Step], disagreements: usize) -> i32 {
    let failed = |code: i32| steps.iter().any(|s| !s.finished() && s.exit == code);
    if steps
        .iter()
        .any(|s| !s.finished() && s.exit != EXIT_UNAVAILABLE)
    {
        EXIT_BROKEN
    } else if failed(EXIT_UNAVAILABLE) {
        EXIT_UNAVAILABLE
    } else if disagreements > 0 {
        EXIT_DISAGREE
    } else {
        EXIT_AGREE
    }
}

/// What the night found, for the printed line and both reports.
pub struct Night<'a> {
    pub date: &'a str,
    pub steps: &'a [Step],
    pub fidelity: &'a Fidelity,
    pub disagreements: &'a [Disagreement],
    pub asks: &'a [String],
    pub digest: &'a [String],
    pub coverage: &'a Coverage,
}

impl Night<'_> {
    /// `exit`, except that a night which compared nothing although a rollup is fresh is not
    /// agreement (1).
    pub fn exit(&self) -> i32 {
        match exit(self.steps, self.disagreements.len()) {
            EXIT_AGREE if self.coverage.uncompared.is_some() => EXIT_DISAGREE,
            code => code,
        }
    }

    /// Nothing fresh on either side: no rollup compared and none new.
    fn quiet(&self) -> bool {
        self.fidelity.rollups.is_empty() && self.coverage.fresh.is_empty()
    }

    /// The one printed line.
    pub fn line(&self) -> String {
        let code = self.exit();
        let failed = self
            .steps
            .iter()
            .find(|s| !s.finished() && (s.exit == EXIT_UNAVAILABLE) == (code == EXIT_UNAVAILABLE));
        match (code, failed) {
            (EXIT_BROKEN, Some(s)) => format!(
                "SHADOW BROKEN date={} step={} exit={}",
                self.date, s.name, s.exit
            ),
            (EXIT_UNAVAILABLE, Some(s)) => {
                format!("SHADOW UNAVAILABLE date={} step={}", self.date, s.name)
            }
            (EXIT_DISAGREE, _) if !self.disagreements.is_empty() => format!(
                "SHADOW DIFF date={} disagreements={} first=\"{}\"",
                self.date,
                self.disagreements.len(),
                self.disagreements[0].sentence().replace('"', "'")
            ),
            (EXIT_DISAGREE, _) => format!(
                "SHADOW UNCOMPARED date={} reason=\"{}\"",
                self.date,
                self.coverage
                    .uncompared
                    .as_deref()
                    .unwrap_or_default()
                    .replace('"', "'")
            ),
            _ if self.quiet() => format!("SHADOW QUIET date={}", self.date),
            _ => format!(
                "SHADOW OK date={} rollups={} agree={} asks={} digest_lines={} fresh={} compared={}",
                self.date,
                self.fidelity.rollups.len(),
                self.fidelity.agree(),
                self.asks.len(),
                self.digest.len(),
                self.coverage.fresh.len(),
                self.coverage.fresh_compared
            ),
        }
    }

    /// report.json (schema in NOTES.md, M5a item 1).
    pub fn json(&self, counts: Value, sources: Value) -> Value {
        json!({
            "date": self.date,
            "line": self.line(),
            "exit": self.exit(),
            "steps": self.steps.iter().map(|s| json!({"name":s.name,"exit":s.exit})).collect::<Vec<_>>(),
            "counts": counts,
            "sources": sources,
            "rollups": self.fidelity.rollups,
            "not_compared": self.coverage.not_compared,
            "fresh_rollups": self.coverage.fresh,
            "fresh_compared": self.coverage.fresh_compared,
            "unverdicted_canaries": self.fidelity.unverdicted,
            "unclassified": self.coverage.unclassified,
            "uncompared_reason": self.coverage.uncompared,
            "disagreements": self.disagreements.iter().map(Disagreement::json).collect::<Vec<_>>(),
            "would_send": {"asks": self.asks, "digest": self.digest},
        })
    }

    /// REPORT.md: at most 60 lines, the full lists stay in report.json.
    pub fn markdown(&self) -> String {
        let mut lines = vec![
            format!("# Shadow night {}", self.date),
            String::new(),
            format!("`{}`", self.line()),
            CANARY_CAVEAT.into(),
            String::new(),
            "## Steps".into(),
        ];
        let steps: Vec<String> = self
            .steps
            .iter()
            .map(|s| format!("{} exit={}", s.name, s.exit))
            .collect();
        lines.push(steps.join(" · "));
        section(&mut lines, "Disagreements (old → new)", &sentences(self));
        let not_compared: Vec<String> = (self.coverage.not_compared.iter())
            .chain(&self.fidelity.unverdicted)
            .cloned()
            .chain(self.coverage.unclassified.iter().map(unclassified_reason))
            .collect();
        section(
            &mut lines,
            "Covered by palace watch, not by the Python report (not compared)",
            &not_compared,
        );
        section(&mut lines, "Asks that would have been sent", self.asks);
        section(&mut lines, "Digest that would have been sent", self.digest);
        lines.join("\n") + "\n"
    }
}

/// README and every REPORT.md say it: the replay judge answers canaries from the Python report.
pub const CANARY_CAVEAT: &str = "Canary verdicts are replayed from the old path's Python report, so only floor, depth and the claim checks are compared independently.";

fn sentences(night: &Night) -> Vec<String> {
    night
        .disagreements
        .iter()
        .map(Disagreement::sentence)
        .collect()
}

/// A titled list clipped to 10 items (the rest are counted), or "none": four full sections
/// keep REPORT.md under 60 lines.
fn section(lines: &mut Vec<String>, title: &str, items: &[String]) {
    const SHOWN: usize = 10;
    lines.push(String::new());
    lines.push(format!("## {title}"));
    if items.is_empty() {
        lines.push("none".into());
    }
    lines.extend(items.iter().take(SHOWN).map(|i| format!("- {i}")));
    if items.len() > SHOWN {
        lines.push(format!("- +{} more in report.json", items.len() - SHOWN));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(rows: Value) -> Value {
        json!({"summary":{},"results":rows})
    }

    #[test]
    fn equal_statuses_agree_and_a_flip_names_rollup_and_check() {
        let old = report(json!([
            {"check":"floor","rollup":"r1","status":"fail"},
            {"check":"depth","rollup":"r1","status":"warn"},
            {"check":"canary","rollup":"r1","canary_id":"c","status":"absent"},
        ]));
        let mut new = old.clone();
        new["results"]
            .as_array_mut()
            .unwrap()
            .push(json!({"check":"claim","rollup":"r1","status":"fail"}));
        new["results"]
            .as_array_mut()
            .unwrap()
            .push(json!({"check":"floor","rollup":"r2","status":"pass"}));
        let same = fidelity(&old, &new);
        assert_eq!((same.rollups.len(), same.agree()), (1, 1));
        assert!(same.disagreements.is_empty());
        new["results"][0]["status"] = json!("pass");
        new["results"][2]["canary_id"] = json!("d");
        let diff = fidelity(&old, &new);
        assert_eq!(diff.agree(), 0);
        let checks: Vec<&str> = diff
            .disagreements
            .iter()
            .map(|d| d.check.as_str())
            .collect();
        assert_eq!(checks, ["floor", "canary c"]);
        assert_eq!(diff.unverdicted, ["r1 canary d"]);
        assert_eq!(
            diff.disagreements[0].sentence(),
            "r1 floor: old fail, new pass"
        );
    }

    #[test]
    fn relation_verdicts_compare_by_position() {
        let old = [json!({"ok":true}), json!({"accepted":false})];
        let new = [json!({"ok":true,"reason":null}), json!({"ok":true})];
        let d = relations("r.json", &old, &new);
        assert_eq!(d.len(), 1);
        assert_eq!(
            d[0].sentence(),
            "r.json relation 2 gate: old rejected, new accepted"
        );
        assert_eq!(relations("r.json", &old[..1], &new).len(), 1);
    }

    #[test]
    fn exit_prefers_broken_then_unavailable_then_disagreement() {
        let step = |name: &str, exit| Step {
            name: name.into(),
            exit,
        };
        assert_eq!(exit(&[step("watch", 1), step("shadow-diff", 1)], 0), 0);
        assert_eq!(exit(&[step("watch", 1)], 2), 1);
        assert_eq!(exit(&[step("auto", 3)], 2), 3);
        assert_eq!(exit(&[step("auto", 3), step("digest", 2)], 0), 2);
        assert_eq!(exit(&[step("auto", 1)], 0), 2);
    }

    #[test]
    fn markdown_stays_under_sixty_lines() {
        let fidelity = Fidelity {
            rollups: vec!["r1".into()],
            disagreements: vec![],
            unverdicted: vec![],
        };
        let many: Vec<String> = (0..40).map(|i| format!("line {i}")).collect();
        let steps = [Step {
            name: "watch".into(),
            exit: 0,
        }];
        let coverage = Coverage {
            not_compared: many.clone(),
            fresh: vec!["r1".into()],
            fresh_compared: 1,
            unclassified: vec![],
            uncompared: None,
        };
        let disagreements: Vec<Disagreement> = (0..40)
            .map(|i| Disagreement {
                kind: "graph",
                subject: format!("triple {i}"),
                check: "confidence".into(),
                old: json!(1),
                new: json!(0),
            })
            .collect();
        let mut night = Night {
            date: "2026-09-28",
            steps: &steps,
            fidelity: &fidelity,
            disagreements: &disagreements,
            asks: &many,
            digest: &many,
            coverage: &coverage,
        };
        assert!(night.markdown().lines().count() < 60);
        night.disagreements = &[];
        assert_eq!(
            night.line(),
            "SHADOW OK date=2026-09-28 rollups=1 agree=1 asks=40 digest_lines=40 fresh=1 compared=1"
        );
    }

    #[test]
    fn only_a_report_stamped_after_the_old_run_is_fresh() {
        let run = "2026-09-22T09:30:00Z";
        assert!(fresh_report("20260922T093000Z.json", run));
        assert!(fresh_report("20260923T000000Z.json", run));
        assert!(!fresh_report("20260922T091000Z.json", run));
        assert!(!fresh_report("latest.json", run));
        assert!(!fresh_report("20260922T0930Z.json", run));
    }

    fn night_line(python: &Python, watch: &Value, old_run: &str) -> (String, i32, Vec<String>) {
        let fidelity = fidelity(python.compared(), watch);
        let coverage = coverage(python, watch, &fidelity, old_run);
        let steps = [Step {
            name: "watch".into(),
            exit: 1,
        }];
        let night = Night {
            date: "2026-09-28",
            steps: &steps,
            fidelity: &fidelity,
            disagreements: &fidelity.disagreements,
            asks: &[],
            digest: &[],
            coverage: &coverage,
        };
        (night.line(), night.exit(), coverage.not_compared)
    }

    /// M5b-1 item 3 (review M-2): a missing, stale or row-less Python report is not agreement
    /// when a rollup is fresh; with nothing fresh the night is quiet; uncompared rollups are named.
    #[test]
    fn nothing_compared_is_uncompared_or_quiet_never_ok() {
        let row = |r: &str| json!({"check":"floor","rollup":r,"status":"ok"});
        let watch = report(json!([
            row("2026-09-19-aaaaaaaaaaaa"),
            row("2026-09-28-bbbbbbbbbbbb")
        ]));
        let tonight = report(json!([row("2026-09-28-bbbbbbbbbbbb")]));
        let empty = report(json!([]));
        let run = "2026-09-28T00:00:00Z";
        let (line, code, not_compared) = night_line(
            &Python::Fresh("20260928T091000Z.json", &tonight),
            &watch,
            run,
        );
        assert!(
            line.starts_with("SHADOW OK date=2026-09-28 rollups=1 agree=1"),
            "{line}"
        );
        assert_eq!(
            (code, not_compared),
            (0, vec!["2026-09-19-aaaaaaaaaaaa".to_owned()])
        );
        for (python, reason) in [
            (
                Python::Missing,
                "no Python report, while 2026-09-28-bbbbbbbbbbbb is new tonight",
            ),
            (
                Python::Stale("20260925T091000Z.json"),
                "the newest Python report 20260925T091000Z.json is older than tonight's old-path run (2026-09-28T00:00:00Z), while 2026-09-28-bbbbbbbbbbbb is new tonight",
            ),
            (
                Python::Fresh("20260928T091000Z.json", &empty),
                "tonight's Python report 20260928T091000Z.json has no result rows, while 2026-09-28-bbbbbbbbbbbb is new tonight",
            ),
        ] {
            let (line, code, not_compared) = night_line(&python, &watch, run);
            assert_eq!(
                line,
                format!("SHADOW UNCOMPARED date=2026-09-28 reason=\"{reason}\"")
            );
            assert_eq!((code, not_compared.len()), (1, 2));
        }
        let (line, code, _) = night_line(&Python::Missing, &watch, "2026-09-29T00:00:00Z");
        assert_eq!((line.as_str(), code), ("SHADOW QUIET date=2026-09-28", 0));
    }

    /// M5b-2 item 3 (review N-2): a fresh report whose rows cover only an older rollup, on a
    /// night when another rollup is fresh, is UNCOMPARED naming that rollup, never OK.
    #[test]
    fn a_fresh_rollup_missing_from_the_compared_set_is_uncompared() {
        let row = |r: &str| json!({"check":"floor","rollup":r,"status":"ok"});
        let old = "2026-09-19-aaaaaaaaaaaa";
        let new = "2026-09-22-bbbbbbbbbbbb";
        let watch = report(json!([row(old), row(new)]));
        let older_only = report(json!([row(old)]));
        let python = Python::Fresh("20260922T091000Z.json", &older_only);
        let (line, code, _) = night_line(&python, &watch, "2026-09-22T00:00:00Z");
        assert_eq!(
            (line.as_str(), code),
            (
                "SHADOW UNCOMPARED date=2026-09-28 reason=\"tonight's Python report does not cover 2026-09-22-bbbbbbbbbbbb\"",
                1
            )
        );
        let both = report(json!([row(old), row(new)]));
        let python = Python::Fresh("20260922T091000Z.json", &both);
        let (line, code, _) = night_line(&python, &watch, "2026-09-22T00:00:00Z");
        assert!(line.ends_with("fresh=1 compared=1"), "{line}");
        assert_eq!(code, 0);
    }

    /// M5b-2 item 4 (review S-g): a canary palace watch answered while tonight's Python report
    /// has no row for it (so the replay judge had no verdict) is not compared, never a
    /// disagreement, and the night is UNCOMPARED.
    #[test]
    fn a_canary_the_python_report_has_no_row_for_is_not_compared() {
        let rollup = "2026-09-22-bbbbbbbbbbbb";
        let floor = json!({"check":"floor","rollup":rollup,"status":"ok"});
        let canary = json!({"check":"canary","rollup":rollup,"canary_id":"c1","status":"warn",
            "reason":"no replay answer for 2026-09-22-bbbbbbbbbbbb / c1"});
        let watch = report(json!([floor, canary]));
        let python_rows = report(json!([floor]));
        let python = Python::Fresh("20260922T091000Z.json", &python_rows);
        let (line, code, _) = night_line(&python, &watch, "2026-09-22T00:00:00Z");
        assert_eq!(
            (line.as_str(), code),
            (
                "SHADOW UNCOMPARED date=2026-09-28 reason=\"tonight's Python report has no verdict for 2026-09-22-bbbbbbbbbbbb canary c1, which the replay judge answers from it\"",
                1
            )
        );
        assert!(fidelity(&python_rows, &watch).disagreements.is_empty());
    }

    /// M5b-3 F-1: a fresh rollup whose Supersedes section names files outside both lists makes
    /// the night UNCOMPARED with the names in the reason and in report.json; an old rollup's
    /// unclassified names are not surfaced.
    #[test]
    fn unclassified_names_on_a_fresh_rollup_are_uncompared() {
        let (old, new) = ("2026-09-01-aaaaaaaaaaaa", "2026-09-22-bbbbbbbbbbbb");
        let row = |r: &str| json!({"check":"floor","rollup":r,"status":"ok"});
        let mut watch = report(json!([row(old), row(new)]));
        watch["unclassified"] = json!([{"rollup":old,"names":["2026-08-22-cccccccccccc"]}]);
        let python = Python::Fresh("20260922T091000Z.json", &watch);
        let (line, code, _) = night_line(&python, &watch, "2026-09-22T00:00:00Z");
        assert_eq!(code, 0, "{line}");
        watch["unclassified"].as_array_mut().unwrap().push(
            json!({"rollup":new,"names":["2026-08-30-dddddddddddd","2026-08-31-eeeeeeeeeeee"]}),
        );
        let python = Python::Fresh("20260922T091000Z.json", &watch);
        let (line, code, _) = night_line(&python, &watch, "2026-09-22T00:00:00Z");
        assert_eq!(
            (line.as_str(), code),
            (
                "SHADOW UNCOMPARED date=2026-09-28 reason=\"2026-09-22-bbbbbbbbbbbb names 2026-08-30-dddddddddddd, 2026-08-31-eeeeeeeeeeee outside both the superseded list and every referenced-only region\"",
                1
            )
        );
        let fidelity = fidelity(&watch, &watch);
        let coverage = coverage(&python, &watch, &fidelity, "2026-09-22T00:00:00Z");
        assert_eq!(coverage.unclassified.len(), 1);
    }

    /// M5b-3 F-3: an old-path run later than now is refused.
    #[test]
    fn an_old_run_later_than_now_is_refused() {
        let now = "20260929T052000Z";
        assert!(old_run_not_future("2026-09-29T05:20:00Z", now));
        assert!(old_run_not_future("2026-09-29T04:20:00Z", now));
        assert!(!old_run_not_future("2026-09-29T05:20:01Z", now));
        assert!(!old_run_not_future("2026-09-30T00:00:00Z", now));
        assert!(!old_run_not_future("tomorrow", now));
    }

    /// M5b-2 item 6 (review S-b): the old-path run falls on the night's date or the day before.
    #[test]
    fn the_old_run_falls_on_the_night_or_the_day_before() {
        assert!(old_run_on_night("2026-09-29", "2026-09-29T08:20:00Z"));
        assert!(old_run_on_night("2026-09-29", "2026-09-28T23:59:59Z"));
        assert!(old_run_on_night("2026-03-01", "2026-02-28T08:20:00Z"));
        assert!(!old_run_on_night("2026-09-29", "2026-09-01T00:00:00Z"));
        assert!(!old_run_on_night("2026-09-29", "2026-09-30T00:00:00Z"));
        assert!(!old_run_on_night("2026-09-29", "2026-09-27T23:59:59Z"));
        assert!(!old_run_on_night("2026-09-29", "yesterday"));
    }
}
