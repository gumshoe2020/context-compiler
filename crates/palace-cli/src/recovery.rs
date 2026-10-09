//! `recover`'s intent half at the start of `palace improve`, `rollup`, `propose-batch`, `watch
//! --store` and (M4d-2c item 6) the writing `palace kg` operations (M4d-2a.1 item 2): every
//! unsettled intent is reconciled before the command writes anything. A diverged row prints `DIVERGED <table> <key> <column>` and exits 2 at once,
//! whatever the command's own exit-code table: nothing may be written over a row nobody logged.
use palace_store::{
    Error, Refused,
    graph::{Choice, Diverged, Graph},
};
use std::path::Path;

pub fn reconcile(graph: &mut Graph, command: &str) -> Result<(), Error> {
    match graph.reconcile() {
        Ok(acted) => {
            for line in acted {
                eprintln!(
                    "palace {command}: {} {} ({} row(s))",
                    line["operation"].as_str().unwrap_or("?"),
                    line["intent"].as_str().unwrap_or("?"),
                    line["rows"].as_array().map_or(0, Vec::len)
                );
            }
            Ok(())
        }
        Err(error) => match Diverged::of(&error) {
            Some(diverged) => {
                for line in &diverged.lines {
                    println!("{line}");
                }
                eprintln!(
                    "palace {command}: diverged row(s) of {}; nothing changed (see `palace doctor --check-divergence`); {}",
                    diverged.intent,
                    palace_core::reconcile::settle_hint(&diverged.intent)
                );
                std::process::exit(2)
            }
            None => Err(error),
        },
    }
}

/// For `watch --store`: the KG beside the claim store, when there is one (intents are only ever
/// written by a graph, which needs its KG).
pub fn beside_store(store: &Path) -> Result<(), Error> {
    let kg = store
        .parent()
        .ok_or("--store has no parent")?
        .join("knowledge_graph.sqlite3");
    if !kg.is_file() {
        return Ok(());
    }
    let mut graph = Graph::open(store, &kg, None)?;
    reconcile(&mut graph, "watch")
}

/// `palace doctor --settle <intent> --keep-current|--restore-before --by <person>|<assistant> --kg <kg>`
/// (M4d-2c item 6): a person settles a DIVERGED intent; prints `SETTLED <intent> <choice> by <who>
/// (<n> row(s))`.
pub fn settle(o: &crate::doctor_m3c::Options, intent: &str) -> Result<(), Error> {
    let choice = match (o.keep_current, o.restore_before) {
        (true, false) => Choice::KeepCurrent,
        (false, true) => Choice::RestoreBefore,
        _ => {
            return Err(Refused::error(
                "--settle needs --keep-current or --restore-before",
            ));
        }
    };
    let kg = o.kg.clone().ok_or("--settle requires --kg")?;
    let paths = crate::graph::GraphPaths {
        palace: o.palace.clone(),
        kg,
        wal: None,
    };
    let by = o.by.as_deref().unwrap_or_default();
    let line = crate::graph::open(&paths)?.settle(intent, choice, by)?;
    println!(
        "SETTLED {intent} {} by {by} ({} row(s))",
        line["choice"].as_str().unwrap_or("?"),
        line["rows"].as_array().map_or(0, Vec::len)
    );
    Ok(())
}

/// `palace doctor --check-divergence --kg <kg>`: the same reconciliation, read-only. Prints every
/// pending intent and diverged row; any is an error (exit 1).
pub fn check(kg: &Path) -> Result<(), Error> {
    let kg = crate::paths::input(kg)?;
    let compiler = crate::rollup_commit::compiler_path(&kg)?;
    let lines = palace_store::graph::check_divergence(&compiler, &kg)?;
    for line in &lines {
        println!("{line}");
    }
    if lines.is_empty() {
        println!("DIVERGENCE none");
        return Ok(());
    }
    Err(format!("{} pending intent or diverged line(s)", lines.len()).into())
}
