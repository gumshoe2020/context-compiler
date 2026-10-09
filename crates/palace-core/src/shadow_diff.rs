//! `palace doctor --shadow-diff` (M5a item 2, review S-2), all pure: what the palace write log
//! (graph writes, read with `wal_replay::effect`) and the compiler's chained write log
//! (`recovered_reapply`, `recovered_rollback`, `recovered_manual` rows) say each triple they
//! touch should be — live or not, its `valid_to`, its confidence — against the graph's rows.
//! A triple no line touches is not compared (the logs say nothing about it); a line that names
//! no triple and no fact is counted as `unkeyed`.
use crate::{
    graph_input::{name, predicate},
    ids::entity_id,
    wal_replay::{Effect, effect},
};
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// One graph triple as stored (subject and object are entity ids).
#[derive(Clone, Debug)]
pub struct Row {
    pub id: String,
    pub fact: (String, String, String),
    pub valid_to: Option<String>,
    pub confidence: f64,
}

/// One line of either log, with where it came from (`wal:<line>`, `log:<sequence>`).
pub struct Logged {
    pub source: String,
    pub line: Value,
}

/// What the logs say about one triple; `None` where they say nothing.
#[derive(Clone, Debug, Default)]
struct Expect {
    live: Option<bool>,
    valid_to: Option<Option<String>>,
    confidence: Option<f64>,
    source: String,
}

/// One field of one triple where the graph and the logs differ.
#[derive(Debug, PartialEq)]
pub struct Difference {
    pub triple: String,
    pub field: &'static str,
    pub graph: Value,
    pub logs: Value,
    pub source: String,
}

pub struct Outcome {
    pub rows: usize,
    pub unkeyed: usize,
    pub differences: Vec<Difference>,
}

impl Outcome {
    pub fn equal(&self) -> bool {
        self.differences.is_empty()
    }
    pub fn lines(&self) -> Vec<String> {
        let mut lines: Vec<String> = self
            .differences
            .iter()
            .map(|d| {
                format!(
                    "DIFF triple {} {} graph={} logs={} ({})",
                    d.triple, d.field, d.graph, d.logs, d.source
                )
            })
            .collect();
        let verdict = if self.equal() { "equal" } else { "different" };
        lines.push(format!(
            "SHADOW-DIFF {verdict} rows={} differ={} unkeyed={}",
            self.rows,
            self.differences.len(),
            self.unkeyed
        ));
        lines
    }
    pub fn json(&self) -> Value {
        let differences: Vec<Value> = self
            .differences
            .iter()
            .map(|d| json!({"triple":d.triple,"field":d.field,"graph":d.graph,"logs":d.logs,"source":d.source}))
            .collect();
        json!({"equal":self.equal(),"rows":self.rows,"unkeyed":self.unkeyed,"differences":differences})
    }
}

/// The palace write log's lines (file order) merged with the compiler log's recovery lines (log
/// order): a recovery line goes before the first WAL line stamped later in the same UTC format
/// (`…Z`); WAL lines in another format keep their place ahead of it.
pub fn merge(wal: Vec<Logged>, log: Vec<Logged>) -> Vec<Logged> {
    let mut log = log.into_iter().peekable();
    let mut merged = Vec::new();
    for line in wal {
        while log.peek().is_some_and(|l| before(l, &line)) {
            merged.extend(log.next());
        }
        merged.push(line);
    }
    merged.extend(log);
    merged
}

fn before(log: &Logged, wal: &Logged) -> bool {
    let (at, stamp) = (log.line["at"].as_str(), wal.line["timestamp"].as_str());
    matches!((at, stamp), (Some(a), Some(s)) if a.ends_with('Z') && s.ends_with('Z') && a < s)
}

/// Every touched triple's expectation against `rows`.
pub fn compare(lines: &[Logged], rows: &[Row]) -> Result<Outcome, String> {
    let mut expected: BTreeMap<String, Expect> = BTreeMap::new();
    let mut unkeyed = 0;
    for logged in lines {
        for effect in effects(&logged.line)? {
            if !apply(&mut expected, rows, effect, &logged.source) {
                unkeyed += 1;
            }
        }
    }
    let by_id: BTreeMap<&str, &Row> = rows.iter().map(|r| (r.id.as_str(), r)).collect();
    let differences = expected
        .iter()
        .flat_map(|(id, e)| differ(id, e, by_id.get(id.as_str()).copied()))
        .collect();
    Ok(Outcome {
        rows: expected.len(),
        unkeyed,
        differences,
    })
}

/// The effects of one line of either log.
fn effects(line: &Value) -> Result<Vec<Effect>, String> {
    let operation = line["operation"].as_str().unwrap_or_default();
    if !operation.starts_with("recovered_") {
        return Ok(vec![effect(line)?]);
    }
    let side = match (operation, line["choice"].as_str()) {
        ("recovered_reapply", _) => "after",
        ("recovered_manual", Some("keep-current")) => "now",
        _ => "before",
    };
    Ok(line["rows"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|r| r["table"] == "triples")
        .map(|r| recovered(r, side))
        .collect())
}

/// A recovery row's effect: the triple's column set to `side` (`*`: the row re-inserted, or the
/// insert undone by ending it).
fn recovered(row: &Value, side: &str) -> Effect {
    let id = row["key"].as_str().unwrap_or_default().to_owned();
    let value = &row[side];
    match (row["column"].as_str(), side) {
        (Some("*"), "after") => Effect::Add {
            id,
            confidence: value["confidence"].as_f64().unwrap_or(1.0),
            valid_to: value["valid_to"].as_str().map(str::to_owned),
        },
        (Some("*"), "before") => Effect::Invalidate { id, ended: None },
        (Some("valid_to"), _) if value.is_null() => Effect::Reopen {
            id,
            confidence: None,
        },
        (Some("valid_to"), _) => Effect::Invalidate {
            id,
            ended: value.as_str().map(str::to_owned),
        },
        (Some("confidence"), _) => {
            value
                .as_f64()
                .map_or(Effect::None, |confidence| Effect::Confidence {
                    id,
                    confidence,
                })
        }
        _ => Effect::None,
    }
}

/// Folds one effect into the expectations; false when it names nothing that can be keyed.
fn apply(
    expected: &mut BTreeMap<String, Expect>,
    rows: &[Row],
    effect: Effect,
    source: &str,
) -> bool {
    let mut touch = |id: String, change: &dyn Fn(&mut Expect)| {
        let e = expected.entry(id).or_default();
        change(e);
        e.source = source.to_owned();
    };
    match effect {
        Effect::Add {
            id,
            confidence,
            valid_to,
        } => touch(id, &|e| {
            e.live = Some(valid_to.is_none());
            e.valid_to = Some(valid_to.clone());
            e.confidence = Some(confidence);
        }),
        Effect::Invalidate { id, ended } => touch(id, &|e| {
            e.live = Some(false);
            e.valid_to = ended.clone().map(Some);
        }),
        Effect::InvalidateFact {
            subject,
            predicate,
            object,
            ended,
        } => {
            let Some(fact) = fact_key(&subject, &predicate, &object) else {
                return false;
            };
            // Only rows live at that point take `ended`; any other keeps a valid_to unknown here.
            for row in rows.iter().filter(|r| r.fact == fact) {
                touch(row.id.clone(), &|e| {
                    e.valid_to = ended.clone().filter(|_| e.live == Some(true)).map(Some);
                    e.live = Some(false);
                });
            }
        }
        Effect::Reopen { id, confidence } => touch(id, &|e| {
            e.live = Some(true);
            e.valid_to = Some(None);
            if confidence.is_some() {
                e.confidence = confidence;
            }
        }),
        Effect::Confidence { id, confidence } => touch(id, &|e| e.confidence = Some(confidence)),
        Effect::None => {}
    }
    true
}

/// A fact as the graph stores it (entity ids, normalised predicate).
fn fact_key(subject: &str, predicate_text: &str, object: &str) -> Option<(String, String, String)> {
    let args = json!({"subject":subject,"predicate":predicate_text,"object":object});
    Some((
        entity_id(&name(&args, "subject").ok()?),
        predicate(&args).ok()?,
        entity_id(&name(&args, "object").ok()?),
    ))
}

/// The fields of one expectation the row does not hold.
fn differ(id: &str, e: &Expect, row: Option<&Row>) -> Vec<Difference> {
    let difference = |field, graph: Value, logs: Value| Difference {
        triple: id.to_owned(),
        field,
        graph,
        logs,
        source: e.source.clone(),
    };
    let Some(row) = row else {
        let known =
            e.live == Some(true) || matches!(e.valid_to, Some(Some(_))) || e.confidence.is_some();
        return if known {
            vec![difference("row", json!("missing"), json!("present"))]
        } else {
            Vec::new()
        };
    };
    let mut found = Vec::new();
    if let Some(live) = e.live.filter(|live| *live != row.valid_to.is_none()) {
        found.push(difference("live", json!(!live), json!(live)));
    }
    if let Some(valid_to) = e.valid_to.as_ref().filter(|v| **v != row.valid_to) {
        found.push(difference("valid_to", json!(row.valid_to), json!(valid_to)));
    }
    if let Some(confidence) = e.confidence.filter(|c| (c - row.confidence).abs() > 1e-9) {
        found.push(difference(
            "confidence",
            json!(row.confidence),
            json!(confidence),
        ));
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, valid_to: Option<&str>, confidence: f64) -> Row {
        Row {
            id: id.into(),
            fact: (entity_id("a"), "supports".into(), entity_id(id)),
            valid_to: valid_to.map(str::to_owned),
            confidence,
        }
    }

    fn wal(lines: &[Value]) -> Vec<Logged> {
        lines
            .iter()
            .enumerate()
            .map(|(i, l)| Logged {
                source: format!("wal:{}", i + 1),
                line: l.clone(),
            })
            .collect()
    }

    fn lines() -> Vec<Value> {
        vec![
            json!({"operation":"kg_add","params":{},"result":{"created":true,"triple_id":"t_b","confidence":0.6}}),
            json!({"operation":"kg_add","params":{},"result":{"created":false,"triple_id":"t_a"}}),
            json!({"operation":"kg_set_confidence","params":{"triple_id":"t_b","confidence":0.45}}),
            json!({"operation":"kg_invalidate","params":{"triple_id":"t_c","ended":"2026-09-26"}}),
            json!({"operation":"kg_set_entity","params":{"name":"x"}}),
        ]
    }

    #[test]
    fn a_graph_that_matches_its_logs_is_equal_and_untouched_rows_are_not_compared() {
        let rows = [
            row("t_a", None, 0.1),
            row("t_b", None, 0.45),
            row("t_c", Some("2026-09-26"), 1.0),
        ];
        let outcome = compare(&wal(&lines()), &rows).unwrap();
        assert!(outcome.equal(), "{:?}", outcome.differences);
        assert_eq!((outcome.rows, outcome.unkeyed), (2, 0));
        assert_eq!(
            outcome.lines(),
            ["SHADOW-DIFF equal rows=2 differ=0 unkeyed=0"]
        );
    }

    #[test]
    fn a_hand_edited_confidence_and_a_reopened_row_are_named() {
        let rows = [row("t_b", None, 0.9), row("t_c", None, 1.0)];
        let outcome = compare(&wal(&lines()), &rows).unwrap();
        let named: Vec<(&str, &str)> = outcome
            .differences
            .iter()
            .map(|d| (d.triple.as_str(), d.field))
            .collect();
        assert_eq!(
            named,
            [("t_b", "confidence"), ("t_c", "live"), ("t_c", "valid_to")]
        );
        assert_eq!(outcome.differences[0].source, "wal:3");
        assert!(outcome.lines()[0].starts_with("DIFF triple t_b confidence graph=0.9 logs=0.45"));
    }

    #[test]
    fn invalidate_by_fact_ends_every_row_of_that_fact() {
        let by_name = json!({"operation":"kg_invalidate","params":{"subject":"a","predicate":"supports","object":"t_b"}});
        let unkeyed = json!({"operation":"kg_invalidate","params":{"subject":"a","predicate":"supports","object":""}});
        let rows = [row("t_b", None, 1.0)];
        let outcome = compare(&wal(&[by_name, unkeyed]), &rows).unwrap();
        assert_eq!(outcome.unkeyed, 1);
        assert_eq!(outcome.differences[0].field, "live");
    }

    #[test]
    fn recovery_lines_set_their_side_and_merge_by_utc_time() {
        let add = json!({"timestamp":"2026-09-27T10:00:00.000000Z","operation":"kg_add","params":{},"result":{"created":true,"triple_id":"t_b","confidence":0.6}});
        let rollback = json!({"operation":"recovered_rollback","at":"2026-09-27T09:00:00.000000Z",
            "rows":[{"table":"triples","key":"t_b","column":"confidence","before":0.3,"after":0.6}]});
        let merged = merge(
            wal(&[add]),
            vec![Logged {
                source: "log:9".into(),
                line: rollback,
            }],
        );
        assert_eq!(merged[0].source, "log:9");
        let outcome = compare(&merged, &[row("t_b", None, 0.6)]).unwrap();
        assert!(outcome.equal());
        let keep = json!({"operation":"recovered_manual","choice":"keep-current",
            "rows":[{"table":"triples","key":"t_b","column":"valid_to","before":null,"after":"2026-09-27","now":null}]});
        let undone = json!({"operation":"recovered_rollback","rows":[{"table":"triples","key":"t_x","column":"*","before":null,"after":{}}]});
        let lines = [
            Logged {
                source: "log:1".into(),
                line: keep,
            },
            Logged {
                source: "log:2".into(),
                line: undone,
            },
        ];
        assert!(compare(&lines, &[row("t_b", None, 0.6)]).unwrap().equal());
    }
}
