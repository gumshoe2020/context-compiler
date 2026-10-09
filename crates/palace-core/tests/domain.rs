use palace_core::ids::*;
#[test]
fn identity_uses_unambiguous_unicode_length_prefixes() {
    assert_eq!(length_prefixed(&["é🦀", "12"]), "2:é🦀2:12");
    assert_ne!(length_prefixed(&["ab", "c"]), length_prefixed(&["a", "bc"]));
    assert_eq!(entity_id("Callum's Bank"), "callums_bank");
    assert_eq!(
        drawer_id("openclaw", "general", "x", 0),
        format!("drawer_openclaw_general_{}", &sha256(b"1:x1:0")[..24])
    );
    assert_eq!(
        triple_id("a", "p", "b", "x", "y"),
        format!("t_a_p_b_{}", &sha256(b"1:x1:y")[..12])
    );
}
