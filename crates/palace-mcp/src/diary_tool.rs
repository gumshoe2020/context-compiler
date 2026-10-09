//! PKG-1.2 item 2: `palace_diary_write` files a diary entry as a file, by the diary plugin's
//! recipe (`palace_core::diary_file`, named by the local calendar day), then mines the diary root
//! into the palace with the pinned MemPalace, so the nightly rollup and watch read it. PKG-1.3
//! item 3: with `--wings-root`, an entry for another wing goes to `<wings-root>/<wing>/diary/`
//! and only that directory is mined: the nightly job, which compiles the diary root alone, never
//! reads it, and no later mine of the diary root can file it in the default wing. Item 6: the
//! mine has a deadline (`--mine-timeout-ms`, 20 s): past it MemPalace and its descendants are
//! ended (`child`), the file stays, and the reply says so. PKG-2 item 0: a mine MemPalace refuses
//! because another process holds the palace's lock is tried again, within the same deadline.
use crate::options::DiaryOptions;
use palace_mempalace::mine::Target;
use palace_store::Error;
use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub const NAME: &str = "palace_diary_write";
const MAX_TEXT: usize = 64 * 1024;
const MAX_TITLE: usize = 200;
/// MemPalace's refusal while another process holds the palace's mine lock (pinned 3.5.0:
/// `mempalace: palace <path> is held by <holder>; wait for it to finish …`).
const LOCK_HELD: &str = " is held by ";
/// The waits before each retry of a refused mine; a retry whose wait would pass the deadline is
/// not made. Any other failure is final.
const RETRY_WAITS: [Duration; 3] = [
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(4),
];

pub struct DiaryTool {
    options: DiaryOptions,
}
impl DiaryTool {
    pub fn new(options: DiaryOptions) -> Self {
        Self { options }
    }
    pub fn schema() -> Value {
        json!({"name":NAME,
            "description":"File a diary entry (a decision, a new fact, an action item, a thread to continue) as a file in the diary that the nightly memory job reads, then index it for search. Returns the file.",
            "inputSchema":{"type":"object","properties":{
                "text":{"type":"string","description":"The entry: what was decided and why, with names and dates."},
                "title":{"type":"string","description":"A one-line title."},
                "wing":{"type":"string","description":"The wing to file it in (default: this memory's own)."}},
            "required":["text"]}})
    }
    /// Writes the entry, then mines; a failed or ended mine still returns the file (the nightly
    /// job mines the diary root and every wing's directory first; another wing's next entry mines
    /// its directory), so a retry never files the entry twice. `mine_tries` when the mine was
    /// tried more than once.
    pub fn call(&self, args: &Value) -> Result<String, Error> {
        let entry = Entry::from(args, &self.options.wing)?;
        let root = self.root_of(&entry.wing)?;
        let path = self.write(&root, &entry)?;
        let mut tries = 0;
        let mined = self.mine(&root, &entry.wing, &mut tries);
        let mut result = json!({"file":path,"wing":entry.wing,"mined":mined.is_ok()});
        if tries > 1 {
            result["mine_tries"] = json!(tries);
        }
        if let Err(error) = mined {
            let reason: String = error.to_string().chars().take(500).collect();
            let later = if root == self.options.root {
                "the nightly job mines the diary".to_owned()
            } else {
                format!(
                    "the nightly job, or the next entry filed in wing {}, mines it",
                    entry.wing
                )
            };
            result["not_mined"] = json!(format!("{reason}; {later}"));
        }
        Ok(result.to_string())
    }
    /// The directory holding the `diary/` an entry of `wing` goes to: the diary root for this
    /// memory's own wing (and for every wing without `--wings-root`), else `<wings-root>/<wing>`.
    fn root_of(&self, wing: &str) -> Result<PathBuf, Error> {
        let o = &self.options;
        match &o.wings {
            Some(wings) if wing != o.wing => {
                let root = wings.join(wing);
                fs::create_dir_all(root.join("diary"))
                    .map_err(|e| format!("{}: {e}", root.display()))?;
                Ok(root)
            }
            _ => Ok(o.root.clone()),
        }
    }
    fn write(&self, root: &Path, entry: &Entry) -> Result<PathBuf, Error> {
        let millis = i64::try_from(SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis())?;
        let day = local_date(millis.div_euclid(1000))?;
        let file = palace_core::diary_file::render_on(&day, millis, &entry.title, &entry.text);
        let diary = root.join("diary");
        let path = absolute(&diary)?.join(&file.name);
        let mut out = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        out.write_all(file.content.as_bytes())?;
        Ok(path)
    }
    /// Mines `root` into `wing` within `--mine-timeout-ms`, counting the tries in `tries`.
    fn mine(&self, root: &Path, wing: &str, tries: &mut u32) -> Result<(), Error> {
        let o = &self.options;
        if let Some(check) = &o.check {
            let pin = palace_mempalace::version::pin_from(check)?;
            palace_mempalace::version::check_against(&o.mempalace, &pin, false)?;
        }
        let deadline = Instant::now() + o.mine_timeout;
        let mut waits = RETRY_WAITS.iter();
        loop {
            *tries += 1;
            let Some(failure) = self.mine_once(root, wing, deadline)? else {
                return Ok(());
            };
            match waits.next() {
                Some(wait) if failure.contains(LOCK_HELD) && Instant::now() + *wait < deadline => {
                    thread::sleep(*wait);
                }
                _ => return Err(failure.into()),
            }
        }
    }
    /// One mine within what is left of the deadline: `None` when it succeeded, else its failure.
    fn mine_once(
        &self,
        root: &Path,
        wing: &str,
        deadline: Instant,
    ) -> Result<Option<String>, Error> {
        let o = &self.options;
        let to = Target {
            wing,
            agent: &o.agent,
        };
        let command = palace_mempalace::mine::mine_command_to(
            &o.mempalace,
            &absolute(&o.palace)?,
            &absolute(root)?,
            &to,
        );
        let done = crate::child::run(command, deadline.saturating_duration_since(Instant::now()))?;
        match done.status {
            Some(status) if status.success() => Ok(None),
            Some(status) => {
                let stderr: String = done.stderr.chars().take(2048).collect();
                Ok(Some(format!("mempalace mine exited {status}: {stderr}")))
            }
            None => Err(format!(
                "mempalace mine did not finish within {} ms and was ended; the entry's file stays",
                o.mine_timeout.as_millis()
            )
            .into()),
        }
    }
}
fn absolute(path: &Path) -> Result<PathBuf, Error> {
    path.canonicalize()
        .map_err(|e| format!("{}: {e}", path.display()).into())
}

/// The arguments, checked: a text that is not blank, an optional one-line title, a wing name.
struct Entry {
    text: String,
    title: String,
    wing: String,
}
impl Entry {
    fn from(args: &Value, default_wing: &str) -> Result<Self, Error> {
        let field = |key: &str| match &args[key] {
            Value::Null => Ok(None),
            Value::String(s) => Ok(Some(s.clone())),
            _ => Err(format!("{key} must be a string")),
        };
        let text = field("text")?.unwrap_or_default();
        if text.trim().is_empty() || text.len() > MAX_TEXT {
            return Err(format!("text must be non-blank and at most {MAX_TEXT} bytes").into());
        }
        let title = field("title")?.unwrap_or_default();
        if title.contains(['\n', '\r']) || title.chars().count() > MAX_TITLE {
            return Err(format!("title must be one line of at most {MAX_TITLE} characters").into());
        }
        let wing = match field("wing")? {
            Some(wing) if !wing.trim().is_empty() => {
                palace_core::filters::optional_name(Some(&wing))?
                    .filter(|w| *w == wing)
                    .ok_or("invalid wing name")?
            }
            _ => default_wing.to_owned(),
        };
        Ok(Self { text, title, wing })
    }
}

/// `YYYY-MM-DD` of `seconds` in the local time zone (`TZ`, else the system's), from `date`.
pub fn local_date(seconds: i64) -> Result<String, Error> {
    let output = date_command(seconds).output()?;
    let day = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    let shape = day.len() == 10
        && day.char_indices().all(|(i, c)| {
            if i == 4 || i == 7 {
                c == '-'
            } else {
                c.is_ascii_digit()
            }
        });
    if !output.status.success() || !shape {
        return Err(format!("date -r {seconds}: {day:?} ({})", output.status).into());
    }
    Ok(day)
}
fn date_command(seconds: i64) -> Command {
    let mut command = Command::new("/bin/date");
    command
        .arg("-r")
        .arg(seconds.to_string())
        .arg("+%Y-%m-%d")
        .stdin(Stdio::null())
        .stderr(Stdio::null());
    command
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn local_date_follows_tz() {
        // 03:05Z on 2026-09-26 is the 25th in New York and the 26th in UTC.
        let at = palace_core::diary_file::days_from_civil(2026, 9, 26) * 86400 + 3 * 3600 + 300;
        for (tz, day) in [("America/New_York", "2026-09-25"), ("UTC", "2026-09-26")] {
            let output = date_command(at).env("TZ", tz).output().unwrap();
            assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), day, "{tz}");
        }
        assert_eq!(local_date(at).unwrap().len(), 10);
    }
    #[test]
    fn arguments_are_checked() {
        let entry = |args: Value| Entry::from(&args, "memory");
        let ok = entry(json!({"text":"x"})).unwrap();
        assert_eq!((ok.title.as_str(), ok.wing.as_str()), ("", "memory"));
        assert_eq!(
            entry(json!({"text":"x","wing":"work"})).unwrap().wing,
            "work"
        );
        assert_eq!(
            entry(json!({"text":"x","wing":" "})).unwrap().wing,
            "memory"
        );
        for bad in [
            json!({}),
            json!({"text":"  "}),
            json!({"text":7}),
            json!({"text":"x","title":"a\nb"}),
            json!({"text":"x","wing":"../x"}),
            json!({"text":"x".repeat(MAX_TEXT + 1)}),
        ] {
            assert!(entry(bad.clone()).is_err(), "{bad}");
        }
    }
}
