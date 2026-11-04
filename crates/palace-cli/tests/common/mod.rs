//! Shared invented identity map: the audience categories live in the file, in this order.
#![allow(dead_code)]
pub const PEOPLE: &str = r#"{
  "people": {
    "orin": {"handles": [], "owner": true},
    "mira": {"handles": []},
    "nella": {"handles": []},
    "torin": {"handles": []},
    "wren": {"handles": []}
  },
  "conversations": [],
  "private_tags": {
    "audience-elira-health": ["orin", "mira", "nella"],
    "audience-household": ["orin", "mira"],
    "audience-orin": ["orin"],
    "audience-nella": ["orin", "nella"],
    "audience-torin": ["orin", "torin"],
    "audience-wren": ["orin", "wren"],
    "audience-study": ["orin", "torin", "nella"]
  }
}
"#;
