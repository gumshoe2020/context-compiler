//! `mempalace mine` against a palace copy. MemPalace resolves derived files (hallways.json,
//! tunnels.json) from `MEMPALACE_PALACE_PATH`, not `--palace`, so every child we spawn against a
//! palace sets it to that palace (PLAN Grounded facts, 2026-09-26). HOME is never changed.
use crate::Error;
use std::{
    path::Path,
    process::{Command, Stdio},
};
pub const PALACE_ENV: &str = "MEMPALACE_PALACE_PATH";
/// Environment for any MemPalace child working on `palace`.
pub fn palace_env(command: &mut Command, palace: &Path) {
    command
        .env(PALACE_ENV, palace)
        .env("HF_HUB_OFFLINE", "1")
        .env("TRANSFORMERS_OFFLINE", "1")
        .env("ANONYMIZED_TELEMETRY", "False");
}
/// The wing and agent of the diary plugin's mine, the default everywhere.
pub const DEFAULT_WING: &str = "openclaw";
pub const DEFAULT_AGENT: &str = "openclaw";
/// Where a mine files its drawers: `--wing` and `--agent` of `mempalace mine`.
pub struct Target<'a> {
    pub wing: &'a str,
    pub agent: &'a str,
}
impl Default for Target<'_> {
    fn default() -> Self {
        Self {
            wing: DEFAULT_WING,
            agent: DEFAULT_AGENT,
        }
    }
}
/// `mempalace --palace <palace> mine <root> --wing openclaw --agent openclaw` (the diary plugin's call).
pub fn mine_command(executable: &Path, palace: &Path, root: &Path) -> Command {
    mine_command_to(executable, palace, root, &Target::default())
}
/// `mempalace --palace <palace> mine <root> --wing <wing> --agent <agent>`.
pub fn mine_command_to(executable: &Path, palace: &Path, root: &Path, to: &Target) -> Command {
    let mut command = Command::new(executable);
    command
        .arg("--palace")
        .arg(palace)
        .arg("mine")
        .arg(root)
        .args(["--wing", to.wing, "--agent", to.agent])
        .stdin(Stdio::null());
    palace_env(&mut command, palace);
    command
}
/// Runs the mine; `palace` and `root` must be absolute. Returns MemPalace's stdout.
pub fn mine(executable: &Path, palace: &Path, root: &Path) -> Result<String, Error> {
    mine_to(executable, palace, root, &Target::default())
}
/// `mine` into `to.wing` as `to.agent`, saying so on stderr.
pub fn mine_to(
    executable: &Path,
    palace: &Path,
    root: &Path,
    to: &Target,
) -> Result<String, Error> {
    if !palace.is_absolute() || !root.is_absolute() {
        return Err("mine requires absolute palace and source paths".into());
    }
    eprintln!("mine: {PALACE_ENV}={} HOME unchanged", palace.display());
    mine_quietly(executable, palace, root, to)
}
/// `mine_to` without the stderr line (palace-mcp, whose stderr is the client's log).
pub fn mine_quietly(
    executable: &Path,
    palace: &Path,
    root: &Path,
    to: &Target,
) -> Result<String, Error> {
    if !palace.is_absolute() || !root.is_absolute() {
        return Err("mine requires absolute palace and source paths".into());
    }
    let output = mine_command_to(executable, palace, root, to).output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "mempalace mine exited {}: {}",
            output.status,
            stderr.chars().take(2048).collect::<String>()
        )
        .into());
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;
    #[test]
    fn mine_sets_palace_env_and_leaves_home_alone() {
        let command = mine_command(
            Path::new("mempalace"),
            Path::new("/p/target/m4b/palace"),
            Path::new("/p/target/m4b/diary-root"),
        );
        let envs: Vec<_> = command.get_envs().collect();
        assert!(envs.contains(&(
            OsStr::new(PALACE_ENV),
            Some(OsStr::new("/p/target/m4b/palace"))
        )));
        assert!(!envs.iter().any(|(k, _)| *k == "HOME"));
        let args: Vec<_> = command.get_args().collect();
        assert_eq!(
            args,
            [
                "--palace",
                "/p/target/m4b/palace",
                "mine",
                "/p/target/m4b/diary-root",
                "--wing",
                "openclaw",
                "--agent",
                "openclaw"
            ]
        );
    }
    #[test]
    fn mine_to_names_the_wing_and_agent() {
        let to = Target {
            wing: "memory",
            agent: "codex",
        };
        let command = mine_command_to(
            Path::new("mempalace"),
            Path::new("/p"),
            Path::new("/d"),
            &to,
        );
        let args: Vec<_> = command.get_args().collect();
        assert_eq!(
            args,
            [
                "--palace", "/p", "mine", "/d", "--wing", "memory", "--agent", "codex"
            ]
        );
    }
}
