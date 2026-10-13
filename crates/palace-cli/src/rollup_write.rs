//! `palace rollup` stages 4–5 (M5b-10 item 2): the first write, then, while every problem is a
//! claim line's, a repair of those lines (at most two; the rest of the body kept byte for byte),
//! otherwise one rewrite of the whole; at most three attempts, then exit 4.
use crate::rollup::Run;
use palace_core::{
    rollup_cluster::Cluster,
    rollup_repair::{self, Mode},
    rollup_validate::{self, Target, Validation},
};
use palace_mempalace::Evidence;
use palace_store::Error;
use serde_json::Value;

/// One attempt: the body it validated, the validation and its report, how it was made.
pub(crate) struct Attempt {
    pub body: String,
    pub validation: Validation,
    pub report: Value,
    pub mode: Mode,
    pub repaired: usize,
}
/// The rollup's inputs to every attempt.
pub(crate) struct Job<'a> {
    pub target: &'a Target<'a>,
    pub cluster: &'a Cluster,
    pub evidence: &'a Evidence,
}

impl Run {
    /// Every attempt in order; the last one passed, or failed with no attempt left.
    pub(crate) fn write_validated(&self, job: &Job) -> Result<Vec<Attempt>, Error> {
        let (mut attempts, mut mode) = (Vec::<Attempt>::new(), Mode::Write);
        loop {
            let attempt = self.attempt(job, mode, attempts.last())?;
            report_attempt(attempts.len() + 1, &attempt);
            let repairable = repair_problems(&attempt).is_some();
            let passed = attempt.validation.passed();
            attempts.push(attempt);
            let count = |m: Mode| attempts.iter().filter(|a| a.mode == m).count();
            let next = rollup_repair::next_mode(
                repairable,
                attempts.len(),
                count(Mode::Repair),
                count(Mode::Rewrite),
            );
            match next {
                Some(next) if !passed => mode = next,
                _ => return Ok(attempts),
            }
        }
    }
    fn attempt(&self, job: &Job, mode: Mode, previous: Option<&Attempt>) -> Result<Attempt, Error> {
        let (Mode::Repair, Some(previous)) = (mode, previous) else {
            let retry = previous
                .map(|p| serde_json::to_string_pretty(&p.report))
                .transpose()?;
            let mut prompt = rollup_validate::write_prompt(
                &self.policy,
                job.target,
                job.cluster,
                retry.as_deref(),
            );
            prompt.push_str(&rollup_validate::specifics_section(
                &self.live_specifics(job.cluster)?,
            ));
            let body = crate::rollup::normalise(&self.models.write.complete(&prompt)?);
            let (body, mends) = rollup_repair::mend_layout(&body, &self.diary_names()?);
            for mend in mends {
                eprintln!("rollup: layout mended: {mend}");
            }
            return self.validated(job, body, mode, 0);
        };
        self.repair(job, previous)
    }
    /// Asks for the failed claim lines only and splices the reply into the previous body; a reply
    /// that breaks a line is a refused repair (the previous body, with the refusal as problems).
    fn repair(&self, job: &Job, previous: &Attempt) -> Result<Attempt, Error> {
        let problems = repair_problems(previous).ok_or("repair without claim problems")?;
        let refused: Vec<String> = previous
            .validation
            .problems
            .iter()
            .filter(|p| rollup_repair::claim_of(p).is_none())
            .cloned()
            .collect();
        let prompt = rollup_repair::prompt(&self.policy, &problems, job.cluster, &refused);
        let reply = self.models.repair.complete(&prompt)?;
        let asked: Vec<String> = problems.iter().map(|p| p.number.clone()).collect();
        match rollup_repair::splice(&previous.body, &reply, &asked) {
            Ok((body, repaired)) => self.validated(job, body, Mode::Repair, repaired),
            Err(refusals) => Ok(refused_attempt(previous, refusals)),
        }
    }
    /// Validates `body`; a cited live specific's missing value is mended (SETTLE-1h §1, no model
    /// call), counted as repaired, and the mended body validated again in full.
    fn validated(
        &self,
        job: &Job,
        body: String,
        mode: Mode,
        repaired: usize,
    ) -> Result<Attempt, Error> {
        let (validation, report) = self.validate(&body, job.target, job.cluster, job.evidence)?;
        let mut attempt = Attempt {
            body,
            validation,
            report,
            mode,
            repaired,
        };
        let missing = |p: &String| rollup_validate::missing_value_of(p).is_some();
        if attempt.validation.problems.iter().any(missing) {
            self.mend_cited_values(job, &mut attempt)?;
        }
        Ok(attempt)
    }
    fn mend_cited_values(&self, job: &Job, attempt: &mut Attempt) -> Result<(), Error> {
        let specifics = self.live_specifics(job.cluster)?;
        let (body, mends) = rollup_repair::mend_cited_values(
            &attempt.body,
            &attempt.validation.problems,
            &specifics,
        );
        if mends.is_empty() {
            return Ok(());
        }
        for mend in &mends {
            eprintln!("rollup: cited value mended: line {} {}", mend.line, mend.id);
        }
        (attempt.validation, attempt.report) =
            self.validate(&body, job.target, job.cluster, job.evidence)?;
        attempt.body = body;
        attempt.repaired += mends.len();
        Ok(())
    }
}

/// The claim-line problems of an attempt when they are all it has.
fn repair_problems(attempt: &Attempt) -> Option<Vec<rollup_repair::ClaimProblem>> {
    rollup_repair::claim_problems(&attempt.body, &attempt.validation.problems)
}

/// A refused repair: the previous body stands, with its claim problems and the refusals.
fn refused_attempt(previous: &Attempt, refusals: Vec<String>) -> Attempt {
    let mut problems: Vec<String> = previous
        .validation
        .problems
        .iter()
        .filter(|p| rollup_repair::claim_of(p).is_some())
        .cloned()
        .collect();
    problems.extend(refusals);
    let mut report = previous.report.clone();
    report["passed"] = false.into();
    report["problems"] = problems.clone().into();
    Attempt {
        body: previous.body.clone(),
        validation: Validation {
            problems,
            claims: previous.validation.claims.clone(),
            relations: previous.validation.relations.clone(),
            superseded: previous.validation.superseded.clone(),
        },
        report,
        mode: Mode::Repair,
        repaired: 0,
    }
}

fn report_attempt(number: usize, attempt: &Attempt) {
    let v = &attempt.validation;
    eprintln!(
        "rollup: validate attempt={number} {} claims={} relations={} mode={} repaired={}",
        if v.passed() { "pass" } else { "fail" },
        v.claims.len(),
        v.relations.len(),
        attempt.mode.as_str(),
        attempt.repaired
    );
}
