//! Synthetic distill events; invented people only, no actual conversations.
use palace_store::privacy::{self, Identity, People};
use rusqlite::Connection;
use serde_json::json;
#[test]
fn distill_events_apply_audience_mappings_and_fail_closed_if_a_mapping_is_missing() {
    let people = People::parse(r#"{"people":{"orin":{"handles":[]},"mira":{"handles":[]},"nella":{"handles":[]}},"conversations":[{"match":"synthetic-session","people":["orin"]}],"private_tags":{"audience-household":["orin","mira"],"audience-elira-health":["orin","mira","nella"]}}"#).unwrap();
    privacy::configure(Identity {
        people: Some(people),
        conversation: None,
        local_operator: true,
    })
    .unwrap();
    let c = Connection::open_in_memory().unwrap();
    c.execute_batch(privacy::SCHEMA).unwrap();
    for (file, tags, expected) in [
        (
            "synthetic-family.md",
            json!(["observatory", "audience-household"]),
            vec!["mira", "orin"],
        ),
        (
            "synthetic-health.md",
            json!(["observatory", "audience-elira-health"]),
            vec!["mira", "nella", "orin"],
        ),
        ("synthetic-shared.md", json!(["observatory"]), vec![]),
        ("synthetic-unmapped.md", json!(["audience-torin"]), vec![]),
    ] {
        privacy::distill_event(&c,&json!({"session":"synthetic-session","wing":"synthetic","files":[file],"reply":{"entries":[{"title":"Invented","body":"Orin Vale planned a cobalt observatory.","tags":tags}]}})).unwrap();
        let markers = privacy::markers(&c, "entry", file).unwrap();
        assert!(!markers.is_empty());
        for marker in markers {
            assert_eq!(marker.audience.into_iter().collect::<Vec<_>>(), expected);
            assert_eq!(marker.private, file != "synthetic-shared.md");
            assert_eq!(marker.quarantine, file == "synthetic-unmapped.md");
        }
    }
}
