//! The MemPalace pin. Its one source of truth is the `PIN=` line of
//! `scripts/mempalace-check.sh` (M4b.2 0c): every command reads it at run time from the script
//! given by `--mempalace-check`, else `$PALACE_MEMPALACE_CHECK`, else the repository's own script
//! (M4d-1 0b). A missing script or `PIN=` line is a broken configuration, never "unavailable".
use crate::Error;
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::OnceLock,
    thread,
    time::{Duration, Instant},
};

/// The environment variable naming the check script when `--mempalace-check` is not given.
pub const SCRIPT_ENV: &str = "PALACE_MEMPALACE_CHECK";

static SCRIPT: OnceLock<PathBuf> = OnceLock::new();
static PIN: OnceLock<String> = OnceLock::new();
static DEFAULT_SCRIPT: OnceLock<PathBuf> = OnceLock::new();

/// Names the check script used when neither the flag nor the environment does: the binary's own
/// `<root>/scripts/mempalace-check.sh` (palace sets it first thing; PKG-1.2 item 5 keeps the
/// build path out of this crate).
pub fn set_default_script(script: PathBuf) {
    let _ = DEFAULT_SCRIPT.set(script);
}

/// The script `set_default_script` named, else `scripts/mempalace-check.sh` in the working
/// directory.
pub fn default_script() -> PathBuf {
    DEFAULT_SCRIPT
        .get()
        .cloned()
        .unwrap_or_else(|| PathBuf::from("scripts/mempalace-check.sh"))
}

/// `flag` (from `--mempalace-check`) wins, then `$PALACE_MEMPALACE_CHECK`, then the default.
pub fn choose_script(flag: Option<&Path>) -> PathBuf {
    flag.map(Path::to_path_buf)
        .or_else(|| {
            std::env::var_os(SCRIPT_ENV)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        })
        .unwrap_or_else(default_script)
}

/// The pin's source for this process: the script set by `use_script`, else `choose_script(None)`.
pub fn script() -> PathBuf {
    SCRIPT.get().cloned().unwrap_or_else(|| choose_script(None))
}

/// Makes `script` the pin's source for this process.
pub fn use_script(script: &Path) -> Result<(), Error> {
    let chosen = SCRIPT.get_or_init(|| script.to_owned());
    if chosen != script {
        return Err(format!("pin script already set to {}", chosen.display()).into());
    }
    Ok(())
}

/// The pinned version (e.g. `3.5.0`), read once from `script()` and then cached, so a script that
/// disappears mid-run cannot turn into a transport failure later.
pub fn pin() -> Result<String, Error> {
    if let Some(pin) = PIN.get() {
        return Ok(pin.clone());
    }
    let pin = pin_from(&script())?;
    Ok(PIN.get_or_init(|| pin).clone())
}

/// The `PIN=` value of `script`, with a clear error naming the path when the script or the line
/// is absent.
pub fn pin_from(script: &Path) -> Result<String, Error> {
    let text = fs::read_to_string(script).map_err(|e| {
        format!(
            "MemPalace pin: cannot read {} ({e}); pass --mempalace-check <path to mempalace-check.sh> or set {SCRIPT_ENV}",
            script.display()
        )
    })?;
    palace_core::regress::parse_pin(&text)
        .map_err(|e| format!("MemPalace pin: {e} in {}", script.display()).into())
}

/// `mempalace --version` must equal `MemPalace <pin>` unless `allow`.
pub fn check(executable: &Path, allow: bool) -> Result<String, Error> {
    check_against(executable, &pin()?, allow)
}

pub fn check_against(executable: &Path, pin: &str, allow: bool) -> Result<String, Error> {
    let version = read(executable)?;
    let wanted = format!("MemPalace {pin}");
    if version != wanted {
        if !allow {
            return Err(
                format!("requires {wanted} (PIN= in the check script), found {version:?}").into(),
            );
        }
        eprintln!("allow-mempalace-version: expected {wanted}, found {version:?}");
    }
    Ok(version)
}
/// `mempalace --version` output (e.g. `MemPalace 3.5.0`), no pin comparison; 5 s deadline.
pub fn read(executable: &Path) -> Result<String, Error> {
    let mut child = Command::new(executable)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let mut stdout = child.stdout.take().ok_or("missing version stdout")?;
    let reader = thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout
            .by_ref()
            .take(4096)
            .read_to_end(&mut bytes)
            .map(|_| bytes)
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err("MemPalace version check timed out".into());
        }
        thread::sleep(Duration::from_millis(10));
    };
    let text = String::from_utf8(reader.join().map_err(|_| "version reader panicked")??)?;
    let version = text.trim_end_matches(['\r', '\n']);
    if !status.success() {
        return Err("MemPalace version command failed".into());
    }
    Ok(version.to_owned())
}
