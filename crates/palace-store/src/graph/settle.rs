//! `palace doctor --settle` (M4d-2c item 6, review S-5): the one exit from a DIVERGED intent. A
//! person chooses: keep the KG rows as they are now, or restore every row's `before`. Either way
//! the intent is settled `recovered_manual` in the same guarded transaction that writes an
//! audited `recovered_manual` line (intent, rows with their value now, choice, by, at) to the
//! compiler's hash-chained write log. Only a diverged intent is settled by hand; any other is the
//! reconciling commands' job.
use super::{Graph, intent, reconcile};
use crate::{Error, Refused, write_log};
use palace_core::reconcile::Action;
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Choice {
    KeepCurrent,
    RestoreBefore,
}

impl Choice {
    fn name(self) -> &'static str {
        match self {
            Self::KeepCurrent => "keep-current",
            Self::RestoreBefore => "restore-before",
        }
    }
}

impl Graph {
    /// Settles the diverged intent `id` by `by`'s `choice`; returns the logged line.
    pub fn settle(&mut self, id: &str, choice: Choice, by: &str) -> Result<Value, Error> {
        let judged = self.diverged_intent(id, by)?;
        let (at, day) = (self.now()?, super::day(&self.store.connection)?);
        let rows = self.rows_now(&judged)?;
        let line = json!({"operation":"recovered_manual","intent":id,"of":judged.operation,"choice":choice.name(),"by":by,"rows":rows,"at":at});
        let settles = Some((id, "recovered_manual"));
        intent::guarded(
            &mut self.store.connection,
            ("recovered_manual", &at),
            settles,
            |tx| {
                if choice == Choice::RestoreBefore {
                    for (row, _) in judged.actions.iter().rev() {
                        reconcile::write(tx, row, Action::Restore, &day)?;
                    }
                }
                write_log::append(tx, &line.to_string())
            },
        )?;
        self.flush_wal()?;
        Ok(line)
    }

    /// The unsettled, diverged intent `id`, when `by` is a person.
    fn diverged_intent(&self, id: &str, by: &str) -> Result<reconcile::Judged, Error> {
        if !palace_core::names::is_person(by) {
            let (person, assistant) = (
                palace_core::names::person(),
                palace_core::names::assistant(),
            );
            return Err(Refused::error(format!(
                "settling is {person}'s or {assistant}'s (--by)"
            )));
        }
        let judged = reconcile::judge(&self.store.connection)?
            .into_iter()
            .find(|j| j.id == id)
            .ok_or_else(|| Refused::error(format!("{id} is not an unsettled intent")))?;
        if judged.diverged().is_empty() {
            return Err(Refused::error(format!(
                "{id} is not diverged: a reconciling command settles it (see palace doctor --check-divergence)"
            )));
        }
        Ok(judged)
    }

    /// The intent's rows, each with the KG's value now (`now`), for the audit line.
    fn rows_now(&self, judged: &reconcile::Judged) -> Result<Vec<Value>, Error> {
        judged
            .actions
            .iter()
            .map(|(row, _)| {
                let mut row = row.clone();
                row["now"] = reconcile::current(&self.store.connection, &row)?;
                Ok(row)
            })
            .collect()
    }
}
