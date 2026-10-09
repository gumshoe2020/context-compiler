//! `palace regress` (M4c): the frozen queries through the pinned MemPalace, compared with the
//! frozen answers. The palace given is only ever read: regress copies it into its own scratch
//! directory, strips and searches the copy, and removes the copy unless `--keep` (M4b.2 0a).
//! Exit 0 pass, 1 regression, 3 MemPalace unavailable, 2 broken.
use crate::scratch::Scratch;
use clap::Args;
use palace_core::{
    regress::{self, Case, Gates, Metrics},
    search_hit::SearchHit,
};
use palace_mempalace::{MemPalace, Unavailable, diary_only};
use palace_store::Error;
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

pub const EXIT_REGRESSION: i32 = 1;
pub const EXIT_BROKEN: i32 = 2;
pub const EXIT_UNAVAILABLE: i32 = 3;
pub const VARIANTS: [(&str, f64); 2] = [("default_0", 0.0), ("max_distance_1_5", 1.5)];

#[derive(Args, Clone)]
pub struct Options {
    /// The palace to measure (e.g. fixtures/palace). Read only: regress works on its own copy.
    #[arg(long)]
    pub palace: PathBuf,
    /// Scratch directory for the copy (must not exist; default target/regress-<pid>-<ns>/).
    #[arg(long)]
    pub scratch: Option<PathBuf>,
    /// Leave the scratch copy in place for inspection.
    #[arg(long)]
    pub keep: bool,
    /// Seconds allowed for the diary-only step (the pinned interpreter); then exit 3.
    #[arg(long, default_value_t = 120)]
    pub strip_timeout: u64,
    #[arg(long)]
    pub queries: PathBuf,
    #[arg(long)]
    pub expected: PathBuf,
    /// default_0 | max_distance_1_5 | all
    #[arg(long, default_value = "all")]
    pub variant: String,
    #[arg(long, default_value = "openclaw")]
    pub wing: String,
    #[arg(long)]
    pub report: Option<PathBuf>,
    /// Write fresh frozen answers (fixture schema) to a path outside fixtures/.
    #[arg(long)]
    pub write_expected: Option<PathBuf>,
    #[arg(long, default_value_t = 45)]
    pub min_top1: usize,
    #[arg(long, default_value_t = 0.90)]
    pub min_top5: f64,
    /// Queries whose ordered top-5 must be identical (per variant).
    #[arg(long, default_value_t = 45)]
    pub min_exact: usize,
    #[arg(long, default_value_t = 0)]
    pub max_errors: usize,
    #[command(flatten)]
    pub connection: crate::MemPalaceOptions,
}

/// One finished comparison (pass or regression).
pub struct Outcome {
    pub passed: bool,
    pub line: String,
    pub report: Value,
}

/// One query's answer for one variant: the hits, or the search error.
struct Answer {
    query: String,
    expected: Vec<String>,
    hits: Result<Vec<SearchHit>, String>,
}

struct Versions {
    pin: String,
    cli: String,
    server: String,
}

pub fn main(options: Options) -> i32 {
    let result = run(&options);
    if let Ok(outcome) = &result {
        print_outcome(outcome);
    }
    exit_code(result.map(|outcome| outcome.passed))
}

/// `Ok(true)` → 0, `Ok(false)` → 1, `Unavailable` → 3 (by type), any other error → 2.
pub fn exit_code(result: Result<bool, Error>) -> i32 {
    match result.map_err(palace_store::busy::named) {
        Ok(true) => 0,
        Ok(false) => EXIT_REGRESSION,
        Err(error) if Unavailable::is(&error) || palace_store::busy::Busy::is(&error) => {
            eprintln!("palace regress: {error}");
            EXIT_UNAVAILABLE
        }
        Err(error) => {
            eprintln!("palace regress: {error}");
            EXIT_BROKEN
        }
    }
}

pub fn run(o: &Options) -> Result<Outcome, Error> {
    let inputs = Inputs::load(o)?;
    let scratch = Scratch::create(o.scratch.as_deref(), o.keep)?;
    let palace = scratch.copy_palace(&inputs.source)?;
    let timeout = std::time::Duration::from_secs(o.strip_timeout);
    let stripped = diary_only::strip(&o.connection.mempalace, &palace, timeout)?;
    let (mempalace, versions) = connect(o, &palace, inputs.pin.clone())?;
    let mut measured = Vec::new();
    for ((name, max_distance), frozen) in inputs.variants.iter().zip(&inputs.frozen) {
        let answers = search_all(&mempalace, &o.wing, *max_distance, &inputs.queries, frozen)?;
        measured.push((*name, *max_distance, answers));
    }
    let context = json!({"palace": inputs.source, "copy": palace, "diary_only": stripped});
    let outcome = assess(o, &versions, &measured, context);
    inputs.write_outputs(&o.wing, &measured, &outcome)?;
    Ok(outcome)
}

/// Everything read or validated before the copy is made (a refusal creates nothing).
struct Inputs {
    write_expected: Option<PathBuf>,
    report: Option<PathBuf>,
    variants: Vec<(&'static str, f64)>,
    queries: Vec<String>,
    /// Per variant, the frozen top-5 names of every query (parsed strictly up front).
    frozen: Vec<Vec<Vec<String>>>,
    pin: String,
    source: PathBuf,
}

impl Inputs {
    fn load(o: &Options) -> Result<Self, Error> {
        let write_expected = o.write_expected.as_deref().map(output).transpose()?;
        let report = o.report.as_deref().map(output).transpose()?;
        let variants = variants(&o.variant)?;
        let queries = load_queries(&crate::paths::input(&o.queries)?)?;
        let expected: Value =
            serde_json::from_slice(&fs::read(crate::paths::input(&o.expected)?)?)?;
        let frozen = variants
            .iter()
            .map(|(name, _)| frozen_tops(&expected, name, &queries))
            .collect::<Result<_, _>>()?;
        Ok(Self {
            write_expected,
            report,
            variants,
            queries,
            frozen,
            pin: palace_mempalace::version::pin()?,
            source: crate::paths::input(&o.palace)?,
        })
    }

    fn write_outputs(
        &self,
        wing: &str,
        measured: &[(&str, f64, Vec<Answer>)],
        outcome: &Outcome,
    ) -> Result<(), Error> {
        if let Some(path) = &self.write_expected {
            write_json(path, &expected_file(wing, measured))?;
        }
        if let Some(path) = &self.report {
            write_json(path, &outcome.report)?;
        }
        Ok(())
    }
}

fn output(path: &Path) -> Result<PathBuf, Error> {
    crate::paths::database(path).map_err(|e| format!("{}: {e}", path.display()).into())
}

pub fn variants(choice: &str) -> Result<Vec<(&'static str, f64)>, Error> {
    match choice {
        "all" => Ok(VARIANTS.to_vec()),
        _ => VARIANTS
            .iter()
            .find(|(name, _)| *name == choice)
            .map(|v| vec![*v])
            .ok_or_else(|| format!("unknown --variant {choice:?}").into()),
    }
}

fn load_queries(path: &Path) -> Result<Vec<String>, Error> {
    let value: Value = serde_json::from_slice(&fs::read(path)?)?;
    let queries = value["queries"]
        .as_array()
        .ok_or("queries file: `queries` is not an array")?;
    queries
        .iter()
        .enumerate()
        .map(|(i, q)| {
            q.as_str()
                .map(str::to_owned)
                .ok_or_else(|| format!("queries file: queries[{i}] is not a string").into())
        })
        .collect()
}

/// Opens the MCP server without the compiler's own pin (the pin under test comes from the
/// script) and reads both versions for the gate.
fn connect(o: &Options, palace: &Path, pin: String) -> Result<(MemPalace, Versions), Error> {
    let unavailable = |e: Error| Unavailable::error(e);
    let cli = palace_mempalace::version::read(&o.connection.mempalace).map_err(unavailable)?;
    let mempalace = MemPalace::open(
        palace,
        &o.connection.mempalace_mcp,
        &o.connection.mempalace,
        true,
    )
    .map_err(unavailable)?;
    let server = mempalace.server_version().unwrap_or_default();
    let versions = Versions { pin, cli, server };
    Ok((mempalace, versions))
}

/// The frozen top-5 `source_file` lists of one variant. Strict: any schema error names the field
/// (`variants.<v>.results[i].top[j].source_file`) and is a broken run (exit 2).
fn frozen_tops(
    expected: &Value,
    variant: &str,
    queries: &[String],
) -> Result<Vec<Vec<String>>, Error> {
    let at = format!("variants.{variant}.results");
    let rows = expected["variants"][variant]["results"]
        .as_array()
        .ok_or_else(|| format!("expected file: {at} is missing or not an array"))?;
    if rows.len() != queries.len() {
        let n = rows.len();
        return Err(format!(
            "expected file: {at} has {n} rows for {} queries",
            queries.len()
        )
        .into());
    }
    rows.iter()
        .zip(queries)
        .enumerate()
        .map(|(i, (row, query))| frozen_row(row, query, &format!("{at}[{i}]")))
        .collect()
}

fn frozen_row(row: &Value, query: &str, at: &str) -> Result<Vec<String>, Error> {
    let broken = |what: String| -> Error { format!("expected file: {at}{what}").into() };
    if row["query"].as_str() != Some(query) {
        return Err(broken(format!(
            ".query is not {query:?} (rows out of order)"
        )));
    }
    if !(row["error"].is_null() || row["error"].is_string()) {
        return Err(broken(".error must be null or a string".into()));
    }
    let top = row["top"].as_array();
    let top = top.ok_or_else(|| broken(".top is missing or not an array".into()))?;
    if top.len() > regress::TOP {
        let (n, max) = (top.len(), regress::TOP);
        return Err(broken(format!(".top has {n} hits (max {max})")));
    }
    let file = |(j, hit): (usize, &Value)| match hit["source_file"].as_str() {
        Some(file) if !file.is_empty() => Ok(file.to_owned()),
        _ => Err(broken(format!(".top[{j}].source_file is missing or empty"))),
    };
    top.iter().enumerate().map(file).collect()
}

fn search_all(
    mempalace: &MemPalace,
    wing: &str,
    max_distance: f64,
    queries: &[String],
    frozen: &[Vec<String>],
) -> Result<Vec<Answer>, Error> {
    let wing = (!wing.is_empty()).then_some(wing);
    queries
        .iter()
        .zip(frozen)
        .map(|(query, expected)| {
            let hits = mempalace.search_in(query, regress::TOP, max_distance, wing);
            if let Some(reason) = mempalace.unavailable() {
                return Err(Unavailable::error(reason));
            }
            let hits = hits.map_err(|e| e.to_string());
            Ok(Answer {
                query: query.clone(),
                expected: expected.clone(),
                hits,
            })
        })
        .collect()
}

fn cases(answers: &[Answer]) -> Vec<Case> {
    answers
        .iter()
        .map(|a| Case {
            query: a.query.clone(),
            expected: a.expected.clone(),
            actual: a.hits.as_ref().ok().map(|hits| names(hits)),
        })
        .collect()
}

fn names(hits: &[SearchHit]) -> Vec<String> {
    hits.iter().map(|hit| hit.source_file.clone()).collect()
}

fn assess(
    o: &Options,
    versions: &Versions,
    measured: &[(&str, f64, Vec<Answer>)],
    context: Value,
) -> Outcome {
    let gates = Gates {
        min_top1: o.min_top1,
        min_top5: o.min_top5,
        min_exact: o.min_exact,
        max_errors: o.max_errors,
    };
    let per_variant: Vec<Metrics> = measured
        .iter()
        .map(|(_, _, a)| regress::metrics(&cases(a)))
        .collect();
    let mut failed: Vec<String> = per_variant
        .iter()
        .zip(measured)
        .flat_map(|(m, (name, _, _))| {
            regress::failures(m, &gates)
                .into_iter()
                .map(move |f| format!("{name}:{f}"))
        })
        .collect();
    failed.extend(version_failures(versions));
    let worst = regress::worst(&per_variant).unwrap_or_else(|| regress::metrics(&[]));
    let line = summary_line(&versions.pin, measured.len(), &worst, &failed);
    let report = report(
        versions,
        &gates,
        measured,
        &per_variant,
        (&line, &failed),
        context,
    );
    Outcome {
        passed: failed.is_empty(),
        line,
        report,
    }
}

pub fn version_failures_of(pin: &str, cli: &str, server: &str) -> Vec<String> {
    let mut failed = Vec::new();
    if cli != format!("MemPalace {pin}") {
        failed.push(format!("cli_version={cli:?}"));
    }
    if server != pin {
        failed.push(format!("server_version={server:?}"));
    }
    failed
}

fn version_failures(v: &Versions) -> Vec<String> {
    version_failures_of(&v.pin, &v.cli, &v.server)
}

pub fn summary_line(pin: &str, variants: usize, worst: &Metrics, failed: &[String]) -> String {
    let body = regress::summary(pin, variants, worst);
    if failed.is_empty() {
        format!("M4c OK {body}")
    } else {
        format!("M4c FAIL {body} failed={}", failed.join(","))
    }
}

fn report(
    versions: &Versions,
    gates: &Gates,
    measured: &[(&str, f64, Vec<Answer>)],
    metrics: &[Metrics],
    (line, failed): (&str, &[String]),
    context: Value,
) -> Value {
    let variants: serde_json::Map<String, Value> = measured
        .iter()
        .zip(metrics)
        .map(|((name, max_distance, answers), m)| {
            (name.to_string(), variant_report(*max_distance, answers, m))
        })
        .collect();
    json!({
        "line": line,
        "status": if failed.is_empty() { "ok" } else { "fail" },
        "failures": failed,
        "mempalace": {"pin": versions.pin, "cli": versions.cli, "server": versions.server},
        "context": context,
        "gates": {"min_top1": gates.min_top1, "min_top5": gates.min_top5, "min_exact": gates.min_exact, "max_errors": gates.max_errors},
        "variants": variants,
    })
}

fn variant_report(max_distance: f64, answers: &[Answer], m: &Metrics) -> Value {
    let differing: Vec<Value> = answers
        .iter()
        .filter(|a| a.hits.as_ref().map(|h| names(h)).as_ref() != Ok(&a.expected))
        .map(|a| {
            json!({
                "query": a.query,
                "expected": a.expected,
                "actual": a.hits.as_ref().ok().map(|h| names(h)),
                "error": a.hits.as_ref().err(),
            })
        })
        .collect();
    json!({
        "max_distance": max_distance,
        "queries": m.queries, "answered": m.answered, "top1": m.top1,
        "top5": m.top5, "exact": m.exact, "errors": m.errors,
        "differing": differing,
    })
}

/// The frozen-answer file in the fixture's exact schema.
fn expected_file(wing: &str, measured: &[(&str, f64, Vec<Answer>)]) -> Value {
    let variants: serde_json::Map<String, Value> = measured
        .iter()
        .map(|(name, _, answers)| {
            let results: Vec<Value> = answers.iter().map(expected_row).collect();
            (name.to_string(), json!({"results": results}))
        })
        .collect();
    json!({
        "captured": today(),
        "palace": "diary-only copy of fixtures/palace (import drawers/closets deleted by palace regress)",
        "function": format!("mempalace-mcp mempalace_search(query, wing={wing:?}, limit=5, max_distance=<variant>) via palace regress"),
        "variants": variants,
    })
}

fn expected_row(answer: &Answer) -> Value {
    match &answer.hits {
        Ok(hits) => {
            json!({"query": answer.query, "error": null, "top": hits.iter().map(expected_hit).collect::<Vec<_>>()})
        }
        Err(error) => json!({"query": answer.query, "error": error, "top": []}),
    }
}

pub fn expected_hit(hit: &SearchHit) -> Value {
    let field = |key: &str| hit.metadata.get(key).cloned().unwrap_or(Value::Null);
    json!({
        "source_path": hit.source_path,
        "source_file": hit.source_file,
        "distance": field("distance"),
        "effective_distance": field("effective_distance"),
        "closet_boost": field("closet_boost"),
        "matched_via": field("matched_via"),
        "similarity": field("similarity"),
        "created_at": field("created_at"),
        "text_head": hit.text.chars().take(120).collect::<String>(),
    })
}

fn today() -> String {
    let days = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() / 86_400) as i64;
    let (y, m, d) = palace_core::diary_file::civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}")
}

fn write_json(path: &Path, value: &Value) -> Result<(), Error> {
    fs::write(path, serde_json::to_string_pretty(value)? + "\n")?;
    Ok(())
}

/// The summary line, then every differing query with both top-5 lists, pass or fail.
fn print_outcome(outcome: &Outcome) {
    println!("{}", outcome.line);
    for line in differs_lines(&outcome.report) {
        println!("{line}");
    }
}

pub fn differs_lines(report: &Value) -> Vec<String> {
    let mut lines = Vec::new();
    for (variant, row) in report["variants"].as_object().into_iter().flatten() {
        for diff in row["differing"].as_array().into_iter().flatten() {
            lines.push(format!(
                "  differs variant={variant} query={} expected={} actual={}",
                diff["query"],
                diff["expected"],
                diff.get("error")
                    .filter(|e| !e.is_null())
                    .map_or_else(|| diff["actual"].to_string(), |e| format!("error:{e}"))
            ));
        }
    }
    lines
}

// The tests that read the private fixtures (PUB-1a item 3): src/doctors/, never exported.
#[cfg(all(test, doctors))]
#[path = "doctors/regress_fixtures.rs"]
mod fixture_tests;
#[cfg(test)]
mod tests {
    use super::*;

    pub(super) fn options(write_expected: Option<PathBuf>) -> Options {
        Options {
            palace: PathBuf::from("target/regress-test/palace"),
            scratch: None,
            keep: false,
            strip_timeout: 120,
            queries: PathBuf::from("fixtures/queries-50.json"),
            expected: PathBuf::from("fixtures/mempalace-search-50.json"),
            variant: "all".into(),
            wing: "openclaw".into(),
            report: None,
            write_expected,
            min_top1: 1,
            min_top5: 0.2,
            min_exact: 0,
            max_errors: 0,
            connection: crate::MemPalaceOptions {
                mempalace_mcp: "/nonexistent/mempalace-mcp".into(),
                mempalace: "/nonexistent/mempalace".into(),
                allow_mempalace_version: false,
            },
        }
    }
    fn hit(file: &str) -> SearchHit {
        serde_json::from_value(json!({
            "text": "x".repeat(200), "source_path": format!("/d/{file}"), "source_file": file,
            "wing": "openclaw", "room": "general", "distance": 0.5, "effective_distance": 0.5,
            "closet_boost": 0.0, "matched_via": "drawer", "similarity": 0.5,
            "created_at": "2026-09-22T00:00:00", "bm25_score": 1.0
        }))
        .unwrap()
    }
    pub(super) fn answer(query: &str, expected: &[&str], actual: Result<&[&str], &str>) -> Answer {
        Answer {
            query: query.into(),
            expected: expected.iter().map(|s| s.to_string()).collect(),
            hits: actual
                .map(|files| files.iter().map(|f| hit(f)).collect())
                .map_err(str::to_owned),
        }
    }
    fn versions(cli: &str, server: &str) -> Versions {
        Versions {
            pin: "3.5.0".into(),
            cli: cli.into(),
            server: server.into(),
        }
    }

    #[test]
    fn exit_codes_are_mapped_by_outcome_and_error_type() {
        assert_eq!(exit_code(Ok(true)), 0);
        assert_eq!(exit_code(Ok(false)), EXIT_REGRESSION);
        assert_eq!(exit_code(Err(Unavailable::error("EOF"))), EXIT_UNAVAILABLE);
        assert_eq!(
            exit_code(Err("MemPalace unavailable: lookalike".into())),
            EXIT_BROKEN
        );
    }

    #[test]
    fn write_expected_refuses_fixtures_before_touching_anything() {
        let target = crate::paths::root().join("fixtures/regress-refusal-test.json");
        let error = run(&options(Some(target.clone()))).err().expect("refused");
        assert!(error.to_string().contains("fixtures"), "{error}");
        assert_eq!(exit_code(Err(error)), EXIT_BROKEN);
        assert!(!target.exists());
    }

    /// A read-only stand-in palace and a fake interpreter that deletes `chroma.sqlite3` from
    /// every palace it is handed: the stand-in is byte-identical afterwards, the copy was the
    /// one hit, and the scratch dir is gone.
    /// The repository's check script as the pin's default, as `palace` names it at start.
    pub(super) fn checkout_pin() {
        let script = crate::paths::root().join("scripts/mempalace-check.sh");
        palace_mempalace::version::set_default_script(script);
    }

    #[test]
    fn pin_is_read_from_the_check_script() {
        checkout_pin();
        assert_eq!(palace_mempalace::version::pin().unwrap(), "3.5.0");
    }

    #[test]
    fn the_expected_file_is_parsed_strictly_naming_the_field() {
        let hit = |f: &str| json!({"source_file": f});
        let file = |top: Value| json!({"variants":{"default_0":{"results":[{"query":"q","error":null,"top":top}]}}});
        let queries = ["q".to_string()];
        let ok = frozen_tops(&file(json!([hit("a"), hit("b")])), "default_0", &queries);
        assert_eq!(ok.unwrap(), [["a", "b"]]);
        for (top, field) in [
            (
                json!([hit("a"), {"distance": 0.1}]),
                "variants.default_0.results[0].top[1].source_file",
            ),
            (json!([hit("")]), "results[0].top[0].source_file"),
            (
                json!(["a", "b", "c", "d", "e", "f"].map(hit)),
                "results[0].top has 6 hits",
            ),
            (json!("a"), "results[0].top is missing"),
        ] {
            let error = frozen_tops(&file(top), "default_0", &queries)
                .unwrap_err()
                .to_string();
            assert!(error.contains(field), "{error}");
        }
        let error = frozen_tops(&file(json!([])), "max_distance_1_5", &queries).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("variants.max_distance_1_5.results")
        );
        let wrong = frozen_tops(&file(json!([])), "default_0", &["other".to_string()]).unwrap_err();
        assert!(wrong.to_string().contains("results[0].query"));
    }

    #[test]
    fn variants_and_version_gates() {
        assert_eq!(variants("all").unwrap().len(), 2);
        assert_eq!(variants("default_0").unwrap(), [("default_0", 0.0)]);
        assert!(variants("max_distance_2").is_err());
        assert!(version_failures_of("3.5.0", "MemPalace 3.5.0", "3.5.0").is_empty());
        assert_eq!(
            version_failures_of("3.6.0", "MemPalace 3.5.0", "3.5.0"),
            [
                "cli_version=\"MemPalace 3.5.0\"",
                "server_version=\"3.5.0\""
            ]
        );
    }

    #[test]
    fn report_shape_and_lines() {
        let measured = vec![(
            "default_0",
            0.0,
            vec![
                answer("q1", &["a", "b"], Ok(&["a", "b"])),
                answer("q2", &["a", "b"], Ok(&["b", "a"])),
                answer("q3", &["a"], Err("boom")),
            ],
        )];
        let pass = assess(
            &options(None),
            &versions("MemPalace 3.5.0", "3.5.0"),
            &measured,
            json!({}),
        );
        assert!(!pass.passed, "one error fails max_errors 0");
        assert_eq!(
            pass.line,
            "M4c FAIL mempalace=3.5.0 variants=1 queries=2/3 top1=1/3 top5=0.27 exact=1/3 errors=1 failed=default_0:errors>0"
        );
        let r = &pass.report;
        assert_eq!(r["status"], "fail");
        assert_eq!(
            r["mempalace"],
            json!({"pin":"3.5.0","cli":"MemPalace 3.5.0","server":"3.5.0"})
        );
        assert_eq!(r["gates"]["min_top1"], 1);
        let v = &r["variants"]["default_0"];
        assert_eq!(
            (v["top1"].clone(), v["exact"].clone(), v["errors"].clone()),
            (json!(1), json!(1), json!(1))
        );
        let differing = v["differing"].as_array().unwrap();
        assert_eq!(differing.len(), 2);
        assert_eq!(
            differing[0],
            json!({"query":"q2","expected":["a","b"],"actual":["b","a"],"error":null})
        );
        assert_eq!(differing[1]["error"], "boom");
        assert_eq!(differing[1]["actual"], Value::Null);
        assert_eq!(
            differs_lines(r),
            [
                "  differs variant=default_0 query=\"q2\" expected=[\"a\",\"b\"] actual=[\"b\",\"a\"]",
                "  differs variant=default_0 query=\"q3\" expected=[\"a\"] actual=error:\"boom\""
            ]
        );
    }

    #[test]
    fn ok_line_and_version_failure() {
        let measured = vec![(
            "max_distance_1_5",
            1.5,
            vec![answer("q", &["a"], Ok(&["a"]))],
        )];
        let ok = assess(
            &options(None),
            &versions("MemPalace 3.5.0", "3.5.0"),
            &measured,
            json!({}),
        );
        assert!(ok.passed);
        assert_eq!(
            ok.line,
            "M4c OK mempalace=3.5.0 variants=1 queries=1/1 top1=1/1 top5=0.20 exact=1/1 errors=0"
        );
        let drift = assess(
            &options(None),
            &versions("MemPalace 3.5.1", "3.5.1"),
            &measured,
            json!({}),
        );
        assert!(!drift.passed && drift.line.contains("failed=cli_version="));
    }

    #[test]
    fn differing_rows_are_listed_on_a_pass_and_exact_is_gated() {
        let measured = vec![(
            "default_0",
            0.0,
            vec![
                answer("q", &["a", "b"], Ok(&["a", "b"])),
                answer("r", &["a", "b"], Ok(&["a", "c"])),
            ],
        )];
        let lenient = assess(
            &options(None),
            &versions("MemPalace 3.5.0", "3.5.0"),
            &measured,
            json!({}),
        );
        assert!(lenient.passed);
        assert_eq!(differs_lines(&lenient.report).len(), 1, "printed on OK too");
        let mut strict = options(None);
        strict.min_exact = 2;
        let fail = assess(
            &strict,
            &versions("MemPalace 3.5.0", "3.5.0"),
            &measured,
            json!({}),
        );
        assert!(!fail.passed && fail.line.ends_with("failed=default_0:exact<2"));
        assert_eq!(fail.report["gates"]["min_exact"], 2);
    }

    #[test]
    fn expected_file_uses_the_fixture_schema() {
        let measured = vec![(
            "default_0",
            0.0,
            vec![
                answer("q", &["a"], Ok(&["a"])),
                answer("e", &[], Err("bad")),
            ],
        )];
        let file = expected_file("openclaw", &measured);
        for key in ["captured", "palace", "function", "variants"] {
            assert!(file.get(key).is_some(), "{key}");
        }
        let rows = &file["variants"]["default_0"]["results"];
        let hit = rows[0]["top"][0].as_object().unwrap();
        let mut keys: Vec<&str> = hit.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "closet_boost",
                "created_at",
                "distance",
                "effective_distance",
                "matched_via",
                "similarity",
                "source_file",
                "source_path",
                "text_head"
            ]
        );
        assert_eq!(hit["text_head"].as_str().unwrap().chars().count(), 120);
        assert_eq!(rows[1], json!({"query":"e","error":"bad","top":[]}));
    }
}
