//! `palace improve --auto` (M4d item 4): SPEC §5.1 over every open claim finding, as revised
//! 2026-09-27 — mechanical kinds applied (`unsupported` with a source → `weaken`, `stale` →
//! `expire`), conflation and omission recorded as proposals, a reversal disputed (`weaken`, no
//! second decrement, the claim `disputed`, the finding left in Fergus's queue; never an ask), an
//! ungrounded claim at or above 0.40 given one grounding attempt (success `add --supersede`,
//! failure `weaken --grounding-failed`) and below 0.40 asked. Every event is `rule:auto`. Prints
//! `AUTO events=<n> asks=<n> skipped=<n> vetoed=<n>`, one line per event, the ask lines (SPEC §6),
//! then queue entries, proposals, vetoes and skips — also when a candidate fails, before the
//! error (M4d-2e item 2); the wrapper still delivers asks from `--unsent`. `--dry-run` prints
//! the plan and writes nothing. Prints, never sends. A finding the watch saw again that repeats a
//! standing `rule:auto` event on an unchanged claim is `SKIP <date> Cn <token> repeat of e_…`
//! (M5b-5 item 1): no event, no diary entry, nothing for the digest.
use crate::auto_ground::{Found, ground};
use crate::improve::{Call, Ctx, Options, apply_call, recover};
use palace_core::{
    auto::{self, Budget, Facts, Grounded, Plan},
    improve::{Verb, coinage},
};
use palace_store::{
    Error, Refused, Skipped,
    auto::{Ask, Candidate, candidates},
    claims::ClaimStore,
    repeat,
};
use serde_json::{Value, json};

pub fn run(o: &Options) -> i32 {
    match looped(o) {
        Ok(()) => 0,
        Err(e) => crate::improve::fail("palace improve --auto", e),
    }
}

/// What the run did (or, dry, would do), in print order.
#[derive(Default)]
struct Report {
    events: Vec<String>,
    asks: Vec<String>,
    others: Vec<String>,
    skipped: usize,
    vetoed: usize,
}

struct Runner<'a> {
    ctx: Ctx<'a>,
    dry: bool,
    judge: Option<crate::auto_ground::BoxedJudge>,
    budget: Budget,
    report: Report,
}

fn looped(o: &Options) -> Result<(), Error> {
    if o.by.as_deref().is_some_and(|by| by != auto::BY) {
        return Err(Refused::error(format!("--auto acts as {} only", auto::BY)));
    }
    let ctx = Ctx::of(o)?;
    if !o.dry_run {
        recover(&ctx)?;
    }
    let kg = crate::paths::input(&ctx.paths.kg)?;
    let found = candidates(&ctx.compiler()?, &kg)?;
    let repeated = palace_store::repeat::repeats(&ctx.compiler()?, &kg)?;
    let mut runner = Runner {
        judge: o
            .judge
            .as_deref()
            .map(crate::auto_ground::judge)
            .transpose()?,
        ctx,
        dry: o.dry_run,
        budget: Budget::new(o.max_verbs),
        report: Report::default(),
    };
    repeated.iter().for_each(|r| runner.repeat(r));
    // Review M-1: what the run did is printed even when a later candidate fails.
    let done = found.iter().try_for_each(|candidate| runner.one(candidate));
    runner.print();
    done
}

impl Runner<'_> {
    fn one(&mut self, c: &Candidate) -> Result<(), Error> {
        let item = Item::of(c);
        let own = ungrounded(c) && crate::auto_ground::own_source_found(&self.ctx, &c.claim)?;
        match auto::plan(&facts(c, own)) {
            Plan::Apply(verb) => self.apply(&item, verb),
            Plan::Dispute => self.dispute(&item),
            Plan::Ground => self.ground(&item),
            Plan::Ask(kind) => self.ask(&item, kind),
            Plan::Propose(verb) => self.propose(&item, verb),
            Plan::Skip(why) => {
                self.skip(&item, why);
                Ok(())
            }
        }
    }

    fn apply(&mut self, item: &Item, verb: Verb) -> Result<(), Error> {
        let kg = crate::paths::input(&self.ctx.paths.kg)?;
        let (compiler, ids) = (self.ctx.compiler()?, (item.claim_id(), item.finding_id()));
        let confidence = palace_store::auto::confidence_without(&compiler, &kg, ids.0, ids.1)?;
        if auto::needs_second_check(confidence)
            && !crate::auto_ground::second_check(&self.ctx, item.finding, item.claim)?
        {
            self.skip(item, "second check disagrees");
            return Ok(());
        }
        if !self.budget.admit(verb) {
            self.skip(item, "rate limit");
            return Ok(());
        }
        let text = (verb == Verb::Coin).then(|| item.proposal_text());
        let call = self.call(verb.name(), item.finding_id(), text.as_deref());
        self.event(item, &call, "")
    }

    /// A reversal: `weaken` with the dispute reason (the store marks the claim `disputed` and
    /// leaves the finding open), then a queue line for Fergus.
    fn dispute(&mut self, item: &Item) -> Result<(), Error> {
        let call = self.call("weaken", item.finding_id(), Some(auto::DISPUTE_REASON));
        let reason = format!(" reason=\"{}\"", auto::DISPUTE_REASON);
        self.event(item, &call, &reason)?;
        let assistant = palace_core::names::assistant();
        let line = format!(
            "QUEUE {assistant} {} {} reversal",
            item.label(),
            item.token()
        );
        self.report.others.push(line);
        Ok(())
    }

    /// One grounding attempt; `after_ground` turns its result into add, veto, weaken or nothing.
    fn ground(&mut self, item: &Item) -> Result<(), Error> {
        let found = ground(&self.ctx, self.judge.as_deref(), item.claim)?;
        let exists = found.as_ref().is_some_and(|f| f.exists);
        match (
            auto::after_ground(found.is_some(), exists, item.person_acted),
            found,
        ) {
            (Grounded::Add, Some(found)) => self.grounded(item, &found),
            (Grounded::Veto, Some(found)) => self.veto(item, &found),
            (Grounded::Skip, _) => {
                self.skip(item, "people first");
                Ok(())
            }
            _ => {
                let mut call = self.call("weaken", item.finding_id(), None);
                call.grounding_failed = true;
                self.event(item, &call, " reason=\"grounding attempt failed\"")
            }
        }
    }

    /// A found source: `add --supersede` citing it (a refusal as identical is a veto).
    fn grounded(&mut self, item: &Item, found: &Found) -> Result<(), Error> {
        if !self.budget.admit(Verb::Add) {
            self.skip(item, "rate limit");
            return Ok(());
        }
        let mut call = self.call("add", item.finding_id(), Some(&found.statement));
        call.supersede = true;
        let suffix = format!(" found_source={}", found.source);
        match self.event(item, &call, &suffix) {
            Err(e) if Refused::is(&e) && e.to_string().contains("already exists") => {
                self.veto(item, found)
            }
            other => other,
        }
    }

    fn veto(&mut self, item: &Item, found: &Found) -> Result<(), Error> {
        let detail = json!({"verb":"add","statement":found.statement,"source":found.source});
        if !self.dry {
            ClaimStore::open(&self.ctx.compiler()?)?.mark(
                item.finding_id(),
                "vetoed",
                item.claim_id(),
                &detail,
            )?;
        }
        self.report.vetoed += 1;
        let line = format!(
            "VETOED add {} {}: identical add refused before (operator undo); not retried",
            item.label(),
            item.token()
        );
        self.report.others.push(line);
        Ok(())
    }

    fn call<'c>(&self, verb: &'c str, target: &'c str, text: Option<&'c str>) -> Call<'c> {
        Call {
            verb,
            target,
            text,
            by: auto::BY,
            supersede: false,
            grounding_failed: false,
        }
    }

    /// Applies `call` (or, dry, plans it) and reports its event line.
    fn event(&mut self, item: &Item, call: &Call, suffix: &str) -> Result<(), Error> {
        if self.dry {
            let line = format!(
                "PLAN {} {} {}{suffix}",
                call.verb,
                item.label(),
                item.token()
            );
            self.report.events.push(line);
            return Ok(());
        }
        match apply_call(&self.ctx, call) {
            Ok(done) => {
                let after = self.applied(item, &done.applied.event)?;
                let line = event_line(item, &done.applied, &after, suffix);
                self.report.events.push(line);
                Ok(())
            }
            Err(e) if Skipped::is(&e) => {
                self.skip(item, &e.to_string());
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    /// The claim right after `event`, recorded on the finding (M5b-5 item 1): a recurrence on the
    /// unchanged claim is then a repeat, not a new finding.
    fn applied(&self, item: &Item, event: &str) -> Result<Value, Error> {
        let mut store = ClaimStore::open(&self.ctx.compiler()?)?;
        let after = store.claim(item.claim_id())?.unwrap_or(Value::Null);
        let (mark, detail) = (repeat::mark_name(event), repeat::detail(event, &after));
        store.mark(item.finding_id(), &mark, item.claim_id(), &detail)?;
        Ok(after)
    }

    /// A finding the watch saw again that repeats a standing event: nothing to apply.
    fn repeat(&mut self, r: &repeat::Repeat) {
        let item = Item {
            finding: &r.finding,
            claim: &r.claim,
            person_acted: false,
        };
        self.report.skipped += 1;
        let line = format!(
            "SKIP {} {} repeat of {}",
            item.label(),
            item.token(),
            r.event
        );
        self.report.others.push(line);
    }

    /// One SPEC §6 line, recorded in `asks` so it is never repeated while open.
    fn ask(&mut self, item: &Item, kind: &str) -> Result<(), Error> {
        let line = crate::findings::signal_for(item.finding, item.claim, None);
        if !self.dry {
            let ask = Ask {
                finding: item.finding_id(),
                claim: item.claim_id(),
                kind,
                token: &item.token(),
                line: &line,
                proposal: None,
            };
            let mut store = ClaimStore::open(&self.ctx.compiler()?)?;
            let at = store.now()?;
            if !store.record_ask(&ask, &at)? {
                self.skip(item, "ask open");
                return Ok(());
            }
        }
        self.report.asks.push(format!("ASK {kind} {line}"));
        Ok(())
    }

    /// Conflation and omission (and an uncoinable vocabulary gap): the proposal is recorded on
    /// the finding once, never applied.
    fn propose(&mut self, item: &Item, verb: &str) -> Result<(), Error> {
        let proposal = &item.finding["proposal"];
        let new = self.dry
            || ClaimStore::open(&self.ctx.compiler()?)?.mark(
                item.finding_id(),
                "proposal",
                item.claim_id(),
                proposal,
            )?;
        self.report.skipped += 1;
        if new {
            let text = proposal["text"].as_str().unwrap_or_default();
            let line = format!("PROPOSAL {verb} {} {}: {text}", item.label(), item.token());
            self.report.others.push(line);
        }
        Ok(())
    }

    fn skip(&mut self, item: &Item, why: &str) {
        self.report.skipped += 1;
        let kind = item.finding["kind"].as_str().unwrap_or_default();
        let line = format!("SKIP {} {} {kind}: {why}", item.label(), item.token());
        self.report.others.push(line);
    }

    fn print(&self) {
        let r = &self.report;
        let head = if self.dry { "AUTO DRY-RUN" } else { "AUTO" };
        println!(
            "{head} events={} asks={} skipped={} vetoed={}",
            r.events.len(),
            r.asks.len(),
            r.skipped,
            r.vetoed
        );
        for line in r.events.iter().chain(&r.asks).chain(&r.others) {
            println!("{line}");
        }
    }
}

/// `EVENT <verb> <date> Cn <claim> event=… by=rule:auto axis=… confidence_after=… status_after=… to_claim=…`.
fn event_line(
    item: &Item,
    a: &palace_store::graph::Applied,
    after: &Value,
    suffix: &str,
) -> String {
    format!(
        "EVENT {} {} {} event={} by={} axis={} confidence_after={:.2} status_after={} to_claim={}{suffix}",
        a.verb,
        item.label(),
        item.claim_id(),
        a.event,
        a.by,
        a.axis.join(","),
        after["confidence"].as_f64().unwrap_or_default(),
        after["status"].as_str().unwrap_or("?"),
        a.to_claim.as_deref().unwrap_or("-"),
    )
}

/// An `unsupported` finding whose source was absent at the watch.
fn ungrounded(c: &Candidate) -> bool {
    c.finding["kind"] == "unsupported" && c.finding["evidence"]["status"] == "absent-source"
}

fn facts(c: &Candidate, own_source_found: bool) -> Facts<'_> {
    let f = &c.finding;
    let status = c.claim["status"].as_str().unwrap_or_default();
    Facts {
        kind: f["kind"].as_str().unwrap_or_default(),
        claim_live: matches!(status, "live" | "disputed"),
        disputed: status == "disputed",
        confidence: c.claim["confidence"].as_f64().unwrap_or_default(),
        source_present: f["evidence"]["status"] != "absent-source",
        own_source_found,
        asked: c.asked,
        vetoed: c.vetoed,
        person_acted: c.person_acted,
        coinable: coinage(f["proposal"]["text"].as_str().unwrap_or_default()).is_ok(),
    }
}

/// One candidate's finding and claim, with the labels its lines use.
struct Item<'a> {
    finding: &'a Value,
    claim: &'a Value,
    person_acted: bool,
}

impl<'a> Item<'a> {
    fn of(c: &'a Candidate) -> Self {
        Self {
            finding: &c.finding,
            claim: &c.claim,
            person_acted: c.person_acted,
        }
    }
    fn finding_id(&self) -> &'a str {
        self.finding["id"].as_str().unwrap_or_default()
    }
    fn claim_id(&self) -> &'a str {
        self.claim["id"].as_str().unwrap_or_default()
    }
    fn token(&self) -> String {
        palace_core::finding::reply_token(self.finding_id())
    }
    fn proposal_text(&self) -> String {
        self.finding["proposal"]["text"]
            .as_str()
            .unwrap_or_default()
            .to_owned()
    }
    /// `<rollup date> Cn` where the finding's rollup lists the claim.
    fn label(&self) -> String {
        let listed = self.claim["numbers"].as_array().and_then(|n| {
            n.iter()
                .find(|n| n["rollup"] == self.finding["rollup"])
                .or(n.first())
        });
        let field = |k: &str| listed.and_then(|l| l[k].as_str()).unwrap_or("?").to_owned();
        format!("{} {}", field("date"), field("number"))
    }
}
