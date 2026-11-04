//! Exhaustive finite audience property: restrictions cannot admit a formerly denied reader.
use palace_store::{
    Store,
    privacy::{self, Audience, LegacyRow, Marker, People},
};
use std::{collections::BTreeSet, fs, path::PathBuf};
#[test]
fn remarking_never_widens_any_existing_audience_or_quarantine() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
        "../../target/priv2/property-{}.sqlite",
        std::process::id()
    ));
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    if path.exists() {
        fs::remove_file(&path).unwrap();
    }
    let mut store = Store::open(&path).unwrap();
    let c = rusqlite::Connection::open(&path).unwrap();
    let members = ["ann", "wren", "cy"];
    let audience = |bits: usize| -> BTreeSet<String> {
        members
            .iter()
            .enumerate()
            .filter(|(i, _)| bits & (1 << i) != 0)
            .map(|(_, p)| (*p).to_owned())
            .collect()
    };
    for old_bits in 0..=9 {
        // Every subset, shared and quarantine.
        for new_bits in 0..8 {
            let id = format!("entry-{old_bits}-{new_bits}");
            let old = if old_bits == 8 {
                Marker::shared("original")
            } else if old_bits == 9 {
                Marker::quarantine("original")
            } else {
                Marker {
                    audience: audience(old_bits),
                    private: true,
                    ..Marker::shared("original")
                }
            };
            let restriction = Marker {
                audience: audience(new_bits),
                private: true,
                ..Marker::shared("restriction")
            };
            for kind in ["entry", "claim", "triple", "item", "fidelity", "ask"] {
                privacy::put(&c, kind, &id, &old).unwrap();
            }
            let plan: Vec<_> = ["entry", "claim", "triple", "item", "fidelity", "ask"]
                .into_iter()
                .map(|kind| LegacyRow {
                    kind: kind.into(),
                    id: id.clone(),
                    marker: restriction.clone(),
                    reason: "tag:audience-synthetic".into(),
                })
                .collect();
            store.apply_legacy_privacy_backfill(&plan).unwrap();
            for kind in ["entry", "claim", "triple", "item", "fidelity", "ask"] {
                for result in privacy::markers(&c, kind, &id).unwrap() {
                    assert!(result.private);
                    for readers in 0..8 {
                        let readers: Vec<_> = audience(readers).into_iter().collect();
                        assert!(!result.admits(&readers) || old.admits(&readers));
                        assert!(!result.admits(&readers) || restriction.admits(&readers));
                    }
                }
            }
            // Re-running is idempotent.
            store.apply_legacy_privacy_backfill(&plan).unwrap();
        }
    }
}
#[test]
fn audience_tags_intersect_each_other_and_existing_private_tags() {
    let p = People::parse(r#"{"people":{"ann":{"handles":[]},"wren":{"handles":[]},"cy":{"handles":[]}},"conversations":[],"private_tags":{"legacy":["wren"],"audience-one":["ann","wren"],"audience-two":["ann","cy"]}}"#).unwrap();
    let base = Audience::default().marker("synthetic");
    let tagged = |text| p.tagged(&base, &privacy::entry_tags(text)).0;
    assert_eq!(
        tagged("Tags: audience-one, audience-two.").audience,
        BTreeSet::from(["ann".to_owned()])
    );
    assert!(tagged("Tags: legacy, audience-two.").audience.is_empty());
    assert_eq!(
        tagged("Tags: legacy, audience-one.").audience,
        BTreeSet::from(["wren".to_owned()])
    );
}

#[test]
fn new_audience_tags_quarantine_when_no_identity_map_is_installed() {
    let marker = privacy::tagged_entry(&Marker::shared("synthetic"), "Tags: audience-household.");
    assert!(marker.private && marker.quarantine && marker.audience.is_empty());
    assert!(!privacy::tagged_entry(&Marker::shared("synthetic"), "Tags: telescope.").private);
}
