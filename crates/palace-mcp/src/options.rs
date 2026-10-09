use palace_store::Error;
use std::{ffi::OsString, path::PathBuf, time::Duration};
pub struct Options {
    pub palace: Option<PathBuf>,
    pub kg: PathBuf,
    pub wal: Option<PathBuf>,
    /// PKG-1.2 item 2: with `--diary-root`, the tool `palace_diary_write` is offered.
    pub diary: Option<DiaryOptions>,
    /// PKG-1.3 item 4: `--person`, `--assistant` (the roles' `--by` names, as `palace`'s);
    /// without them, palace-core's defaults.
    pub names: (Option<String>, Option<String>),
    /// PKG-1.3 item 5: with `--palace-bin`, the tool `palace_reply` is offered.
    pub reply: Option<ReplyOptions>,
}
/// `--diary-root <dir> [--wing openclaw] [--agent openclaw] [--mempalace mempalace]
/// [--mempalace-check <script>] [--wings-root <dir>] [--mine-timeout-ms <ms>]`; the entries are
/// mined into `--palace`, which is then required.
pub struct DiaryOptions {
    pub root: PathBuf,
    /// PKG-1.3 item 3: with `--wings-root`, an entry for a wing other than `wing` is written under
    /// `<wings-root>/<wing>/diary/`, outside the compiled diary root, and mined from there alone.
    pub wings: Option<PathBuf>,
    /// PKG-1.3 item 6: `--mine-timeout-ms` (default 20 s), the mine's deadline.
    pub mine_timeout: Duration,
    pub palace: PathBuf,
    pub wing: String,
    pub agent: String,
    pub mempalace: PathBuf,
    pub check: Option<PathBuf>,
}
/// `--palace-bin <exe>`: `palace_reply` runs `<exe> improve --reply` on this memory's graph,
/// palace and diary root, as the person; needs `--diary-root`, `--person` and `--assistant`.
pub struct ReplyOptions {
    pub palace_bin: PathBuf,
    pub kg: PathBuf,
    pub palace: PathBuf,
    pub diary: PathBuf,
    pub mempalace: PathBuf,
    pub check: Option<PathBuf>,
    pub person: String,
    pub assistant: String,
}
const USAGE: &str = "expected --kg <sqlite> [--wal <path>] [--palace <path>] [--person <name> \
    --assistant <name>] [--diary-root <dir> \
    [--wing <name>] [--agent <name>] [--mempalace <exe>] [--mempalace-check <script>] \
    [--wings-root <dir>] [--mine-timeout-ms <ms>] [--palace-bin <exe>]]";
/// The mine's deadline when `--mine-timeout-ms` is not given (review S4).
const MINE_TIMEOUT_MS: u64 = 20_000;
#[derive(Default)]
struct Raw {
    palace: Option<OsString>,
    kg: Option<OsString>,
    wal: Option<OsString>,
    diary_root: Option<OsString>,
    wing: Option<OsString>,
    agent: Option<OsString>,
    mempalace: Option<OsString>,
    check: Option<OsString>,
    wings: Option<OsString>,
    person: Option<OsString>,
    assistant: Option<OsString>,
    palace_bin: Option<OsString>,
    mine_timeout: Option<OsString>,
}
impl Options {
    pub fn parse(mut args: impl Iterator<Item = OsString>) -> Result<Self, Error> {
        let mut raw = Raw::default();
        while let Some(flag) = args.next() {
            let slot = match flag.to_str() {
                Some("--palace") if raw.palace.is_none() => &mut raw.palace,
                Some("--kg") if raw.kg.is_none() => &mut raw.kg,
                Some("--wal") if raw.wal.is_none() => &mut raw.wal,
                Some("--diary-root") if raw.diary_root.is_none() => &mut raw.diary_root,
                Some("--wing") if raw.wing.is_none() => &mut raw.wing,
                Some("--agent") if raw.agent.is_none() => &mut raw.agent,
                Some("--mempalace") if raw.mempalace.is_none() => &mut raw.mempalace,
                Some("--mempalace-check") if raw.check.is_none() => &mut raw.check,
                Some("--wings-root") if raw.wings.is_none() => &mut raw.wings,
                Some("--person") if raw.person.is_none() => &mut raw.person,
                Some("--assistant") if raw.assistant.is_none() => &mut raw.assistant,
                Some("--palace-bin") if raw.palace_bin.is_none() => &mut raw.palace_bin,
                Some("--mine-timeout-ms") if raw.mine_timeout.is_none() => &mut raw.mine_timeout,
                _ => return Err(USAGE.into()),
            };
            *slot = Some(args.next().ok_or("missing argument value")?);
        }
        let palace = raw.palace.clone().map(PathBuf::from);
        let kg = raw
            .kg
            .clone()
            .map(PathBuf::from)
            .or_else(|| palace.as_ref().map(|p| p.join("knowledge_graph.sqlite3")))
            .ok_or("--kg or --palace is required")?;
        let text = |value: Option<OsString>| value.map(|v| v.into_string()).transpose();
        let names = (
            text(raw.person.take()).map_err(|_| "a non-UTF-8 name")?,
            text(raw.assistant.take()).map_err(|_| "a non-UTF-8 name")?,
        );
        let palace_bin = raw.palace_bin.take();
        let diary = diary(&mut raw, palace.as_ref())?;
        let reply = match palace_bin {
            Some(bin) => Some(reply(bin.into(), &kg, diary.as_ref(), &names)?),
            None => None,
        };
        Ok(Self {
            names,
            reply,
            diary,
            wal: raw.wal.map(PathBuf::from),
            palace,
            kg,
        })
    }
}
fn diary(raw: &mut Raw, palace: Option<&PathBuf>) -> Result<Option<DiaryOptions>, Error> {
    let Some(root) = raw.diary_root.take() else {
        let diary_only = [
            &raw.wing,
            &raw.agent,
            &raw.mempalace,
            &raw.check,
            &raw.wings,
            &raw.mine_timeout,
        ];
        if diary_only.iter().any(|flag| flag.is_some()) {
            return Err(
                "--wing, --agent, --mempalace, --mempalace-check, --wings-root and \
                --mine-timeout-ms need --diary-root"
                    .into(),
            );
        }
        return Ok(None);
    };
    let name = |value: Option<OsString>, default: &str| -> Result<String, Error> {
        let value = value.map_or(Ok(default.to_owned()), |v| {
            v.into_string().map_err(|_| "a non-UTF-8 name")
        })?;
        palace_core::filters::optional_name(Some(&value))?
            .filter(|v| *v == value)
            .ok_or_else(|| format!("invalid name {value:?}").into())
    };
    Ok(Some(DiaryOptions {
        root: PathBuf::from(root),
        wings: raw.wings.take().map(PathBuf::from),
        mine_timeout: Duration::from_millis(match raw.mine_timeout.take() {
            None => MINE_TIMEOUT_MS,
            Some(ms) => ms
                .to_str()
                .and_then(|ms| ms.parse().ok())
                .filter(|ms| *ms > 0)
                .ok_or("--mine-timeout-ms: a number of milliseconds above 0")?,
        }),
        palace: palace.cloned().ok_or("--diary-root needs --palace")?,
        wing: name(raw.wing.take(), palace_mempalace::mine::DEFAULT_WING)?,
        agent: name(raw.agent.take(), palace_mempalace::mine::DEFAULT_AGENT)?,
        mempalace: raw
            .mempalace
            .take()
            .map_or("mempalace".into(), PathBuf::from),
        check: raw.check.take().map(PathBuf::from),
    }))
}

fn reply(
    palace_bin: PathBuf,
    kg: &std::path::Path,
    diary: Option<&DiaryOptions>,
    names: &(Option<String>, Option<String>),
) -> Result<ReplyOptions, Error> {
    let diary = diary.ok_or("--palace-bin needs --diary-root")?;
    let (Some(person), Some(assistant)) = names else {
        return Err("--palace-bin needs --person and --assistant".into());
    };
    Ok(ReplyOptions {
        palace_bin,
        kg: kg.to_path_buf(),
        palace: diary.palace.clone(),
        diary: diary.root.clone(),
        mempalace: diary.mempalace.clone(),
        check: diary.check.clone(),
        person: person.clone(),
        assistant: assistant.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn parse(args: &[&str]) -> Result<Options, Error> {
        Options::parse(args.iter().map(OsString::from))
    }
    #[test]
    fn palace_default_and_explicit_override() {
        assert_eq!(
            parse(&["--palace", "target/p"]).unwrap().kg,
            PathBuf::from("target/p/knowledge_graph.sqlite3")
        );
        assert_eq!(
            parse(&["--palace", "target/p", "--kg", "target/explicit"])
                .unwrap()
                .kg,
            PathBuf::from("target/explicit")
        );
        assert!(Options::parse(std::iter::empty()).is_err());
        assert!(parse(&["--palace", "target/p"]).unwrap().diary.is_none());
        assert_eq!(parse(&["--palace", "p"]).unwrap().names, (None, None));
        assert!(parse(&["--palace", "p"]).unwrap().reply.is_none());
        let names = ["--person", "maria", "--assistant", "codex"];
        let diary = ["--diary-root", "/d", "--palace-bin", "/b/palace"];
        let with_reply = [&["--palace", "/p"][..], &names, &diary].concat();
        let reply = parse(&with_reply).unwrap().reply.unwrap();
        assert_eq!(
            (reply.palace_bin, reply.diary),
            ("/b/palace".into(), "/d".into())
        );
        assert_eq!(
            (reply.person.as_str(), reply.assistant.as_str()),
            ("maria", "codex")
        );
        assert!(
            parse(&[&["--palace", "/p"][..], &diary].concat()).is_err(),
            "no names"
        );
        let no_diary = [&["--palace", "/p"][..], &names, &["--palace-bin", "/b"]].concat();
        assert!(parse(&no_diary).is_err(), "no --diary-root");
        let named = parse(&["--palace", "p", "--person", "maria", "--assistant", "codex"]);
        let names = named.unwrap().names;
        assert_eq!(
            (names.0.as_deref(), names.1.as_deref()),
            (Some("maria"), Some("codex"))
        );
    }
    #[test]
    fn diary_flags() {
        let options = parse(&["--palace", "/p", "--diary-root", "/d"]).unwrap();
        let diary = options.diary.unwrap();
        assert_eq!(
            (diary.wing.as_str(), diary.agent.as_str()),
            ("openclaw", "openclaw")
        );
        assert_eq!(diary.mempalace, PathBuf::from("mempalace"));
        assert_eq!(diary.wings, None);
        assert_eq!(diary.mine_timeout, Duration::from_secs(20));
        let timed = parse(&[
            "--palace",
            "/p",
            "--diary-root",
            "/d",
            "--mine-timeout-ms",
            "500",
        ]);
        assert_eq!(
            timed.unwrap().diary.unwrap().mine_timeout,
            Duration::from_millis(500)
        );
        for bad in ["0", "x", "-1"] {
            let args = [
                "--palace",
                "/p",
                "--diary-root",
                "/d",
                "--mine-timeout-ms",
                bad,
            ];
            assert!(parse(&args).is_err(), "{bad}");
        }
        assert!(parse(&["--palace", "/p", "--mine-timeout-ms", "500"]).is_err());
        let options = parse(&[
            "--palace",
            "/p",
            "--diary-root",
            "/d",
            "--wing",
            "memory",
            "--agent",
            "codex",
            "--mempalace",
            "/u/mempalace",
            "--mempalace-check",
            "/c.sh",
        ])
        .unwrap();
        let diary = options.diary.unwrap();
        assert_eq!(
            (diary.wing.as_str(), diary.agent.as_str()),
            ("memory", "codex")
        );
        assert_eq!(diary.check, Some(PathBuf::from("/c.sh")));
        let with_wings = parse(&["--palace", "/p", "--diary-root", "/d", "--wings-root", "/w"]);
        assert_eq!(with_wings.unwrap().diary.unwrap().wings, Some("/w".into()));
        assert!(parse(&["--palace", "/p", "--wings-root", "/w"]).is_err());
        assert!(
            parse(&["--kg", "/k", "--diary-root", "/d"]).is_err(),
            "no --palace"
        );
        assert!(
            parse(&["--palace", "/p", "--wing", "memory"]).is_err(),
            "no --diary-root"
        );
        assert!(parse(&["--palace", "/p", "--diary-root", "/d", "--wing", "../x"]).is_err());
        assert!(parse(&["--palace", "/p", "--diary-root", "/d", "--wing", " m "]).is_err());
    }
}
