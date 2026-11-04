//! Prompt policy files: generic in the repository, the owner's wording local. The audience table
//! is drawn from the people map at run time; `<prompts>/local/<name>.md`, when present, is appended.
use palace_store::{Error, privacy::People};
use std::{fs, path::Path};

const TABLE_PLACEHOLDER: &str = "{AUDIENCE_TABLE}";

/// `<prompts>/<name>` with the audience table filled in and any local policy appended.
pub fn load(prompts: &Path, name: &str, people: Option<&People>) -> Result<String, Error> {
    let text = fs::read_to_string(crate::paths::input(&prompts.join(name))?)?;
    let text = text.replace(TABLE_PLACEHOLDER, &audience_table(people));
    Ok(match local_policy(prompts, name)? {
        Some(local) => format!("{}\n\nLocal policy\n\n{}\n", text.trim_end(), local.trim()),
        None => text,
    })
}

fn local_policy(prompts: &Path, name: &str) -> Result<Option<String>, Error> {
    let path = prompts.join("local").join(name);
    if !path.exists() {
        return Ok(None);
    }
    Ok(Some(fs::read_to_string(crate::paths::input(&path)?)?))
}

/// One row per approved audience category, in the people map's order, then `shared`.
pub fn audience_table(people: Option<&People>) -> String {
    let categories = people.map(People::audience_categories).unwrap_or_default();
    let mut rows = vec!["| Category | Audience |".to_owned(), "|---|---|".to_owned()];
    for (tag, members) in categories {
        rows.push(format!("| {tag} | {} |", members.join(", ")));
    }
    rows.push("| shared | everyone |".to_owned());
    rows.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn without_categories_only_shared_is_offered() {
        let bare = r#"{"conversations":[],"people":{"wren":{"handles":[]}}}"#;
        let people = People::parse(bare).unwrap();
        let expected = "| Category | Audience |\n|---|---|\n| shared | everyone |";
        assert_eq!(audience_table(Some(&people)), expected);
        assert_eq!(audience_table(None), expected);
    }

    #[test]
    fn rows_follow_the_map_in_file_order() {
        let text = r#"{"conversations":[],"people":{"orin":{"handles":[]},"mira":{"handles":[]}},
            "private_tags":{"audience-b":["orin","mira"],"wren-private":["orin"],"audience-a":["orin"]}}"#;
        let table = audience_table(Some(&People::parse(text).unwrap()));
        let rows: Vec<_> = table.lines().skip(2).collect();
        assert_eq!(
            rows,
            [
                "| audience-b | orin, mira |",
                "| audience-a | orin |",
                "| shared | everyone |"
            ]
        );
    }
}
