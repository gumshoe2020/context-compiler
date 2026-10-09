//! The per-turn reads of `palace context` (M5c item 2): live graph nodes with their aliases, the
//! first-order relations of the matched nodes in the hook's order (numeric source_closet
//! confidence first, then newest), the claim behind each relation, and the open asks touching a
//! matched node. compiler.sqlite is opened read-only with the KG attached read-only: no lock is
//! taken, nothing is reconciled or written.
use crate::Error;
use palace_core::entity_match::Node;
use rusqlite::{Connection, OpenFlags, OptionalExtension, params_from_iter};
use std::path::Path;

/// One relation line's data.
#[derive(Debug, Clone, PartialEq)]
pub struct Relation {
    pub triple: String,
    pub subject: String,
    pub predicate: String,
    pub object: String,
    /// The triple's own confidence (`source_closet.confidence`), when it is a number.
    pub confidence: Option<f64>,
    pub claim: Option<ClaimRef>,
}

/// The claim behind a relation: the claim that created the triple, else the latest citing it.
#[derive(Debug, Clone, PartialEq)]
pub struct ClaimRef {
    pub id: String,
    /// `<rollup date> Cn` of its newest numbering, when it has one.
    pub handle: Option<String>,
    pub status: String,
    pub confidence: f64,
    pub source: String,
}

/// An open ask (unanswered, its finding undecided) on a claim.
#[derive(Debug, Clone, PartialEq)]
pub struct Ask {
    pub finding: String,
    pub claim: String,
    pub token: String,
    pub line: String,
}

pub struct Reader {
    c: Connection,
}

impl Reader {
    /// compiler.sqlite and the KG, both read-only (`mode=ro`); either missing is an error.
    pub fn open(store: &Path, kg: &Path) -> Result<Self, Error> {
        for (what, path) in [("store", store), ("KG", kg)] {
            if !path.is_file() {
                return Err(format!("{what} {}: not a file", path.display()).into());
            }
        }
        let flags = OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_URI
            | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let c = Connection::open_with_flags(format!("file:{}?mode=ro", uri(store)?), flags)?;
        c.execute(
            "ATTACH DATABASE ?1 AS kg",
            [format!("file:{}?mode=ro", uri(kg)?)],
        )?;
        Ok(Self { c })
    }

    /// Every node at an end of a live triple, ids ascending (the hook's order), with its aliases.
    pub fn nodes(&self) -> Result<Vec<Node>, Error> {
        let mut s = self.c.prepare(
            "SELECT e.id, CASE WHEN json_valid(e.properties) THEN json_extract(e.properties,'$.aliases') END
             FROM kg.entities e WHERE e.id IN (SELECT subject FROM kg.triples WHERE valid_to IS NULL
             UNION SELECT object FROM kg.triples WHERE valid_to IS NULL) ORDER BY e.id",
        )?;
        let rows = s.query_map([], |r| {
            Ok(Node {
                id: r.get(0)?,
                aliases: aliases(r.get::<_, Option<String>>(1)?.as_deref()),
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// The live relations touching `matched`, at most `max`, and how many there are in all.
    pub fn relations(
        &self,
        matched: &[String],
        max: usize,
    ) -> Result<(Vec<Relation>, usize), Error> {
        if matched.is_empty() {
            return Ok((Vec::new(), 0));
        }
        let rows = self.relation_rows(matched, max)?;
        let total = rows.first().map_or(0, |r| r.total);
        let relations = rows
            .into_iter()
            .map(|row| self.relation(row))
            .collect::<Result<_, _>>()?;
        Ok((relations, total))
    }

    fn relation_rows(&self, matched: &[String], max: usize) -> Result<Vec<Row>, Error> {
        let list = (1..=matched.len())
            .map(|i| format!("?{i}"))
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "SELECT t.id, s.name, t.predicate, o.name,
                    CASE WHEN json_valid(t.source_closet) THEN json_extract(t.source_closet,'$.confidence') END AS c,
                    CASE WHEN json_valid(t.source_closet) THEN json_extract(t.source_closet,'$.claim_id') END,
                    count(*) OVER ()
             FROM kg.triples t JOIN kg.entities s ON t.subject=s.id JOIN kg.entities o ON t.object=o.id
             WHERE t.valid_to IS NULL AND (t.subject IN ({list}) OR t.object IN ({list}))
             ORDER BY c DESC, t.extracted_at DESC LIMIT {max}"
        );
        let mut s = self.c.prepare(&sql)?;
        let rows = s.query_map(params_from_iter(matched), |r| {
            Ok(Row {
                triple: r.get(0)?,
                subject: r.get(1)?,
                predicate: r.get(2)?,
                object: r.get(3)?,
                confidence: number(r.get_ref(4)?),
                claim: text(r.get_ref(5)?),
                total: r.get::<_, i64>(6)?.try_into().unwrap_or(0),
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    fn relation(&self, row: Row) -> Result<Relation, Error> {
        let claim = match self.claim_of(&row.triple, row.claim.as_deref())? {
            Some(id) => Some(self.claim_ref(&id)?),
            None => None,
        };
        Ok(Relation {
            triple: row.triple,
            subject: row.subject,
            predicate: row.predicate,
            object: row.object,
            confidence: row.confidence,
            claim,
        })
    }

    /// The claim that created the triple (its provenance `claim_id`, when the store knows it),
    /// else the claim that cited it last.
    fn claim_of(&self, triple: &str, created_by: Option<&str>) -> Result<Option<String>, Error> {
        if let Some(id) = created_by {
            let known = self
                .c
                .prepare("SELECT 1 FROM claims WHERE id=?1")?
                .exists([id])?;
            if known {
                return Ok(Some(id.to_owned()));
            }
        }
        Ok(self
            .c
            .query_row(
                "SELECT claim_id FROM citations WHERE triple_id=?1 AND withdrawn_at IS NULL AND claim_id IN (SELECT id FROM claims)
                 ORDER BY cited_at DESC, claim_id LIMIT 1",
                [triple],
                |r| r.get(0),
            )
            .optional()?)
    }

    fn claim_ref(&self, id: &str) -> Result<ClaimRef, Error> {
        let (status, source): (String, String) =
            self.c
                .query_row("SELECT status, source FROM claims WHERE id=?1", [id], |r| {
                    Ok((r.get(0)?, r.get(1)?))
                })?;
        let handle = self
            .c
            .query_row(
                "SELECT date || ' ' || number FROM claim_numbers WHERE claim_id=?1 ORDER BY date DESC, rollup DESC LIMIT 1",
                [id],
                |r| r.get(0),
            )
            .optional()?;
        Ok(ClaimRef {
            id: id.to_owned(),
            handle,
            status,
            confidence: crate::claims::derived(&self.c, id)?,
            source,
        })
    }

    /// Open asks, oldest first, that touch the matched nodes: the ask's claim stands behind one
    /// of `relations`, or its subject or object is one of `matched` (entity ids).
    pub fn open_asks(&self, matched: &[String], relations: &[Relation]) -> Result<Vec<Ask>, Error> {
        if matched.is_empty() || !self.has_table("asks")? {
            return Ok(Vec::new());
        }
        let behind: Vec<&str> = relations
            .iter()
            .filter_map(|r| r.claim.as_ref().map(|c| c.id.as_str()))
            .collect();
        let touches = |claim: &str, ends: [&str; 2]| {
            behind.contains(&claim)
                || ends
                    .iter()
                    .any(|name| matched.contains(&palace_core::ids::entity_id(name)))
        };
        Ok(self
            .asks()?
            .into_iter()
            .filter(|(ask, ends)| touches(&ask.claim, [&ends.0, &ends.1]))
            .map(|(ask, _)| ask)
            .collect())
    }

    /// Every open ask (unanswered, its finding undecided) with its claim's subject and object.
    fn asks(&self) -> Result<Vec<(Ask, Ends)>, Error> {
        let mut s = self.c.prepare(
            "SELECT a.finding, a.claim, a.token, a.line, c.subject, c.object FROM asks a JOIN claims c ON c.id=a.claim
             WHERE a.answered_at IS NULL AND NOT EXISTS (SELECT 1 FROM findings f WHERE f.id=a.finding AND f.decision IS NOT NULL)
             ORDER BY a.asked_at, a.finding",
        )?;
        let rows = s.query_map([], |r| {
            let ask = Ask {
                finding: r.get(0)?,
                claim: r.get(1)?,
                token: r.get(2)?,
                line: r.get(3)?,
            };
            Ok((ask, (r.get(4)?, r.get(5)?)))
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// CTX-1: every live specific claim, newest first (date, then first seen), each with the
    /// wing its entry was distilled into: the `wing` of the `distill_commit` event that wrote
    /// the entry (None when no event names it). None at all on a store without memory types.
    pub fn live_specifics(&self) -> Result<Vec<SpecificRow>, Error> {
        if !self.has_column("claims", "kind")? {
            return Ok(Vec::new());
        }
        let wings = self.entry_wings()?;
        let mut s = self.c.prepare(
            "SELECT id,subject,predicate,object,date,COALESCE(quote,''),rollup FROM claims
             WHERE kind='specific' AND status='live' ORDER BY date DESC, first_seen DESC, rowid DESC",
        )?;
        let rows = s.query_map([], |r| {
            Ok(SpecificRow {
                id: r.get(0)?,
                subject: r.get(1)?,
                predicate: r.get(2)?,
                object: r.get(3)?,
                date: r.get(4)?,
                quote: r.get(5)?,
                entry: r.get(6)?,
                wing: None,
            })
        })?;
        let mut rows: Vec<SpecificRow> = rows.collect::<Result<_, _>>()?;
        for row in &mut rows {
            row.wing = wings.get(entry_name(&row.entry)).cloned();
        }
        Ok(rows)
    }

    /// CTX-1 item 3: entry name (without `.md`) → the lowercase subjects of the live claims it
    /// states (any kind).
    pub fn entry_subjects(
        &self,
    ) -> Result<std::collections::HashMap<String, std::collections::BTreeSet<String>>, Error> {
        let mut s = self
            .c
            .prepare("SELECT rollup, lower(subject) FROM claims WHERE status='live'")?;
        let rows = s.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        let mut subjects: std::collections::HashMap<String, std::collections::BTreeSet<String>> =
            std::collections::HashMap::new();
        for row in rows {
            let (entry, subject) = row?;
            subjects
                .entry(entry_name(&entry).to_owned())
                .or_default()
                .insert(subject);
        }
        Ok(subjects)
    }

    /// Entry name (without `.md`) → the wing its `distill_commit` event filed it in.
    fn entry_wings(&self) -> Result<std::collections::HashMap<String, String>, Error> {
        let mut s = self.c.prepare(
            "SELECT json_extract(payload,'$.wing'), json_extract(payload,'$.files') FROM write_log
             WHERE json_valid(payload) AND json_extract(payload,'$.operation')='distill_commit' ORDER BY sequence",
        )?;
        let rows = s.query_map([], |r| {
            Ok((
                r.get::<_, Option<String>>(0)?,
                r.get::<_, Option<String>>(1)?,
            ))
        })?;
        let mut wings = std::collections::HashMap::new();
        for row in rows {
            let (Some(wing), Some(files)) = row? else {
                continue;
            };
            for file in serde_json::from_str::<Vec<String>>(&files).unwrap_or_default() {
                wings.insert(entry_name(&file).to_owned(), wing.clone());
            }
        }
        Ok(wings)
    }

    fn has_column(&self, table: &str, column: &str) -> Result<bool, Error> {
        Ok(self
            .c
            .prepare("SELECT 1 FROM pragma_table_info(?1) WHERE name=?2")?
            .exists([table, column])?)
    }

    fn has_table(&self, table: &str) -> Result<bool, Error> {
        Ok(self
            .c
            .prepare("SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1")?
            .exists([table])?)
    }
}

/// The kinds of `target`'s open findings (`c_…`, `f_…` or "<rollup date> C7"), from the store
/// opened read-only (`palace improve --classify`, M5c item 3).
pub fn open_finding_kinds(store: &Path, target: &str) -> Result<Vec<String>, Error> {
    if !store.is_file() {
        return Err(format!("store {}: not a file", store.display()).into());
    }
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI;
    let c = Connection::open_with_flags(format!("file:{}?mode=ro", uri(store)?), flags)?;
    let claim = crate::lookup::resolve(&c, target)?.claim_id;
    let filter = crate::lookup::FindingFilter {
        open: true,
        claim: Some(claim),
        ..Default::default()
    };
    Ok(crate::lookup::findings(&c, &filter)?
        .iter()
        .filter_map(|f| f["kind"].as_str().map(str::to_owned))
        .collect())
}

/// A live specific claim (CTX-1 block 1).
#[derive(Debug, Clone, PartialEq)]
pub struct SpecificRow {
    pub id: String,
    pub subject: String,
    pub predicate: String,
    pub object: String,
    pub date: String,
    pub quote: String,
    /// The entry that stated it.
    pub entry: String,
    pub wing: Option<String>,
}

/// An entry's name without `.md`.
fn entry_name(entry: &str) -> &str {
    entry.strip_suffix(".md").unwrap_or(entry)
}

/// A claim's subject and object names.
type Ends = (String, String);

struct Row {
    triple: String,
    subject: String,
    predicate: String,
    object: String,
    confidence: Option<f64>,
    claim: Option<String>,
    total: usize,
}

/// A numeric SQLite value as f64 (the hook shows `C=` only for numbers).
fn number(value: rusqlite::types::ValueRef) -> Option<f64> {
    match value {
        rusqlite::types::ValueRef::Integer(i) => Some(i as f64),
        rusqlite::types::ValueRef::Real(f) => Some(f),
        _ => None,
    }
}

fn text(value: rusqlite::types::ValueRef) -> Option<String> {
    match value {
        rusqlite::types::ValueRef::Text(t) => String::from_utf8(t.to_vec()).ok(),
        _ => None,
    }
}

/// The alias strings of `properties.aliases` (a JSON array); anything else is no aliases.
fn aliases(json: Option<&str>) -> Vec<String> {
    json.and_then(|text| serde_json::from_str::<Vec<String>>(text).ok())
        .unwrap_or_default()
}

/// A canonical path percent-encoded for a `file:` URI.
fn uri(path: &Path) -> Result<String, Error> {
    let path = path.canonicalize()?;
    let text = path.to_str().ok_or("non-UTF8 path")?;
    Ok(text
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"/-_.".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect())
}
