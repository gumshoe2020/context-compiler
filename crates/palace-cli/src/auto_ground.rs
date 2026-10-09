//! The auto loop's reads (M4d item 4): one grounding attempt per ungrounded claim — (a) the
//! claim's own words over the diary (its quote verbatim, else its object), (b) the thread's other
//! entries (every source the claim's rollup cites) with subject and object near each other, (c) a
//! configured judge asked about thread entries that mention both — and the second check a claim
//! above 0.80 needs before an automatic verb. Rollups, bookkeeping and IMPROVE entries are never
//! a source. Read-only.
use crate::improve::Ctx;
use palace_core::{
    auto::{claim_of, grounded_statement},
    canary::Question,
    claim::{Claim, claim_id, locates, mentions, unsupported_by},
    evidence::is_bookkeeping,
    rollup::Entry,
    traits::{DiaryReader, Judge},
};
use palace_mempalace::diary::Diary;
use palace_store::Error;
use serde_json::Value;

pub type BoxedJudge = Box<dyn Judge<Error = Error>>;

/// A source that carries the claim: its file, the successor statement citing it, and whether
/// that successor already exists (an identical add was made, and undone, before).
pub struct Found {
    pub source: String,
    pub statement: String,
    pub exists: bool,
}

pub fn ground(
    ctx: &Ctx,
    judge: Option<&dyn Judge<Error = Error>>,
    row: &Value,
) -> Result<Option<Found>, Error> {
    let c = claim_of(row);
    let diary = Diary::open(&ctx.root.join("diary"))?;
    let thread = thread(ctx, row)?;
    let mut hit = own_words(&diary, &c)?;
    if hit.is_none() {
        hit = search(&diary, &c, &thread, locates)?;
    }
    if let (None, Some(judge)) = (&hit, judge) {
        hit = judged(&diary, &c, &thread, judge)?;
    }
    hit.map(|(source, text)| found(ctx, &c, source, &text))
        .transpose()
}

fn found(ctx: &Ctx, c: &Claim, source: String, text: &str) -> Result<Found, Error> {
    let statement = grounded_statement(c, &source, text);
    let id = claim_id(&c.subject, &c.predicate, &c.object, &c.date, &source);
    let kg = crate::paths::input(&ctx.paths.kg)?;
    Ok(Found {
        exists: palace_store::auto::claim_exists(&ctx.compiler()?, &kg, &id)?,
        source,
        statement,
    })
}

/// (a) Every citable entry holding the claim's own words (its quote, else its object), in name
/// order, that locates the claim.
fn own_words(diary: &Diary, c: &Claim) -> Result<Option<(String, String)>, Error> {
    let words = c
        .quote
        .clone()
        .unwrap_or_else(|| c.object.clone())
        .to_lowercase();
    let names = diary.names()?;
    search(diary, c, &names, |c, text| {
        text.to_lowercase().contains(&words) && locates(c, text)
    })
}

/// The first citable entry among `names` (never the claim's own absent source) passing `test`.
fn search(
    diary: &Diary,
    c: &Claim,
    names: &[String],
    test: impl Fn(&Claim, &str) -> bool,
) -> Result<Option<(String, String)>, Error> {
    for name in names {
        let name = name.strip_suffix(".md").unwrap_or(name);
        if format!("{name}.md") == c.source {
            continue;
        }
        if let Some(text) = citable(diary, name)?
            && test(c, &text)
        {
            return Ok(Some((format!("{name}.md"), text)));
        }
    }
    Ok(None)
}

/// An entry's text when it may be a claim's source: not a rollup, not graph bookkeeping, not an
/// IMPROVE entry.
pub(crate) fn citable(diary: &Diary, name: &str) -> Result<Option<String>, Error> {
    let Some(text) = diary.read(name)? else {
        return Ok(None);
    };
    let rollup = Entry::parse(name, text.clone()).is_rollup;
    let improve = text.contains("Tags: claim-improvement");
    Ok((!rollup && !improve && !is_bookkeeping(&text)).then_some(text))
}

/// Review S-3: the claim's OWN cited source, re-read now, exists as a citable entry and locates
/// the claim (the next watch grounds it).
pub fn own_source_found(ctx: &Ctx, row: &Value) -> Result<bool, Error> {
    let c = claim_of(row);
    let diary = Diary::open(&ctx.root.join("diary"))?;
    let name = c.source.strip_suffix(".md").unwrap_or(&c.source);
    Ok(citable(&diary, name)?.is_some_and(|text| locates(&c, &text)))
}

/// (b)'s and (c)'s entries: the sources the claim's rollup cites.
fn thread(ctx: &Ctx, row: &Value) -> Result<Vec<String>, Error> {
    let rollup = row["rollup"].as_str().unwrap_or_default();
    let kg = crate::paths::input(&ctx.paths.kg)?;
    palace_store::auto::thread_sources(&ctx.compiler()?, &kg, rollup)
}

/// (c) The judge, asked only about thread entries that mention subject and object; its
/// `preserved` locates the passage.
fn judged(
    diary: &Diary,
    c: &Claim,
    names: &[String],
    judge: &dyn Judge<Error = Error>,
) -> Result<Option<(String, String)>, Error> {
    let asks = |c: &Claim, text: &str| {
        let question = Question {
            rollup: "grounding".into(),
            id: c.number.clone(),
            statement: c.statement.clone(),
            passage: text.chars().take(12000).collect(),
        };
        mentions(c, text)
            && judge
                .judge(&question)
                .is_ok_and(|v| v["verdict"] == "preserved")
    };
    search(diary, c, names, asks)
}

/// SPEC §5.1 row 1: above 0.80 the finding is checked again and must still hold — a stale claim
/// still has its later claim; an unsupported claim's source still does not support it.
pub fn second_check(ctx: &Ctx, finding: &Value, row: &Value) -> Result<bool, Error> {
    let kg = crate::paths::input(&ctx.paths.kg)?;
    let id = row["id"].as_str().unwrap_or_default();
    match finding["kind"].as_str().unwrap_or_default() {
        "stale" => Ok(palace_store::auto::later_claim(&ctx.compiler()?, &kg, id)?.is_some()),
        "unsupported" => {
            let c = claim_of(row);
            let diary = Diary::open(&ctx.root.join("diary"))?;
            let name = c.source.strip_suffix(".md").unwrap_or(&c.source);
            Ok(diary
                .read(name)?
                .is_some_and(|text| unsupported_by(&c, &text)))
        }
        _ => Ok(true),
    }
}

/// `table:<file>` (JSON `{"C5": "reversed"}`; any other claim `preserved`), `replay`,
/// `claude-cli[:model]` or `command:<exe>`.
pub fn judge(spec: &str) -> Result<BoxedJudge, Error> {
    if let Some(file) = spec.strip_prefix("table:") {
        let text = std::fs::read(crate::paths::input(std::path::Path::new(file))?)?;
        return Ok(Box::new(Table(palace_judge::table::TableJudge(
            serde_json::from_slice(&text)?,
        ))));
    }
    crate::watch::judge_spec(spec)
}

struct Table(palace_judge::table::TableJudge);
impl Judge for Table {
    type Error = Error;
    fn judge(&self, q: &Question) -> Result<Value, Error> {
        Ok(self.0.judge(q)?)
    }
}
