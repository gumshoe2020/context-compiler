//! Terms, minimal (M4d item 8; SPEC §2.4). `palace terms --store … --kg … [--adopt] [--digest]
//! [--as-of <date>] [--diary <root>]`: lists each term against the survival bar; `--adopt`
//! promotes the proposed terms that meet it (an event by `rule:adopt`); `--digest` prints at most
//! ten lines, the words coined in the as-of month with definition, claim count and `veto <word>`.
//! `palace lexicon --kg …` prints the compiled view. `veto <word>` (a reply by the person) retires a
//! term. No keystone command: keystone promotion is Callum's alone.
use clap::Args;
use palace_core::terms::{DIGEST_MAX, digest_line, survival};
use palace_store::{
    Error, Refused,
    claims::ClaimStore,
    graph::{Applied, GraphReader, TermChange},
    terms::{Term, terms, usage},
};
use serde_json::json;
use std::path::{Path, PathBuf};

#[derive(Args)]
pub struct Options {
    /// The claim store (compiler.sqlite beside the KG).
    #[arg(long)]
    pub store: PathBuf,
    #[command(flatten)]
    pub paths: crate::graph::GraphPaths,
    /// Promote every proposed term that meets the survival bar (event by rule:adopt).
    #[arg(long)]
    pub adopt: bool,
    /// The monthly digest: at most ten lines, words coined in the as-of month.
    #[arg(long, conflicts_with = "adopt")]
    pub digest: bool,
    /// YYYY-MM-DD (default: today, UTC).
    #[arg(long)]
    pub as_of: Option<String>,
    /// Diary root: `--adopt` writes each event's IMPROVE entry there (else the next `palace
    /// improve` call writes the missing entries).
    #[arg(long)]
    pub diary: Option<PathBuf>,
}

pub fn run(o: Options) -> i32 {
    match print(&o) {
        Ok(()) => 0,
        Err(e) => crate::improve::fail("palace terms", e),
    }
}

fn print(o: &Options) -> Result<(), Error> {
    let (compiler, kg) = stores(o)?;
    if o.adopt {
        return adopt(o, &compiler);
    }
    let all = terms(&compiler, &kg)?;
    if o.digest {
        return digest(o, &compiler, &kg, &all);
    }
    for term in &all {
        let u = usage(&compiler, &kg, term)?;
        let bar = survival(&u)
            .err()
            .unwrap_or_else(|| "meets the survival bar".into());
        println!(
            "TERM {} {} claims={} rollups={}: {bar}",
            term.word,
            term.status(),
            u.claims,
            u.rollups
        );
    }
    Ok(())
}

/// compiler.sqlite (it must be the store beside `--kg`) and the KG.
fn stores(o: &Options) -> Result<(PathBuf, PathBuf), Error> {
    let kg = crate::paths::input(&o.paths.kg)?;
    let compiler = crate::rollup_commit::compiler_path(&kg)?;
    if crate::paths::input(&o.store)? != compiler {
        return Err(Refused::error(format!(
            "--store {} is not the claim store beside --kg",
            o.store.display()
        )));
    }
    Ok((compiler, kg))
}

/// `--adopt`: each proposed term meeting the bar becomes `adopted` (rule:adopt), with its
/// IMPROVE entry when `--diary` is given.
fn adopt(o: &Options, compiler: &Path) -> Result<(), Error> {
    let mut graph = crate::graph::open(&o.paths)?;
    crate::recovery::reconcile(&mut graph, "terms")?;
    let kg = crate::paths::input(&o.paths.kg)?;
    let mut adopted = Vec::new();
    for term in terms(compiler, &kg)?
        .iter()
        .filter(|t| t.status() == "proposed")
    {
        match survival(&usage(compiler, &kg, term)?) {
            Ok(()) => adopted.push((term.word.clone(), adopt_one(&mut graph, term)?)),
            Err(why) => println!("KEEP {} proposed: {why}", term.word),
        }
    }
    drop(graph);
    for (word, applied) in &adopted {
        if let Some(root) = &o.diary {
            crate::improve::record_entry_at(&crate::paths::diary_root(root)?, compiler, applied)?;
        }
        println!("ADOPT {word} event={}", applied.event);
    }
    Ok(())
}

fn adopt_one(graph: &mut palace_store::graph::Graph, term: &Term) -> Result<Applied, Error> {
    let text = format!("adopted {}", term.word);
    graph.term_status(&TermChange {
        word: &term.word,
        status: "adopted",
        by: "rule:adopt",
        verb: "adopt",
        text: &text,
    })
}

/// At most ten lines: the words coined in the as-of month (not retired).
fn digest(o: &Options, compiler: &Path, kg: &Path, all: &[Term]) -> Result<(), Error> {
    let as_of = match &o.as_of {
        Some(date) => date.clone(),
        None => ClaimStore::open(compiler)?.now()?,
    };
    let month = as_of.get(..7).ok_or("--as-of must be YYYY-MM-DD")?;
    let coined = all.iter().filter(|t| {
        t.status() != "retired"
            && t.coined_at
                .as_deref()
                .is_some_and(|at| at.starts_with(month))
    });
    for term in coined.take(DIGEST_MAX) {
        let definition = term.properties["definition"].as_str().unwrap_or_default();
        println!(
            "{}",
            digest_line(&term.word, definition, usage(compiler, kg, term)?.claims)
        );
    }
    Ok(())
}

/// `veto <word>` (the person's reply): the term is retired, with its IMPROVE entry; idempotent per
/// reply text like every reply.
pub fn veto(ctx: &crate::improve::Ctx, word: &str, by: &str, text: &str) -> Result<String, Error> {
    let compiler = ctx.compiler()?;
    if let Some(event) = ClaimStore::open(&compiler)?.replied("#veto", text)? {
        return Ok(format!(
            "IMPROVE REPLY already applied {text} event={event}"
        ));
    }
    let mut graph = crate::graph::open(ctx.paths)?;
    let applied = graph.term_status(&TermChange {
        word,
        status: "retired",
        by,
        verb: "veto",
        text,
    })?;
    drop(graph);
    let entry = crate::improve::record_entry_at(&ctx.root, &compiler, &applied)?;
    ClaimStore::open(&compiler)?.record_reply("#veto", text, Some(&applied.event))?;
    Ok(format!(
        "IMPROVE OK veto {word} event={} reversible_by={} entry={entry}",
        applied.event, applied.reversible_by
    ))
}

#[derive(Args)]
pub struct LexiconOptions {
    #[arg(long)]
    pub kg: PathBuf,
}

/// `palace lexicon --kg …`: every term as `<word> (<kind>, <status>): <definition> — not:
/// <negative space>`, compiled from the term nodes on demand (read-only).
pub fn lexicon(o: LexiconOptions) -> Result<(), Error> {
    let reader = GraphReader::open(&crate::paths::input(&o.kg)?)?;
    let entities = reader.call("entities", &json!({}))?;
    for e in entities
        .as_array()
        .into_iter()
        .flatten()
        .filter(|e| e["type"] == "term")
    {
        let p = &e["properties"];
        let field = |k: &str| p[k].as_str().unwrap_or("-").to_owned();
        println!(
            "{} ({}, {}): {} — not: {}",
            e["name"].as_str().unwrap_or_default(),
            field("kind"),
            p["status"].as_str().unwrap_or("proposed"),
            field("definition"),
            field("negative_space")
        );
    }
    Ok(())
}
