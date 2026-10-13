#![forbid(unsafe_code)]
mod auto;
mod auto_ground;
mod canary_out;
mod context;
mod context_layered;
mod delivery;
mod depends;
mod digest;
mod distill;
mod distill_commit;
// The acceptance doctors, their test kit and the fixture-backed tests (PUB-1a item 3): private,
// in src/doctors/; `cfg(doctors)` is build.rs's (Cargo.toml `doctors`).
#[cfg(doctors)]
mod doctors;
#[cfg(doctors)]
use doctors::*;
mod doctor_m3c;
mod escalate;
mod fidelity_cmd;
mod fidelity_replay;
mod findings;
mod graph;
mod graph_commands;
mod improve;
mod init;
mod paths;
#[cfg(doctors)] // used by the doctors only (and by tests/pin_failclosed.rs, through #[path])
mod pin_probe;
mod recovery;

mod regress;
mod reply;
mod rollup;
mod rollup_commit;
mod rollup_keep;
mod rollup_reference;
mod rollup_write;
mod scratch;
mod shadow_diff;
mod shadow_report;
mod terms;
mod watch;
use clap::{Args, Parser, Subcommand};
use palace_store::Error;
use std::path::PathBuf;
#[derive(Parser)]
#[command(name = "palace", version)]
struct Cli {
    #[arg(long, global = true, num_args = 1..)]
    roots: Vec<PathBuf>,
    /// The script whose `PIN=` line is the MemPalace pin (else $PALACE_MEMPALACE_CHECK, else the
    /// repository's scripts/mempalace-check.sh).
    #[arg(long, global = true)]
    mempalace_check: Option<PathBuf>,
    /// The person's `--by` name, who corrects and replies (PKG-1.2 item 4). A development build
    /// with owner names has a default; any other build has none: every command but `context` and
    /// `init` needs it.
    #[arg(long, global = true)]
    person: Option<String>,
    /// The assistant's `--by` name, whose queue holds the reversals (as `--person`).
    #[arg(long, global = true)]
    assistant: Option<String>,
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    Watch {
        #[command(flatten)]
        args: watch::Options,
    },
    Traverse {
        #[command(flatten)]
        args: graph_commands::ReadOptions,
    },
    Compile {
        #[command(flatten)]
        args: graph_commands::ReadOptions,
        #[arg(long)]
        task: palace_core::compile::Task,
    },
    RetireEntity {
        #[command(flatten)]
        paths: graph::GraphPaths,
        name: String,
        #[arg(long)]
        reason: String,
        #[arg(long)]
        supplied_by: String,
    },
    RetireFixtures {
        #[command(flatten)]
        paths: graph::GraphPaths,
        /// Default: the assistant.
        #[arg(long)]
        supplied_by: Option<String>,
    },
    Stress {
        #[command(subcommand)]
        operation: graph_commands::StressCommand,
    },
    ProposeBatch {
        #[command(flatten)]
        paths: graph::GraphPaths,
        #[arg(long)]
        diary: PathBuf,
        #[command(flatten)]
        connection: MemPalaceOptions,
        #[arg(long)]
        file: PathBuf,
        #[arg(long)]
        supplied_by_default: Option<String>,
        /// Run every gate check and print `{ok, reason}` per proposal; the graph is never opened.
        #[arg(long)]
        dry_run: bool,
    },
    Kg {
        #[command(subcommand)]
        operation: graph::Operation,
    },
    Doctor {
        #[command(flatten)]
        args: doctor_m3c::Options,
    },
    Rollup {
        #[command(flatten)]
        args: rollup::Options,
    },
    Init {
        #[command(flatten)]
        args: init::Options,
    },
    /// SETTLE-1b: one transcript's new text distilled into entries, typed claims and relations.
    Distill {
        #[command(flatten)]
        args: distill::Options,
    },
    /// SETTLE-1b: recall at both compression points (observe only).
    Fidelity {
        #[command(flatten)]
        args: fidelity_cmd::Options,
    },
    /// M4d: the claim store's findings (list, JSON, or Signal lines with reply tokens).
    Findings {
        #[command(flatten)]
        args: findings::Options,
    },
    /// M4d: the provenance around one claim (read-only).
    Depends {
        #[command(flatten)]
        args: depends::Options,
    },
    /// M4d: apply one improvement verb (or undo one event) to a claim; `--auto`; `--reply`.
    Improve {
        #[command(flatten)]
        args: improve::Options,
    },
    /// M4d item 8: terms against the survival bar; `--adopt`; `--digest`.
    Terms {
        #[command(flatten)]
        args: terms::Options,
    },
    /// M4d item 8: the lexicon, compiled from the term nodes.
    Lexicon {
        #[command(flatten)]
        args: terms::LexiconOptions,
    },
    /// M5a: the shadow night's comparison and reports (run by scripts/palace-shadow.sh).
    ShadowReport {
        #[command(flatten)]
        args: shadow_report::Options,
    },
    /// M5c: the per-turn context blocks for one message (read-only; fail-open).
    Context {
        #[command(flatten)]
        args: context::Options,
    },
    /// M4c: frozen queries through the pinned MemPalace vs the frozen answers.
    Regress {
        #[command(flatten)]
        args: regress::Options,
    },
}
impl Command {
    /// Commands that reach MemPalace (or its install) and therefore need the pin.
    fn reads_pin(&self) -> bool {
        match self {
            Self::Watch { .. }
            | Self::ProposeBatch { .. }
            | Self::Rollup { .. }
            | Self::Distill { .. }
            | Self::Init { .. }
            | Self::Regress { .. } => true,
            Self::Improve { args } => !args.digest && args.classify.is_none(),
            Self::Doctor { args } => {
                cfg!(doctors)
                    && !args.verify
                    && !args.check_divergence
                    && !args.shadow_diff
                    && args.settle.is_none()
            }
            _ => false,
        }
    }
}
/// M4d-1 0b: the pin is resolved once, before any subcommand; a missing script or `PIN=` line is
/// a broken configuration (exit 2) naming the path tried, never "MemPalace unavailable" (3).
fn resolve_pin(flag: Option<&std::path::Path>) -> Result<(), Error> {
    let script = palace_mempalace::version::choose_script(flag);
    let path = paths::input(&script).map_err(|e| {
        format!(
            "MemPalace pin: cannot read {} ({e}); pass --mempalace-check <path> or set {}",
            script.display(),
            palace_mempalace::version::SCRIPT_ENV
        )
    })?;
    palace_mempalace::version::use_script(&path)?;
    palace_mempalace::version::pin().map(drop)
}
/// A MemPalace wing name as a flag value: MemPalace's own rule, with nothing to trim.
fn wing_name(value: &str) -> Result<String, String> {
    match palace_core::filters::optional_name(Some(value)) {
        Ok(Some(name)) if name == value => Ok(name),
        _ => Err(format!("invalid wing name {value:?}")),
    }
}
#[derive(Args, Clone)]
struct MemPalaceOptions {
    #[arg(long, default_value = "mempalace-mcp")]
    mempalace_mcp: PathBuf,
    #[arg(long, default_value = "mempalace")]
    mempalace: PathBuf,
    #[arg(long)]
    allow_mempalace_version: bool,
}
impl MemPalaceOptions {
    fn evidence(
        &self,
        palace: &std::path::Path,
        diary: &std::path::Path,
    ) -> Result<palace_mempalace::Evidence, Error> {
        let palace = paths::database(palace)?.canonicalize()?;
        if !palace.is_dir() {
            return Err("--palace must be a directory".into());
        }
        Ok(palace_mempalace::Evidence {
            searcher: palace_mempalace::MemPalace::open(
                &palace,
                &self.mempalace_mcp,
                &self.mempalace,
                self.allow_mempalace_version,
            )?,
            diary: palace_mempalace::diary::Diary::open(&paths::input(diary)?)?,
        })
    }
}
fn main() {
    if let Err(error) = run() {
        let error = palace_store::busy::named(error);
        eprintln!("palace: {error}");
        std::process::exit(if palace_store::busy::Busy::is(&error) {
            3
        } else {
            1
        });
    }
}
fn run() -> Result<(), Error> {
    let cli = Cli::parse();
    palace_mempalace::version::set_default_script(paths::root().join("scripts/mempalace-check.sh"));
    // Before dispatch an error is "broken" (2), never a command's own exit 1 ("findings",
    // "differences"; M5b-1 item 9, review S-6).
    let explicit = explicit_names(&cli);
    let names = palace_core::names::set(cli.person.as_deref(), cli.assistant.as_deref());
    if let Err(error) = paths::configure(cli.roots).and(names.map_err(Error::from)) {
        if let Command::Context { args } = &cli.command {
            std::process::exit(context::failed(args.strict, &error.to_string()));
        }
        eprintln!("palace: {error}");
        std::process::exit(2);
    }
    if cli.command.reads_pin()
        && let Err(error) = resolve_pin(cli.mempalace_check.as_deref())
    {
        eprintln!("palace: {error}");
        std::process::exit(2);
    }
    // After the names' and the pin's own checks, so that each still names its own problem.
    if let Err(error) = explicit {
        eprintln!("palace: {error}");
        std::process::exit(2);
    }
    dispatch(cli.command)
}

/// PKG-1.3 item 4 (review S2): a release build (without the cargo feature `owner-defaults`)
/// never falls back to the owner's names: a command that may record or check a role needs both.
/// PUB-1a item 2: nor does a build without owner names (`palace_core::names::OWNER`).
fn explicit_names(cli: &Cli) -> Result<(), String> {
    let roles = !matches!(cli.command, Command::Context { .. } | Command::Init { .. });
    names_needed(
        cfg!(feature = "owner-defaults") && palace_core::names::OWNER.is_some(),
        roles,
        &cli.person,
        &cli.assistant,
    )
}
fn names_needed(
    defaults: bool,
    roles: bool,
    person: &Option<String>,
    assistant: &Option<String>,
) -> Result<(), String> {
    if defaults || !roles || (person.is_some() && assistant.is_some()) {
        return Ok(());
    }
    Err("this build has no default names: pass --person <name> --assistant <name>".into())
}

/// Commands that report through their exit code exit here; the rest return.
fn dispatch(command: Command) -> Result<(), Error> {
    let code = match command {
        Command::Watch { args } => watch_code(args),
        Command::ProposeBatch {
            paths,
            diary,
            connection,
            file,
            supplied_by_default,
            dry_run,
        } => graph::batch(
            paths,
            &connection,
            &diary,
            &file,
            graph::BatchMode {
                supplied_by_default: supplied_by_default.as_deref(),
                dry_run,
            },
        )?,
        Command::Rollup { args } => rollup::exit_code(rollup::run(args)),
        Command::Distill { args } => rollup::exit_code(distill::run(args)),
        Command::Fidelity { args } => fidelity_cmd::run(args),
        Command::Regress { args } => regress::main(args),
        Command::Findings { args } => findings::run(args),
        Command::Improve { args } => improve::run(args),
        Command::Depends { args } => depends::run(args),
        Command::Terms { args } => terms::run(args),
        Command::ShadowReport { args } => shadow_report::run(args),
        Command::Context { args } => context::run(args),
        other => return returning(other),
    };
    std::process::exit(code)
}

fn watch_code(args: watch::Options) -> i32 {
    watch::run(args).unwrap_or_else(|error| match crate::improve::fail("palace watch", error) {
        3 => 3,
        _ => 2,
    })
}

/// Commands that return `Ok` on success.
fn returning(command: Command) -> Result<(), Error> {
    match command {
        Command::Traverse { args } => graph_commands::read(args, None),
        Command::Compile { args, task } => graph_commands::read(args, Some(task)),
        Command::RetireEntity {
            paths,
            name,
            reason,
            supplied_by,
        } => graph_commands::retire(paths, &name, &reason, &supplied_by),
        Command::RetireFixtures { paths, supplied_by } => {
            let by = supplied_by
                .as_deref()
                .unwrap_or(palace_core::names::assistant());
            graph_commands::fixtures(paths, by)
        }
        Command::Stress { operation } => graph_commands::stress(operation),
        Command::Kg { operation } => graph::run(operation),
        Command::Doctor { args } => doctor_m3c::run(args),
        Command::Init { args } => init::run(args),
        Command::Lexicon { args } => terms::lexicon(args),
        _ => Err("command dispatched twice".into()),
    }
}

#[cfg(test)]
mod wing_tests {
    use super::*;
    fn wing(args: &[&str]) -> String {
        match Cli::try_parse_from(args).unwrap().command {
            Command::Rollup { args } => args.wing,
            Command::Init { args } => args.wing,
            _ => unreachable!(),
        }
    }
    #[test]
    fn rollup_and_init_mine_into_openclaw_unless_told() {
        let rollup = [
            "palace", "rollup", "--diary", "d", "--palace", "p", "--kg", "k",
        ];
        assert_eq!(wing(&rollup), "openclaw");
        assert_eq!(
            wing(&[&rollup[..], &["--wing", "memory"]].concat()),
            "memory"
        );
        assert_eq!(wing(&["palace", "init", "d", "p"]), "openclaw");
        assert_eq!(
            wing(&["palace", "init", "d", "p", "--wing", "work"]),
            "work"
        );
        assert!(Cli::try_parse_from(["palace", "init", "d", "p", "--wing", " work"]).is_err());
    }
}

#[cfg(test)]
mod name_tests {
    use super::*;
    fn roles(args: &[&str]) -> bool {
        let cli = Cli::try_parse_from(args).unwrap();
        !matches!(cli.command, Command::Context { .. } | Command::Init { .. })
    }
    #[test]
    fn a_release_build_needs_both_names_for_roles() {
        let (some, none) = (Some("maria".to_owned()), None);
        assert!(
            names_needed(true, true, &none, &none).is_ok(),
            "owner-defaults"
        );
        assert!(
            names_needed(false, false, &none, &none).is_ok(),
            "context, init"
        );
        assert!(names_needed(false, true, &some, &some).is_ok());
        for (person, assistant) in [(&none, &none), (&some, &none), (&none, &some)] {
            let refused = names_needed(false, true, person, assistant).unwrap_err();
            assert!(
                refused.contains("--person <name> --assistant <name>"),
                "{refused}"
            );
            for owner in owner_names() {
                assert!(!refused.contains(owner), "{refused}");
            }
        }
        let improve = ["palace", "improve", "--digest", "--store", "s"];
        assert!(roles(&improve));
        assert!(!roles(&["palace", "init", "d", "p"]));
        let context = [
            "palace",
            "context",
            "--message",
            "m",
            "--kg",
            "k",
            "--store",
            "s",
            "--palace",
            "p",
        ];
        assert!(!roles(&context));
    }
    #[test]
    fn no_help_names_the_owners() {
        use clap::CommandFactory;
        let mut cli = Cli::command();
        cli.build();
        let mut texts = vec![cli.render_long_help().to_string()];
        for sub in cli.get_subcommands_mut() {
            texts.push(sub.render_long_help().to_string());
        }
        for text in texts {
            for owner in owner_names() {
                assert!(!text.contains(owner), "{text}");
            }
        }
    }
    /// The owner's names compiled into this build, if any, and the invented cast.
    fn owner_names() -> Vec<&'static str> {
        let owner = palace_core::names::OWNER.map_or(vec![], |(p, a)| vec![p, a]);
        [owner, vec!["callum", "fergus"]].concat()
    }
}
