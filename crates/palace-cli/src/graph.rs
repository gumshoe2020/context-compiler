use clap::{Args, Subcommand};
use palace_core::{
    batch::parse_batch,
    write_gate::{dry_run, propose_relation},
};
use palace_store::{Error, graph::Graph};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
};
#[derive(Args)]
pub struct GraphPaths {
    #[arg(long)]
    pub palace: Option<PathBuf>,
    #[arg(long)]
    pub kg: PathBuf,
    #[arg(long)]
    pub wal: Option<PathBuf>,
}
#[derive(Subcommand)]
pub enum Operation {
    Query {
        #[command(flatten)]
        paths: GraphPaths,
        #[arg(long)]
        entity: String,
        #[arg(long, default_value = "both")]
        direction: String,
        #[arg(long)]
        as_of: Option<String>,
    },
    Invalidate {
        #[command(flatten)]
        paths: GraphPaths,
        #[arg(long)]
        subject: String,
        #[arg(long)]
        predicate: String,
        #[arg(long)]
        object: String,
        #[arg(long)]
        ended: Option<String>,
    },
    Entities {
        #[command(flatten)]
        paths: GraphPaths,
        #[arg(long)]
        names: Option<Vec<String>>,
    },
    SetEntity {
        #[command(flatten)]
        paths: GraphPaths,
        #[arg(long)]
        name: String,
        #[arg(long = "type")]
        r#type: Option<String>,
        #[arg(long)]
        properties: Option<String>,
    },
    RetireEntity {
        #[command(flatten)]
        paths: GraphPaths,
        #[arg(long)]
        name: String,
        #[arg(long)]
        by: String,
        #[arg(long)]
        reason: String,
        #[arg(long)]
        at: Option<String>,
    },
    Timeline {
        #[command(flatten)]
        paths: GraphPaths,
        #[arg(long)]
        entity: Option<String>,
    },
    Stats {
        #[command(flatten)]
        paths: GraphPaths,
    },
}
pub fn open(paths: &GraphPaths) -> Result<Graph, Error> {
    let kg = crate::paths::database(&paths.kg)?;
    let audit = crate::paths::database(
        &kg.parent()
            .ok_or("KG parent missing")?
            .join("compiler.sqlite"),
    )?;
    let wal = paths.wal.clone().unwrap_or_else(|| {
        paths
            .palace
            .as_deref()
            .unwrap_or(kg.parent().unwrap())
            .join("wal/write_log.jsonl")
    });
    let wal = crate::paths::database(&wal)?;
    Graph::open(&audit, &kg, Some(&wal))
}

pub fn run(operation: Operation) -> Result<(), Error> {
    let (paths, op, args) = match operation {
        Operation::Query {
            paths,
            entity,
            direction,
            as_of,
        } => (
            paths,
            "query",
            json!({"entity":entity,"direction":direction,"as_of":as_of}),
        ),
        Operation::Invalidate {
            paths,
            subject,
            predicate,
            object,
            ended,
        } => (
            paths,
            "invalidate",
            json!({"subject":subject,"predicate":predicate,"object":object,"ended":ended}),
        ),
        Operation::Entities { paths, names } => (paths, "entities", json!({"names":names})),
        Operation::SetEntity {
            paths,
            name,
            r#type,
            properties,
        } => (
            paths,
            "set_entity",
            json!({"name":name,"type":r#type,"properties":properties.map(|s|serde_json::from_str::<Value>(&s)).transpose()?}),
        ),
        Operation::RetireEntity {
            paths,
            name,
            by,
            reason,
            at,
        } => (
            paths,
            "retire_entity",
            json!({"name":name,"by":by,"reason":reason,"at":at}),
        ),
        Operation::Timeline { paths, entity } => (paths, "timeline", json!({"entity":entity})),
        Operation::Stats { paths } => (paths, "stats", json!({})),
    };
    let result = if matches!(op, "query" | "entities" | "timeline" | "stats") {
        palace_store::graph::GraphReader::open(&crate::paths::input(&paths.kg)?)?.call(op, &args)?
    } else {
        write(&paths, op, &args)?
    };
    println!("{result}");
    Ok(())
}
/// A writing `palace kg` operation: every unsettled intent is reconciled first (M4d-2c item 6).
fn write(paths: &GraphPaths, op: &str, args: &Value) -> Result<Value, Error> {
    let mut graph = open(paths)?;
    crate::recovery::reconcile(&mut graph, "kg")?;
    graph.call(op, args)
}
pub struct BatchMode<'a> {
    pub supplied_by_default: Option<&'a str>,
    pub dry_run: bool,
}
/// Exit codes: 0 batch ran (each proposal printed with its own verdict); 3 MemPalace unavailable.
pub fn batch(
    paths: GraphPaths,
    connection: &crate::MemPalaceOptions,
    diary: &Path,
    file: &Path,
    mode: BatchMode,
) -> Result<i32, Error> {
    let proposals = parse_batch(
        &fs::read_to_string(crate::paths::input(file)?)?,
        mode.supplied_by_default,
    )?;
    let palace = paths
        .palace
        .as_deref()
        .ok_or("propose-batch requires --palace")?;
    if !mode.dry_run {
        crate::recovery::reconcile(&mut open(&paths)?, "propose-batch")?;
    }
    let evidence = match connection.evidence(palace, diary) {
        Ok(evidence) => evidence,
        Err(error) => return Ok(unavailable(&error.to_string())),
    };
    propose_all(&paths, &evidence, proposals, mode.dry_run)?;
    evidence.diary.report_headers();
    Ok(evidence
        .searcher
        .unavailable()
        .map_or(0, |reason| unavailable(&reason)))
}
/// Each proposal through the write gate (or only its checks, `--dry-run`, without opening the
/// graph), one printed verdict per proposal.
fn propose_all(
    paths: &GraphPaths,
    evidence: &palace_mempalace::Evidence,
    proposals: Vec<palace_core::proposal::Proposal>,
    dry: bool,
) -> Result<(), Error> {
    if dry {
        for proposal in &proposals {
            println!("{}", dry_run(evidence, proposal));
        }
        return Ok(());
    }
    let mut graph = open(paths)?;
    for proposal in proposals {
        let now = graph.now()?;
        println!(
            "{}",
            propose_relation(evidence, &mut graph, &proposal, &now, "palace-rs")?
        );
    }
    Ok(())
}
fn unavailable(reason: &str) -> i32 {
    eprintln!("propose-batch: MemPalace unavailable: {reason}");
    3
}
