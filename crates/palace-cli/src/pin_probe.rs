//! M4d-1 0b: every pin-reading command fails closed (exit 2, the path named) on a missing check
//! script or one without a `PIN=` line, whether the script comes from `--mempalace-check` or
//! `$PALACE_MEMPALACE_CHECK`; the flag wins over the variable. Shared by the M4d-1 doctor's
//! `pin_failclosed` gate. Scratch copies of the script live under `work`; the repository's
//! script is only read.
use palace_mempalace::version::SCRIPT_ENV;
use palace_store::Error;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

/// Each pin-reading command with arguments that parse but name nothing real under `work`.
pub fn commands(work: &Path) -> Vec<(&'static str, Vec<String>)> {
    COMMANDS
        .iter()
        .map(|(name, parts)| {
            let argv = parts
                .iter()
                .map(|part| match part.strip_prefix('@') {
                    Some(file) => work.join(file).display().to_string(),
                    None => (*part).to_owned(),
                })
                .collect();
            (*name, argv)
        })
        .collect()
}

/// Each pin-reading command's argv; `@x` is the scratch file `x` under the work directory.
const COMMANDS: [(&str, &[&str]); 5] = [
    (
        "rollup",
        &["rollup", "--diary", "@d", "--palace", "@p", "--kg", "@k"],
    ),
    ("watch", &["watch", "--no-judge"]),
    (
        "propose-batch",
        &[
            "propose-batch",
            "--kg",
            "@k",
            "--diary",
            "@d",
            "--file",
            "@f",
        ],
    ),
    ("init", &["init", "@d", "@p"]),
    (
        "regress",
        &[
            "regress",
            "--palace",
            "@p",
            "--queries",
            "@q",
            "--expected",
            "@e",
        ],
    ),
];

/// Writes `valid.sh` (a copy of `script`), `nopin.sh` (the copy without its `PIN=` lines) and
/// `twopin.sh` (the copy with its `PIN=` line repeated, M4d-2a item 7).
pub fn scratch_scripts(script: &Path, work: &Path) -> Result<(), Error> {
    fs::create_dir_all(work)?;
    let text = fs::read_to_string(script)?;
    fs::write(work.join("valid.sh"), &text)?;
    let nopin: Vec<&str> = text
        .lines()
        .filter(|l| !l.trim().starts_with("PIN="))
        .collect();
    fs::write(work.join("nopin.sh"), nopin.join("\n"))?;
    let pin = text
        .lines()
        .find(|l| l.trim().starts_with("PIN="))
        .ok_or("the check script has no PIN= line")?;
    fs::write(work.join("twopin.sh"), format!("{pin}\n{text}"))?;
    Ok(())
}

/// Runs every probe with `palace` (the binary under test), each child given `roots` (none: its
/// default) so a doctor on a copy outside the project probes under the same roots (review L-1);
/// `Err` names the first that failed.
pub fn run_all(palace: &Path, work: &Path, roots: &[PathBuf]) -> Result<usize, Error> {
    let missing = work.join("missing.sh");
    let (nopin, twopin) = (work.join("nopin.sh"), work.join("twopin.sh"));
    let mut probes = 0;
    for (name, args) in commands(work) {
        let args = with_roots(args, roots);
        for (script, says) in [
            (&missing, "missing.sh"),
            (&nopin, "no PIN= line"),
            (&twopin, "2 PIN= lines"),
        ] {
            expect_broken(palace, name, (&args, Some(script), None), says)?;
            expect_broken(palace, name, (&args, None, Some(script)), says)?;
            probes += 2;
        }
    }
    flag_wins(palace, work, roots)?;
    Ok(probes + 1)
}

fn with_roots(mut args: Vec<String>, roots: &[PathBuf]) -> Vec<String> {
    if !roots.is_empty() {
        args.push("--roots".into());
        args.extend(roots.iter().map(|r| r.display().to_string()));
    }
    args
}

type Probe<'a> = (&'a [String], Option<&'a Path>, Option<&'a Path>);

fn output(palace: &Path, (args, flag, env): Probe<'_>) -> Result<(Option<i32>, String), Error> {
    let mut command = Command::new(palace);
    command.args(args).env_remove(SCRIPT_ENV);
    if let Some(flag) = flag {
        command.arg("--mempalace-check").arg(flag);
    }
    if let Some(env) = env {
        command.env(SCRIPT_ENV, env);
    }
    let out = command.output()?;
    Ok((
        out.status.code(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    ))
}

fn expect_broken(palace: &Path, name: &str, probe: Probe<'_>, says: &str) -> Result<(), Error> {
    let (code, stderr) = output(palace, probe)?;
    if code == Some(2) && stderr.contains("MemPalace pin") && stderr.contains(says) {
        return Ok(());
    }
    Err(format!("pin_failclosed {name}: exit {code:?}, stderr {stderr:?}").into())
}

/// A valid `--mempalace-check` beats a missing `$PALACE_MEMPALACE_CHECK`: the run gets past the
/// pin (and then fails on its fake palace, which is not a pin error).
fn flag_wins(palace: &Path, work: &Path, roots: &[PathBuf]) -> Result<(), Error> {
    let args = with_roots(commands(work).remove(4).1, roots);
    let probe = (
        &args[..],
        Some(&*work.join("valid.sh")),
        Some(&*work.join("missing.sh")),
    );
    let (code, stderr) = output(palace, probe)?;
    if stderr.contains("MemPalace pin") || code == Some(0) {
        return Err(format!("pin_failclosed flag_wins: exit {code:?}, stderr {stderr:?}").into());
    }
    Ok(())
}
