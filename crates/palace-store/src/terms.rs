//! Terms (M4d item 8), the read side: the KG's `type=term` entities, each term's usage in the
//! claim store (for the survival bar), its definition claim and when it was coined. Read-only.
use crate::Error;
use palace_core::terms::Usage;
use rusqlite::{Connection, OptionalExtension};
use serde_json::Value;
use std::path::Path;

pub struct Term {
    pub id: String,
    pub word: String,
    pub properties: Value,
    /// The `<word> | means | …` claim `coin` created, if any.
    pub definition: Option<String>,
    /// When it was coined (the coin event's `at`).
    pub coined_at: Option<String>,
}

impl Term {
    pub fn status(&self) -> &str {
        self.properties["status"].as_str().unwrap_or("proposed")
    }
}

/// Every term entity, by id.
pub fn terms(compiler: &Path, kg: &Path) -> Result<Vec<Term>, Error> {
    let c = crate::inspect::open(compiler, kg)?;
    let mut s =
        c.prepare("SELECT id, name, properties FROM kg.entities WHERE type='term' ORDER BY id")?;
    let rows: Vec<(String, String, Option<String>)> = s
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<Result<_, _>>()?;
    rows.into_iter()
        .map(|(id, word, properties)| {
            let definition = definition(&c, &word)?;
            Ok(Term {
                coined_at: coined_at(&c, definition.as_deref())?,
                properties: serde_json::from_str(properties.as_deref().unwrap_or("{}"))?,
                id,
                word,
                definition,
            })
        })
        .collect()
}

fn definition(c: &Connection, word: &str) -> Result<Option<String>, Error> {
    Ok(c.query_row(
        "SELECT id FROM claims WHERE lower(subject)=lower(?1) AND predicate='means' ORDER BY first_seen, id LIMIT 1",
        [word],
        |r| r.get(0),
    )
    .optional()?)
}

fn coined_at(c: &Connection, definition: Option<&str>) -> Result<Option<String>, Error> {
    let Some(definition) = definition else {
        return Ok(None);
    };
    Ok(c.query_row(
        "SELECT at FROM improvement_events WHERE verb='coin' AND to_claim=?1 ORDER BY rowid LIMIT 1",
        [definition],
        |r| r.get(0),
    )
    .optional()?)
}

/// Claims that use `word` (as predicate, subject or object; not its own definition claims).
const USES: &str = "(c.predicate=?1 OR lower(c.subject)=?1 OR lower(c.object)=?1)";

/// How `word` is used: claims and rollups (with their as-of dates), open blocking findings, and
/// whether its definition claim survived a check.
pub fn usage(compiler: &Path, kg: &Path, term: &Term) -> Result<Usage, Error> {
    let c = crate::inspect::open(compiler, kg)?;
    let word = term.word.to_lowercase();
    let (claims, rollups, first, last): (i64, i64, Option<String>, Option<String>) = c.query_row(
        &format!("SELECT count(DISTINCT c.id), count(DISTINCT n.rollup), min(n.date), max(n.date) FROM claims c JOIN claim_numbers n ON n.claim_id=c.id WHERE c.status<>'undone' AND NOT (lower(c.subject)=?1 AND c.predicate IN ('means','differs_from')) AND {USES}"),
        [&word],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
    )?;
    let blocking: i64 = c.query_row(
        &format!("SELECT count(*) FROM findings f JOIN claims c ON c.id=f.claim WHERE f.decision IS NULL AND f.kind IN ('vocabulary_gap','conflation') AND {USES}"),
        [&word],
        |r| r.get(0),
    )?;
    let survived: i64 = match &term.definition {
        Some(id) => c.query_row("SELECT survived FROM claims WHERE id=?1", [id], |r| {
            r.get(0)
        })?,
        None => 0,
    };
    Ok(Usage {
        claims: claims as usize,
        rollups: rollups as usize,
        first,
        last,
        blocking: blocking as usize,
        definition_survived: survived > 0,
    })
}
