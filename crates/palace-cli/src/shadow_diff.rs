//! `palace doctor --shadow-diff --palace <copy> --kg <kg> --store <compiler.sqlite> [--wal <file>]
//! [--since <UTC time>] [--json]` (M5a item 2, review S-2): replays the palace write log and the
//! compiler's chained write log and reports whether every triple they touch is live or not, and
//! holds the `valid_to` and confidence they say; lists every row that differs. Read-only. Exit 0
//! equal, 1 different, 2 broken.
use crate::doctor_m3c::Options;
use palace_core::shadow_diff::{compare, merge};
use palace_store::Error;

pub fn run(o: &Options) -> i32 {
    match diff(o) {
        Ok(equal) => i32::from(!equal),
        Err(e) => {
            eprintln!("palace doctor --shadow-diff: {e}");
            2
        }
    }
}

fn diff(o: &Options) -> Result<bool, Error> {
    let input = |p: &Option<std::path::PathBuf>, flag: &str| {
        crate::paths::input(
            p.as_deref()
                .ok_or(format!("--shadow-diff requires {flag}"))?,
        )
    };
    let (palace, kg, store) = (
        input(&o.palace, "--palace")?,
        input(&o.kg, "--kg")?,
        input(&o.store, "--store")?,
    );
    let wal = o
        .wal
        .clone()
        .unwrap_or_else(|| palace.join("wal/write_log.jsonl"));
    let lines = palace_store::shadow_diff::wal_lines(&wal)?;
    let (rows, recoveries) = palace_store::shadow_diff::read(&store, &kg, o.since.as_deref())?;
    let outcome = compare(&merge(lines, recoveries), &rows)?;
    if o.json {
        println!("{}", outcome.json());
    } else {
        outcome.lines().iter().for_each(|l| println!("{l}"));
    }
    Ok(outcome.equal())
}
