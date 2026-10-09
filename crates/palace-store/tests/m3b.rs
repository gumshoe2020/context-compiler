use palace_store::{graph::Graph, m3b_checks};
fn run(case: fn(&mut Graph) -> Result<(), palace_store::Error>) {
    let dir = tempfile::tempdir().unwrap();
    let kg = dir.path().join("kg.sqlite");
    m3b_checks::empty_fixture(&kg).unwrap();
    let mut graph = Graph::open(&dir.path().join("palace.sqlite"), &kg, None).unwrap();
    case(&mut graph).unwrap();
}
#[test]
fn retire_refuses_live() {
    run(m3b_checks::refuses_live);
}
#[test]
fn retire_orphan_idempotent() {
    run(m3b_checks::orphan);
}
#[test]
fn retire_five_fixtures() {
    run(m3b_checks::fixtures);
}
#[test]
fn retire_continues_after_refusal_and_missing() {
    run(m3b_checks::mixed);
}
#[test]
fn in_process_graph_roundtrip() {
    run(m3b_checks::roundtrip);
}
