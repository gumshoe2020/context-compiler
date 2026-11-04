//! INIT-1b: explicit-person ledger commands; no MemPalace or implicit owner defaults.
use clap::{Args, Subcommand};
use palace_core::{
    items,
    transcript::{self, Source, Turn},
};
use palace_store::{Error, claims::ClaimStore};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::{collections::BTreeMap, fs};

#[derive(Subcommand)]
pub enum Operation {
    Poked {
        #[arg(long)]
        store: PathBuf,
        #[arg(long)]
        item: String,
        #[arg(long)]
        at: String,
    },
    Skipped {
        #[arg(long, default_value = "4", value_parser = base_millis)]
        base_hours: i64,
        #[arg(long)]
        store: PathBuf,
        #[arg(long)]
        at: String,
    },
    Due {
        #[arg(long, default_value = "4", value_parser = base_millis)]
        base_hours: i64,
        #[arg(long)]
        store: PathBuf,
        #[arg(long)]
        transcript: PathBuf,
        #[arg(long, default_value = "now")]
        now: String,
        #[arg(long, default_value = "America/New_York")]
        tz: String,
        #[arg(long)]
        json: bool,
    },
    Replay {
        #[command(flatten)]
        args: ReplayOptions,
    },
    Update {
        #[command(flatten)]
        args: UpdateOptions,
    },
    List {
        #[arg(long)]
        store: PathBuf,
        #[arg(long, value_parser = ["open", "done", "dropped", "expired"])]
        status: Option<String>,
        #[arg(long)]
        json: bool,
    },
}
pub fn run(operation: Operation, person: Option<&str>) -> Result<(), Error> {
    match operation {
        Operation::Poked { store, item, at } => {
            let mut store = ClaimStore::open(&crate::paths::database(&store)?)?;
            store.initiative_poked(required_person(person)?, &item, &at)
        }
        Operation::Skipped {
            store,
            at,
            base_hours,
        } => {
            let mut store = ClaimStore::open(&crate::paths::database(&store)?)?;
            store.initiative_skipped(required_person(person)?, &at, base_hours)
        }
        Operation::Due {
            base_hours,
            store,
            transcript,
            now,
            tz,
            json: _,
        } => due(
            &store,
            required_person(person)?,
            &transcript,
            &now,
            &tz,
            base_hours,
        ),
        Operation::Replay { args } => replay(&args, required_person(person)?),
        Operation::Update { args } => {
            let person = required_person(person)?;
            let summary = update(&args, person)?;
            println!("ITEMS {}", json!(summary));
            Ok(())
        }
        Operation::List {
            store,
            status,
            json,
        } => {
            let store = ClaimStore::open(&crate::paths::database(&store)?)?;
            let rows = store.items(person, status.as_deref())?;
            if json {
                println!("{}", serde_json::to_string_pretty(&rows)?);
            } else {
                for row in rows {
                    println!(
                        "{}\t{}\t{}\t{}\t{}",
                        row["id"].as_str().unwrap_or_default(),
                        row["person"].as_str().unwrap_or_default(),
                        row["status"].as_str().unwrap_or_default(),
                        row["kind"].as_str().unwrap_or_default(),
                        row["title"].as_str().unwrap_or_default()
                    );
                }
            }
            Ok(())
        }
    }
}

#[derive(Args)]
pub struct UpdateOptions {
    #[arg(long)]
    pub store: PathBuf,
    #[arg(long)]
    pub transcript: PathBuf,
    /// First new physical line, inclusive and 1-based.
    #[arg(long, value_parser = clap::value_parser!(usize))]
    pub from_line: usize,
    /// Last new physical line, inclusive; default: all complete lines.
    #[arg(long)]
    pub to_line: Option<usize>,
    /// table:<JSON keyed by from-to> | command:<exe> | claude-cli[:model]
    #[arg(long)]
    pub llm: String,
    #[arg(long, default_value = "prompts")]
    pub prompts: PathBuf,
    #[arg(long, default_value_t = 600)]
    pub model_timeout: u64,
}
#[derive(Default, serde::Serialize)]
pub struct Summary {
    pub created: usize,
    pub updated: usize,
    pub closed: usize,
    pub expired: usize,
    pub rejected: usize,
}
fn required_person(person: Option<&str>) -> Result<&str, Error> {
    person
        .filter(|p| !p.trim().is_empty())
        .ok_or_else(|| "items command requires --person <name>".into())
}
/// Use the existing readers, one physical line at a time, so windows never parse later lines.
/// Format is recognized per line; tool/developer/injected messages remain excluded.
fn read_lines(text: &str, from: usize, to: usize) -> Result<Vec<(usize, Turn)>, Error> {
    let mut turns = Vec::new();
    for (i, line) in text
        .split_inclusive('\n')
        .enumerate()
        .skip(from - 1)
        .take(to - from + 1)
    {
        if !line.ends_with('\n') || line.trim().is_empty() {
            continue;
        }
        let v: Value =
            serde_json::from_str(line).map_err(|e| format!("transcript line {}: {e}", i + 1))?;
        let source = if v.get("role").is_some() {
            Source::OpenClaw
        } else if v["type"] == "response_item"
            || v["type"] == "event_msg"
            || v["type"] == "session_meta"
        {
            Source::Codex
        } else {
            Source::ClaudeCode
        };
        let read = transcript::read(source, line, 0)?;
        turns.extend(read.turns.into_iter().map(|(_, turn)| (i + 1, turn)));
    }
    Ok(turns)
}
fn update(args: &UpdateOptions, person: &str) -> Result<Summary, Error> {
    if args.from_line == 0 {
        return Err("--from-line is 1-based".into());
    }
    let path = crate::paths::input(&args.transcript)?;
    let text = fs::read_to_string(&path)?;
    let to = args
        .to_line
        .unwrap_or_else(|| text.bytes().filter(|b| *b == b'\n').count());
    if to < args.from_line {
        return Err("--to-line precedes --from-line".into());
    }
    let turns = read_lines(&text, args.from_line, to)?;
    let mut store = ClaimStore::open(&crate::paths::database(&args.store)?)?;
    if turns.is_empty() {
        return Ok(Summary::default());
    }
    let source_marker = store.privacy_source(&path.to_string_lossy())?;
    crate::privacy::sensitive(source_marker.private);
    let readers = source_marker.people.iter().cloned().collect::<Vec<_>>();
    let open = store.items_for_readers(Some(person), Some("open"), &readers)?;
    let mut inputs = vec![source_marker];
    for row in &open {
        if let Some(id) = row["id"].as_str() {
            inputs.extend(store.privacy_markers("item", id)?);
        }
    }
    store.item_model_inputs(palace_store::privacy::Marker::meet(
        &inputs,
        &path.to_string_lossy(),
    ));
    let mut prompt = format!(
        "{}\n\nPerson: {}\nOpen items: {}\n\nNew transcript lines ({}-{}):\n",
        include_str!("../../../prompts/items.md"),
        json!(person),
        json!(open),
        args.from_line,
        to
    );
    for (line, t) in &turns {
        prompt.push_str(&format!("L{line} [{}] [{}] {}\n", t.ts, t.role, t.text));
    }
    let raw = if let Some(file) = args.llm.strip_prefix("table:") {
        let table: Value = serde_json::from_str(&fs::read_to_string(crate::paths::input(
            &PathBuf::from(file),
        )?)?)?;
        table
            .get(format!("{}-{to}", args.from_line))
            .ok_or("items table has no reply for window")?
            .to_string()
    } else {
        crate::distill::model_spec(&args.llm, &args.prompts, args.model_timeout, None)?
            .complete(&prompt)?
    };
    let Some(reply) = palace_core::distill::json_of(&raw) else {
        reject(
            &mut store,
            person,
            &json!({"raw":raw}),
            "reply contains no JSON object",
            &path,
        )?;
        return Ok(Summary {
            rejected: 1,
            ..Summary::default()
        });
    };
    let Some(actions) = reply["actions"].as_array() else {
        reject(
            &mut store,
            person,
            &reply,
            "reply actions must be an array",
            &path,
        )?;
        return Ok(Summary {
            rejected: 1,
            ..Summary::default()
        });
    };
    let lines: BTreeMap<_, _> = turns.iter().map(|(n, t)| (*n, t.text.as_str())).collect();
    let mut summary = Summary::default();
    for action in actions {
        if let Err(why) = items::check(action, &lines) {
            reject(&mut store, person, action, &why, &path)?;
            summary.rejected += 1;
            continue;
        }
        let citation = if matches!(action["action"].as_str(), Some("close" | "expire")) {
            action.get("close_line").unwrap_or(&action["line"])
        } else {
            &action["line"]
        };
        let ts = &turns
            .iter()
            .find(|(n, _)| Some(*n as u64) == citation.as_u64())
            .ok_or("missing cited turn")?
            .1
            .ts;
        let cited_at = if ts.is_empty() {
            eprintln!("items: missing ts on line {citation}; using run clock");
            store.log(&json!({"operation":"items_timestamp_fallback","person":person,"line":citation,"transcript":path,"at":store.now()?}))?;
            None
        } else {
            if let Err(why) = items::timestamp_millis(ts) {
                reject(&mut store, person, action, &why, &path)?;
                summary.rejected += 1;
                continue;
            }
            Some(ts.as_str())
        };
        if let Err(error) = store.apply_item_at(person, action, &path.to_string_lossy(), cited_at) {
            if !palace_store::Refused::is(&error) {
                return Err(error);
            }
            reject(&mut store, person, action, &error.to_string(), &path)?;
            summary.rejected += 1;
            continue;
        }
        match action["action"].as_str() {
            Some("create") => summary.created += 1,
            Some("update") => summary.updated += 1,
            Some("close") => summary.closed += 1,
            Some("expire") => summary.expired += 1,
            _ => unreachable!(),
        }
    }
    Ok(summary)
}
fn reject(
    store: &mut ClaimStore,
    person: &str,
    action: &Value,
    why: &str,
    path: &std::path::Path,
) -> Result<(), Error> {
    privacy_log!("items: rejected: {why}");
    store.log(&json!({"operation":"items_rejected","person":person,"action":action,"reason":why,"transcript":path,"at":store.now()?}))
}

#[derive(Args)]
pub struct ReplayOptions {
    #[arg(long)]
    store: PathBuf,
    #[arg(long)]
    transcript: PathBuf,
    #[arg(long)]
    llm: String,
    #[arg(long, default_value_t = 30)]
    window_minutes: u64,
    #[arg(long, default_value = "prompts")]
    prompts: PathBuf,
    #[arg(long, default_value_t = 600)]
    model_timeout: u64,
}
fn replay(args: &ReplayOptions, person: &str) -> Result<(), Error> {
    let store = crate::paths::database(&args.store)?;
    if store.exists() {
        return Err("items replay requires a fresh scratch --store (path already exists)".into());
    }
    let transcript = crate::paths::input(&args.transcript)?;
    let text = fs::read_to_string(&transcript)?;
    let complete = text.bytes().filter(|b| *b == b'\n').count();
    let turns = if complete == 0 {
        Vec::new()
    } else {
        read_lines(&text, 1, complete)?
    };
    // Validate every clock before the first write/model call.
    let mut window_turns = turns.clone();
    let fallback = palace_core::items::timestamp_rfc3339(i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis(),
    )?);
    for (_, turn) in &mut window_turns {
        if turn.ts.is_empty() {
            turn.ts = fallback.clone();
        }
    }
    let windows = items::windows(&window_turns, args.window_minutes)?;
    let mut summary = Summary::default();
    for (from, to) in &windows {
        let result = update(
            &UpdateOptions {
                store: store.clone(),
                transcript: transcript.clone(),
                from_line: *from,
                to_line: Some(*to),
                llm: args.llm.clone(),
                prompts: args.prompts.clone(),
                model_timeout: args.model_timeout,
            },
            person,
        )?;
        summary.created += result.created;
        summary.updated += result.updated;
        summary.closed += result.closed;
        summary.expired += result.expired;
        summary.rejected += result.rejected;
    }
    let open = ClaimStore::open(&store)?
        .items(Some(person), Some("open"))?
        .len();
    let mut value = json!(summary);
    value["open"] = json!(open);
    value["windows"] = json!(windows.len());
    println!("ITEMS REPLAY {value}");
    Ok(())
}

const FOUR_HOURS: i64 = 4 * 3_600_000;
fn base_millis(value: &str) -> Result<i64, String> {
    let hours: f64 = value
        .parse()
        .map_err(|_| "base hours must be a positive number")?;
    let millis = (hours * 3_600_000.0).ceil();
    if !hours.is_finite() || hours <= 0.0 || millis >= i64::MAX as f64 {
        return Err("base hours must be a finite positive number within timestamp range".into());
    }
    Ok((millis as i64).max(1))
}
fn clock_millis(at: &str) -> Result<i64, Error> {
    if at == "now" {
        Ok(i64::try_from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_millis(),
        )?)
    } else {
        Ok(items::timestamp_millis(at)?)
    }
}
/// Conversation end includes all roles, unlike the text-only readers used for item extraction.
fn conversation_clocks(text: &str) -> Result<(i64, Option<i64>), Error> {
    let mut end = None;
    let mut inbound = None;
    for line in text
        .split_inclusive('\n')
        .filter(|s| s.ends_with('\n') && !s.trim().is_empty())
    {
        let v: Value = serde_json::from_str(line)?;
        let ts = v
            .get("ts")
            .or_else(|| v.get("timestamp"))
            .and_then(Value::as_str)
            .ok_or("conversation line has no timestamp")?;
        let at = items::timestamp_millis(ts)?;
        end = Some(at);
        let role = v["role"]
            .as_str()
            .or_else(|| v["payload"]["role"].as_str())
            .or_else(|| v["type"].as_str());
        if role == Some("user") {
            inbound = Some(at);
        }
    }
    Ok((end.ok_or("conversation has no complete lines")?, inbound))
}
fn due(
    db: &std::path::Path,
    person: &str,
    transcript: &std::path::Path,
    now: &str,
    tz: &str,
    base: i64,
) -> Result<(), Error> {
    let now = clock_millis(now)?;
    let (end, inbound) =
        conversation_clocks(&fs::read_to_string(crate::paths::input(transcript)?)?)?;
    let mut store = ClaimStore::open(&crate::paths::database(db)?)?;
    let state = store.initiative_state(person)?;
    let last = state["last_poke_at"]
        .as_str()
        .map(items::timestamp_millis)
        .transpose()?;
    let reset = last.is_none() || inbound.zip(last).is_some_and(|(i, p)| i > p);
    let pokes = if reset {
        0
    } else {
        state["pokes"].as_i64().unwrap_or(0)
    };
    let mut at = if reset {
        end.checked_add(base)
            .ok_or("base hours exceeds timestamp range")?
    } else {
        last.unwrap()
            + base
                .saturating_mul(2_i64.saturating_pow(pokes.clamp(0, 63) as u32))
                .min(168 * 3_600_000)
    };
    // A newer inbound cancels a stale backoff, but not a skip/check within this conversation.
    if let Some(next) = state["next_due_at"].as_str() {
        let check = state["last_check_at"]
            .as_str()
            .map(items::timestamp_millis)
            .transpose()?;
        if !reset || check.is_some_and(|c| inbound.is_none_or(|i| c >= i)) {
            at = at.max(items::timestamp_millis(next)?);
        }
    }
    let output = std::process::Command::new("python3")
        .args([
            "-c",
            include_str!("initiative_time.py"),
            &at.to_string(),
            &now.to_string(),
            tz,
        ])
        .output()?;
    if !output.status.success() {
        return Err(format!(
            "timezone conversion failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    let local: Value = serde_json::from_slice(&output.stdout)?;
    let at = items::timestamp_millis(local["due"].as_str().ok_or("invalid timezone reply")?)?;
    let next = items::timestamp_rfc3339(at);
    let mut candidates = Vec::new();
    if now >= at {
        for row in store.items(Some(person), Some("open"))? {
            if row["kind"] == "pastoral" {
                continue;
            }
            let touched =
                items::timestamp_millis(row["touched_at"].as_str().ok_or("missing touched_at")?)?;
            let minimum_age = if row["kind"] == "topic" {
                3 * 24 * 3_600_000
            } else {
                FOUR_HOURS
            };
            if now - touched < minimum_age {
                continue;
            }
            candidates.push((row, touched));
        }
        candidates.sort_by(|(a, ta), (b, tb)| {
            let rank = |v: &Value| {
                if v["due"]
                    .as_str()
                    .is_some_and(|d| d <= local["cutoff"].as_str().unwrap_or_default())
                {
                    0
                } else if v["owner"] == "person"
                    && matches!(v["kind"].as_str(), Some("decision" | "question"))
                {
                    1
                } else if v["kind"] == "topic" {
                    2
                } else {
                    3
                }
            };
            (rank(a), ta, a["id"].as_str()).cmp(&(rank(b), tb, b["id"].as_str()))
        });
    }
    store.initiative_checked(person, pokes, &items::timestamp_rfc3339(now), &next)?;
    if now < at {
        println!("{}", json!({"due":false,"next_due_at":next}));
    } else {
        let candidates: Vec<_> = candidates
            .into_iter()
            .take(5)
            .map(|(row, _)| {
                let mut v = serde_json::Map::new();
                for key in [
                    "id",
                    "title",
                    "kind",
                    "owner",
                    "due",
                    "touched_at",
                    "quote",
                    "source",
                ] {
                    v.insert(key.into(), row[key].clone());
                }
                Value::Object(v)
            })
            .collect();
        println!(
            "{}",
            json!({"due":true,"pokes":pokes,"candidates":candidates})
        );
    }
    Ok(())
}
