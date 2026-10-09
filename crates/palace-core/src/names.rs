//! PKG-1.2 item 4 (review L6): the two people the loop serves, by role. The person corrects the
//! memory and replies to its asks; the assistant keeps the queue of reversals and escalates. Each
//! is a `--by` name in the claim store. `palace --person <name> --assistant <name>` sets them once
//! per process; without them a build with owner names (`OWNER`) uses those, as before.
use std::sync::OnceLock;

/// PUB-1a item 2: the owner's names, compiled in by build.rs (from $PALACE_OWNER_PERSON and
/// $PALACE_OWNER_ASSISTANT, else the private tree's dev/owner-defaults), or `None`: no source
/// file names an owner. palace-core's own unit tests run with the invented cast as the owner.
pub const OWNER: Option<(&str, &str)> = if cfg!(test) {
    Some(("callum", "fergus"))
} else {
    match (
        option_env!("PALACE_OWNER_PERSON"),
        option_env!("PALACE_OWNER_ASSISTANT"),
    ) {
        (Some(person), Some(assistant)) => Some((person, assistant)),
        _ => None,
    }
};
/// The names used when none is set: the owner's, else the roles themselves (only `context` and
/// `init` run without names in a build without owner names; they record no role).
pub const DEFAULT_PERSON: &str = match OWNER {
    Some((person, _)) => person,
    None => "person",
};
pub const DEFAULT_ASSISTANT: &str = match OWNER {
    Some((_, assistant)) => assistant,
    None => "assistant",
};

pub struct Names {
    pub person: String,
    pub assistant: String,
}

static NAMES: OnceLock<Names> = OnceLock::new();

/// Sets the names for this process (once, before any is read); `None` keeps a default.
pub fn set(person: Option<&str>, assistant: Option<&str>) -> Result<(), String> {
    let names = Names {
        person: valid(person.unwrap_or(DEFAULT_PERSON))?,
        assistant: valid(assistant.unwrap_or(DEFAULT_ASSISTANT))?,
    };
    if names.person == names.assistant {
        return Err(format!(
            "the person and the assistant are both {}",
            names.person
        ));
    }
    NAMES
        .set(names)
        .map_err(|_| "the names are already set".to_owned())
}

/// A `--by` name: a lowercase letter, then lowercase letters, digits, `_` or `-` (at most 32).
fn valid(name: &str) -> Result<String, String> {
    let mut chars = name.chars();
    let first = chars.next().is_some_and(|c| c.is_ascii_lowercase());
    let rest = chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "_-".contains(c));
    if !first || !rest || name.len() > 32 {
        return Err(format!(
            "name {name:?}: a lowercase letter, then lowercase letters, digits, _ or -"
        ));
    }
    Ok(name.to_owned())
}

fn names() -> &'static Names {
    NAMES.get_or_init(|| Names {
        person: DEFAULT_PERSON.to_owned(),
        assistant: DEFAULT_ASSISTANT.to_owned(),
    })
}

pub fn person() -> &'static str {
    &names().person
}

pub fn assistant() -> &'static str {
    &names().assistant
}

/// `by` is the person or the assistant (not a rule or a judge).
pub fn is_person(by: &str) -> bool {
    by == person() || by == assistant()
}

/// "<person> or <assistant>", for messages.
pub fn either() -> String {
    format!("{} or {}", person(), assistant())
}

/// A name as prose writes it: the first letter in capitals.
pub fn display(name: &str) -> String {
    let mut chars = name.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}

/// `text` (a prompt) with this process's names: `{Person}` and `{Assistant}` as prose writes
/// them, `{assistant}` as the `--by` name (PKG-1.3 item 1: the rollup prompt names no one), and
/// the owner's names (`OWNER`), as prose writes them, replaced; unchanged under the defaults when
/// it holds no placeholder.
pub fn in_prose(text: &str) -> String {
    let mut text = text
        .replace("{Person}", &display(person()))
        .replace("{Assistant}", &display(assistant()))
        .replace("{assistant}", assistant());
    let Some((owner_person, owner_assistant)) = OWNER else {
        return text;
    };
    let swaps = [(owner_person, person()), (owner_assistant, assistant())];
    for (owner, name) in swaps.iter().filter(|(owner, name)| owner != name) {
        text = text.replace(&display(owner), &display(name));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn defaults_are_the_owners() {
        assert_eq!(OWNER, Some((DEFAULT_PERSON, DEFAULT_ASSISTANT)));
        assert_eq!((person(), assistant()), (DEFAULT_PERSON, DEFAULT_ASSISTANT));
        assert!(is_person("callum") && is_person("fergus") && !is_person("rule:auto"));
        assert_eq!(either(), "callum or fergus");
        let prompt = "Designed with Callum; Fergus proposes.";
        assert_eq!(in_prose(prompt), prompt);
        let placeholders = "for {Person}; {Assistant} acts; \"supplied_by\": \"{assistant}\"";
        assert_eq!(
            in_prose(placeholders),
            "for Callum; Fergus acts; \"supplied_by\": \"fergus\""
        );
    }
    #[test]
    fn names_are_checked() {
        for good in ["maria", "j-smith", "a1"] {
            assert!(valid(good).is_ok(), "{good}");
        }
        for bad in ["", "Maria", "1a", "rule:x", "a b", &"a".repeat(33)] {
            assert!(valid(bad).is_err(), "{bad}");
        }
        assert_eq!(display("maria"), "Maria");
    }
}
