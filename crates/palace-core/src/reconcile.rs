//! Crash recovery across compiler.sqlite and the KG (M4d-2a.1 item 2, review S-1), the pure
//! part. Both files are WAL-mode, so a commit spanning them is atomic per file only. Every
//! KG-touching commit first records an *intent* (per row: table, key, column, `before`, `after`)
//! in compiler.sqlite alone; the cross-file commit marks it done beside its event. An intent
//! never settled is reconciled row by row with this table.

/// What recovery does with one row of an unsettled intent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// The row already agrees with the event's fate.
    Keep,
    /// The event committed but the KG half was lost: write `after` again.
    Reapply,
    /// The KG half committed without its event: write `before` back (the event never happened).
    Restore,
    /// The row is neither `before` nor `after`: someone else changed it; touch nothing.
    Diverged,
}

/// The decision table: `event_present` = the intent was marked done (same compiler transaction
/// as the event); `at_before` / `at_after` = the KG row's current value equals that side.
pub fn action(event_present: bool, at_before: bool, at_after: bool) -> Action {
    match (event_present, at_before, at_after) {
        (true, _, true) | (false, true, _) => Action::Keep,
        (true, true, false) => Action::Reapply,
        (false, false, true) => Action::Restore,
        (_, false, false) => Action::Diverged,
    }
}

/// The one way out of a diverged intent (M4d-2c item 6, review S-5): an operator's audited choice.
pub fn settle_hint(intent: &str) -> String {
    let (person, assistant) = (crate::names::person(), crate::names::assistant());
    format!(
        "settle it by hand: palace doctor --settle {intent} --keep-current|--restore-before --by {person}|{assistant} --kg <kg>"
    )
}

/// How a whole intent settles, given its rows' actions (no `Diverged` among them).
pub fn outcome(event_present: bool, actions: &[Action]) -> &'static str {
    let acted = actions.iter().any(|a| *a != Action::Keep);
    match (event_present, acted) {
        (true, false) => "committed",
        (true, true) => "recovered_reapply",
        (false, true) => "recovered_rollback",
        (false, false) => "dropped",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn the_recovery_table() {
        assert_eq!(action(true, false, true), Action::Keep);
        assert_eq!(action(true, true, false), Action::Reapply);
        assert_eq!(action(false, false, true), Action::Restore);
        assert_eq!(action(false, true, false), Action::Keep);
        assert_eq!(action(true, false, false), Action::Diverged);
        assert_eq!(action(false, false, false), Action::Diverged);
        assert_eq!(action(true, true, true), Action::Keep, "no-op row");
        assert_eq!(outcome(true, &[Action::Keep]), "committed");
        assert_eq!(
            outcome(true, &[Action::Keep, Action::Reapply]),
            "recovered_reapply"
        );
        assert_eq!(outcome(false, &[Action::Restore]), "recovered_rollback");
        assert_eq!(outcome(false, &[Action::Keep]), "dropped");
    }
}
