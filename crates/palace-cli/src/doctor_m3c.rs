use clap::Args;
use palace_store::Error;
use std::path::PathBuf;
#[derive(Args)]
pub struct Options {
    #[arg(long)]
    pub acceptance: Option<String>,
    #[arg(long)]
    pub verify: bool,
    /// M4d-2a.1: unsettled intents and diverged KG rows beside `--kg`, read-only (exit 1 on any).
    #[arg(long)]
    pub check_divergence: bool,
    /// M4d-2c item 6: settle a DIVERGED intent by hand (`--keep-current` or `--restore-before`,
    /// `--by <the person or the assistant>`, `--kg`); writes an audited `recovered_manual` line.
    #[arg(long, value_name = "INTENT")]
    pub settle: Option<String>,
    #[arg(long, requires = "settle", conflicts_with = "restore_before")]
    pub keep_current: bool,
    #[arg(long, requires = "settle")]
    pub restore_before: bool,
    /// `--settle` only: the person or the assistant (`palace --person/--assistant`).
    #[arg(long, requires = "settle")]
    pub by: Option<String>,
    #[arg(long)]
    audit: Option<PathBuf>,
    #[arg(long)]
    pub palace: Option<PathBuf>,
    #[arg(long)]
    pub diary: Option<PathBuf>,
    #[arg(long)]
    pub kg: Option<PathBuf>,
    #[arg(long)]
    pub rollups: Option<PathBuf>,
    #[arg(long)]
    pub expected_gate: Option<PathBuf>,
    #[arg(long)]
    pub expected_traverse: Option<PathBuf>,
    #[arg(long)]
    pub mcp: Option<PathBuf>,
    #[arg(long)]
    pub reports: Option<PathBuf>,
    #[arg(long)]
    pub canaries: Option<PathBuf>,
    #[arg(long)]
    pub expected_claims: Option<PathBuf>,
    #[arg(long)]
    pub expected: Option<PathBuf>,
    /// M4c: the frozen query list (fixtures/queries-50.json).
    #[arg(long)]
    pub queries: Option<PathBuf>,
    #[arg(long)]
    pub prompts: Option<PathBuf>,
    /// M4d-1: fixtures/improve (improve-rollup.md, void-supersedes-note.md).
    #[arg(long)]
    pub improve: Option<PathBuf>,
    /// M5a item 2: the graph against its two write logs (`--palace`, `--kg`, `--store`), read-only;
    /// exit 0 equal, 1 different, 2 broken.
    #[arg(long, requires_all = ["palace", "kg", "store"])]
    pub shadow_diff: bool,
    /// `--shadow-diff`: the compiler store whose chained write log is replayed.
    #[arg(long)]
    pub store: Option<PathBuf>,
    /// `--shadow-diff`: the palace write log (default `<palace>/wal/write_log.jsonl`).
    #[arg(long)]
    pub wal: Option<PathBuf>,
    /// `--shadow-diff`: only the store's recovery lines at or after this UTC time.
    #[arg(long)]
    pub since: Option<String>,
    /// M5a doctor: the shadow script under test (scripts/palace-shadow.sh).
    #[arg(long)]
    pub shadow_script: Option<PathBuf>,
    /// M5b-4 doctor: the scheduled-job wrapper under test (scripts/palace-cron.sh).
    #[arg(long)]
    pub cron_script: Option<PathBuf>,
    /// `--shadow-diff`: print JSON instead of lines.
    #[arg(long)]
    pub json: bool,
    #[command(flatten)]
    pub connection: crate::MemPalaceOptions,
}
pub fn run(options: Options) -> Result<(), Error> {
    if options.shadow_diff {
        std::process::exit(crate::shadow_diff::run(&options));
    }
    if let Some(intent) = &options.settle {
        return crate::recovery::settle(&options, intent);
    }
    if options.check_divergence {
        let kg = options.kg.as_deref();
        return crate::recovery::check(kg.ok_or("--check-divergence requires --kg")?);
    }
    if options.verify {
        return verify(options);
    }
    acceptance::run(options)
}
/// `--verify`: the write-log hash chain of `--audit`, or of the compiler.sqlite beside `--kg`.
fn verify(options: Options) -> Result<(), Error> {
    let path = options
        .audit
        .or_else(|| {
            options
                .kg
                .as_ref()
                .and_then(|p| p.parent())
                .map(|p| p.join("compiler.sqlite"))
        })
        .ok_or("--verify requires --audit or --kg")?;
    palace_store::audit::verify(&crate::paths::input(&path)?)?;
    println!("VERIFY OK chain=ok");
    Ok(())
}
/// `--acceptance <milestone>`: the doctors, in a development build only (cargo feature `doctors`,
/// PKG-1.2 item 5), private (src/doctors/, PUB-1a item 3).
#[cfg(doctors)]
use crate::doctor_m3c_acceptance as acceptance;
#[cfg(not(doctors))]
mod acceptance {
    use super::{Error, Options};
    pub fn run(options: Options) -> Result<(), Error> {
        match options.acceptance {
            Some(_) => {
                Err("the acceptance doctors are not in this build (cargo feature doctors)".into())
            }
            None => Err("specify --verify, --check-divergence, --settle or --shadow-diff".into()),
        }
    }
}
