//! `palace improve --reply "<text>" [--token #f_…] --by <person>|<assistant>` (M4d item 5; SPEC §6 and
//! §5.1 revised 2026-09-27: action words): `keep` → confirm; `accept` → the proposal (an escalated
//! ask's claim as a contradict, else the finding's text-free verb); `drop` → retract; `fix:
//! <text>` → contradict with the text as the successor (parsed under the claim grammar first:
//! malformed → exit 1 naming the grammar problem, the ask stays open); `source: <file>` → for an
//! ungrounded ask, `add --supersede` citing the file when it exists under the diary root and
//! supports the claim, else exit 1 naming why; `ok`/`no` → refused listing the words; anything
//! else → usage, exit 1, no event. Without `--token` the reply applies only when exactly one ask
//! is open. Idempotent per (token, normalised text): a repeat is exit 0 `already applied`. The ask
//! is marked answered. `undo e_<id>` (M4d-2d item 3, from the corrections digest) is the person's:
//! the event is undone under the people-first and compare-and-swap rules, idempotent per text.
use crate::improve::{Call, Ctx, Options, apply_call, ok_line, recover};
use palace_core::{
    auto::{claim_of, grounded_statement},
    claim::locates,
    finding::reply_token,
    improve::statement,
    reply::{Reply, accepted, allowed, already_answered, normalise, parse},
};
use palace_mempalace::diary::{Diary, OutsideRoot};
use palace_store::{Error, Refused, claims::ClaimStore, lookup::FindingFilter};
use serde_json::Value;

pub fn run(o: &Options) -> i32 {
    match replied(o) {
        Ok(line) => {
            println!("{line}");
            0
        }
        Err(e) => crate::improve::fail("palace improve --reply", e),
    }
}

fn replied(o: &Options) -> Result<String, Error> {
    let by = o.by.as_deref().unwrap_or_default();
    let (person, assistant) = (
        palace_core::names::person(),
        palace_core::names::assistant(),
    );
    if !palace_core::names::is_person(by) {
        return Err(Refused::error(format!(
            "a reply is {person}'s or {assistant}'s (--by)"
        )));
    }
    let text = normalise(o.reply.as_deref().unwrap_or_default());
    let reply = parse(&text).map_err(Refused::error)?;
    let ctx = Ctx::of(o)?;
    recover(&ctx)?;
    if let Reply::Veto(word) = &reply {
        return crate::terms::veto(&ctx, word, by, &text);
    }
    if let Reply::Undo(event) = &reply {
        return undone(&ctx, event, by, &text);
    }
    let target = Target::of(&ctx, o.token.as_deref())?;
    if let Some(done) = settled(&ctx, &target, (&reply, &text))? {
        return Ok(done);
    }
    let (verb, body, supersede) = call_for(&ctx, &target, &reply)?;
    let call = Call {
        verb,
        target: target.finding_id(),
        text: body.as_deref(),
        by,
        supersede,
        grounding_failed: false,
    };
    let done = apply_call(&ctx, &call)?;
    answered(&ctx, &target, &text, &done.applied.event)?;
    Ok(format!("{} reply={}", ok_line(&done), target.token))
}

/// `undo e_<id>` by the person: the event's undo (a repeat is `already applied`, exit 0).
fn undone(ctx: &Ctx, event: &str, by: &str, text: &str) -> Result<String, Error> {
    let (person, assistant) = (
        palace_core::names::person(),
        palace_core::names::assistant(),
    );
    if by != person {
        return Err(Refused::error(format!(
            "undo e_… is {person}'s reply ({assistant}: palace improve undo)"
        )));
    }
    let mut store = ClaimStore::open(&ctx.compiler()?)?;
    if let Some(done) = store.replied(event, text)? {
        return Ok(format!(
            "IMPROVE REPLY already applied {event} event={done}"
        ));
    }
    drop(store);
    let call = Call {
        verb: "undo",
        target: event,
        text: None,
        by,
        supersede: false,
        grounding_failed: false,
    };
    let done = apply_call(ctx, &call)?;
    store = ClaimStore::open(&ctx.compiler()?)?;
    store.record_reply(event, text, Some(&done.applied.event))?;
    Ok(format!("{} reply=undo", ok_line(&done)))
}

/// Records the reply (idempotency) and marks the ask answered.
fn answered(ctx: &Ctx, target: &Target, text: &str, event: &str) -> Result<(), Error> {
    let mut store = ClaimStore::open(&ctx.compiler()?)?;
    store.record_reply(&target.token, text, Some(event))?;
    let at = store.now()?;
    store.answer_ask(target.finding_id(), text, &at)
}

/// A repeat of an applied reply (`Some`: already applied, exit 0); a reply to a decided finding
/// or with a word this target does not take is refused (M4d-2c item 3, review S-2).
fn settled(ctx: &Ctx, t: &Target, (reply, text): (&Reply, &str)) -> Result<Option<String>, Error> {
    let store = ClaimStore::open(&ctx.compiler()?)?;
    if let Some(event) = store.replied(&t.token, text)? {
        let token = &t.token;
        return Ok(Some(format!(
            "IMPROVE REPLY already applied {token} event={event}"
        )));
    }
    if let Some(decision) = t.finding["decision"].as_object() {
        let event = store.deciding_event(t.finding_id())?.unwrap_or_default();
        let by = decision.get("by").and_then(Value::as_str).unwrap_or("?");
        let verb = decision.get("verb").and_then(Value::as_str).unwrap_or("?");
        let answer = t.answer().unwrap_or(format!("{verb} by {by}"));
        return Err(Refused::error(already_answered(&answer, &event)));
    }
    allowed(reply, t.ungrounded()).map_err(Refused::error)?;
    Ok(None)
}

/// The finding a reply answers, its claim, its ask (if one was made) and its reply token.
struct Target {
    finding: Value,
    claim: Value,
    ask: Option<Value>,
    token: String,
}

impl Target {
    /// `--token` names the finding; without it, the one open ask (else exit 1).
    fn of(ctx: &Ctx, token: Option<&str>) -> Result<Self, Error> {
        let store = ClaimStore::open(&ctx.compiler()?)?;
        let finding_id = match token {
            Some(token) => store
                .resolve(token)?
                .finding
                .ok_or_else(|| Refused::error(format!("--token {token} names no finding")))?,
            None => only_ask(&store)?,
        };
        let finding = finding(&store, &finding_id)?;
        let claim_id = finding["claim"].as_str().unwrap_or_default();
        let claim = store
            .claim(claim_id)?
            .ok_or_else(|| Refused::error(format!("finding {finding_id} has no claim")))?;
        Ok(Self {
            token: reply_token(&finding_id),
            ask: store.ask(&finding_id)?,
            finding,
            claim,
        })
    }
    fn finding_id(&self) -> &str {
        self.finding["id"].as_str().unwrap_or_default()
    }
    fn kind(&self) -> &str {
        self.finding["kind"].as_str().unwrap_or_default()
    }
    /// The finding says no source was found (the ungrounded ask's target).
    fn ungrounded(&self) -> bool {
        self.finding["evidence"]["status"] == "absent-source"
    }
    /// The recorded answer of this finding's ask, when it was answered.
    fn answer(&self) -> Option<String> {
        Some(self.ask.as_ref()?["answer"].as_str()?.to_owned())
    }
    /// Fergus's proposed claim, when the ask was escalated.
    fn escalated(&self) -> Option<&str> {
        self.ask.as_ref()?["proposal"].as_str()
    }
}

fn only_ask(store: &ClaimStore) -> Result<String, Error> {
    let open = store.open_asks()?;
    match open.as_slice() {
        [ask] => Ok(ask["finding"].as_str().unwrap_or_default().to_owned()),
        _ => Err(Refused::error(format!(
            "{} open asks: give --token #f_… to say which",
            open.len()
        ))),
    }
}

pub fn finding(store: &ClaimStore, id: &str) -> Result<Value, Error> {
    store
        .findings(&FindingFilter::default())?
        .into_iter()
        .find(|f| f["id"] == id)
        .ok_or_else(|| Refused::error(format!("unknown finding {id}")))
}

/// The verb, text and `--supersede` a reply becomes.
fn call_for(
    ctx: &Ctx,
    t: &Target,
    reply: &Reply,
) -> Result<(&'static str, Option<String>, bool), Error> {
    match reply {
        Reply::Keep => Ok(("confirm", None, false)),
        Reply::Drop => Ok(("retract", None, false)),
        Reply::Accept => {
            let verb = t.finding["proposal"]["verb"].as_str().unwrap_or_default();
            let (verb, text) = accepted(t.kind(), verb, t.escalated()).map_err(Refused::error)?;
            Ok((verb.name(), text, false))
        }
        Reply::Fix(text) => fix(t, text).map(|text| ("contradict", Some(text), false)),
        Reply::Source(file) => source(ctx, t, file).map(|text| ("add", Some(text), true)),
        Reply::Veto(_) | Reply::Undo(_) => {
            Err("veto and undo are handled before a target is resolved".into())
        }
    }
}

/// `fix:` text parsed under the claim grammar before anything is written.
fn fix(t: &Target, text: &str) -> Result<String, Error> {
    grammatical(&t.claim, text)
        .map_err(|problem| Refused::error(format!("fix: {problem} (the ask stays open)")))
}

/// `text` as a successor of `claim` under the claim grammar (a `fix:` or an escalated proposal).
pub fn grammatical(claim: &Value, text: &str) -> Result<String, String> {
    palace_core::reply::tokenless(text)?;
    statement(text, "C0", &claim_of(claim)).map(|_| text.to_owned())
}

/// A `source:` file outside the diary root (a symlink out of it) is Callum's bad reply: refused
/// (exit 1), not broken (M4d-2d item 4, review L-2).
fn outside_refused(e: Error, file: &str) -> Error {
    if OutsideRoot::is(&e) {
        return Refused::error(format!("source: {file}: {e}"));
    }
    e
}

/// `source: <file>` for an ungrounded ask: the file exists under the diary root, is a citable
/// primary and supports the claim (its quote verbatim and relevant, or subject and object near).
fn source(ctx: &Ctx, t: &Target, file: &str) -> Result<String, Error> {
    if t.finding["evidence"]["status"] != "absent-source" {
        return Err(Refused::error(
            "source: answers an ungrounded ask (no source found)",
        ));
    }
    let diary = Diary::open(&ctx.root.join("diary"))?;
    let name = file.strip_suffix(".md").unwrap_or(file);
    let text = crate::auto_ground::citable(&diary, name)
        .map_err(|e| outside_refused(e, file))?
        .ok_or_else(|| {
        Refused::error(format!(
            "source: {file} is not a citable diary entry under {} (absent, a rollup, bookkeeping or an IMPROVE entry)",
            ctx.root.display()
        ))
    })?;
    let claim = claim_of(&t.claim);
    if !locates(&claim, &text) {
        return Err(Refused::error(format!(
            "source: {file} does not support the claim (no relevant verbatim quote, subject and object not within 600 characters)"
        )));
    }
    Ok(grounded_statement(&claim, file, &text))
}
