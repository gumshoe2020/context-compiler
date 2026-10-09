# Dependencies (PLAN v3, from M3c)

Only PLAN.md's named list is used. No crate was added for palace-mempalace: `std::process`,
`std::sync::mpsc` and `std::thread` run and time out the `mempalace-mcp` / `mempalace` children.
The runtime MemPalace dependency is not a crate: it is `mempalace==3.5.0` (uv tool), checked by
`mempalace --version` and MCP initialize.serverInfo.version at start-up and by `scripts/mempalace-check.sh`.

| Direct dependency | Used by | Reason | Pulls in (normal + build, incl. itself) |
| --- | --- | --- | --- |
| rusqlite 0.38 (bundled) | palace-store, palace-mempalace | KG opened in place, `compiler.sqlite` hash-chained write log and claim store, one portable SQLite; palace-mempalace reads the drawer count from Chroma's sqlite read-only for `palace init` (M4b, no new package) | 14 (libsqlite3-sys + cc build chain, hashlink/hashbrown, smallvec, bitflags) |
| serde 1 (derive) | palace-core, palace-cli | Domain rows (triples, entities, search hits) as stable JSON | 7 (serde_derive, proc-macro2, quote, syn) |
| serde_json 1 | all six crates | MCP JSON-RPC, audit payloads, fixtures, CLI output | 5 (itoa, memchr, zmij) |
| sha2 0.10 | palace-core | Triple ids (`ids.py` recipe) and the write-log hash chain | 10 (digest, block-buffer, generic-array, cpufeatures) |
| clap 4 (derive, trimmed features) | palace-cli | Typed `palace` subcommands and flags | 11 (clap_builder, clap_derive, anstyle, strsim) |
| tempfile 3 (dev only) | palace-store, palace-mempalace | Isolated KG, audit-chain and diary-confinement tests | dev only (fastrand, rustix, …) |
| palace-core / -store / -mempalace (local) | cli, mcp | Clean Architecture layers | — |

## Counts (M4b, 2026-09-26; unchanged since M4a)

- `Cargo.lock`: **69 packages** = 6 workspace crates + 63 registry crates, all targets and dev
  dependencies included (this is the number `palace doctor --acceptance m3c` reports as `crates=`).
  **Ceiling 100** (PLAN v3 NN 7; was 200 with the ONNX runtime).
- Normal + build graph of the shipped binaries: **43** registry crates.
- Duplicate versions (`cargo tree --workspace -d`): **0**.
- Removed with the Rust store (M3c): tract-onnx, tokenizers and their trees (the old lock had
  ~200 packages). `grep -c -E '^name = "(tract-onnx|tokenizers|ort)"' Cargo.lock` → 0.

## Gates

`cargo deny --offline check` and `cargo audit` run against the project-local, pre-fetched advisory
database; fetching is disabled. The obsolete paste advisory exceptions were removed in M4a S6; no exceptions added.
`#![forbid(unsafe_code)]` in every crate.

The 63 registry lock entries comprise 43 host normal+build crates, 12 additional
rusqlite wasm-target dependencies, and 8 additional dev/platform dependencies.
The earlier NOTES attribution of all 20 extra entries to tempfile was incorrect.


M4a re-measurement: Cargo.lock **69** = 6 local + 63 registry; unique host
normal+build registry packages **43**, all-target normal+build **55**, duplicate
versions **0**. Measured with `cargo tree --workspace --offline --edges normal,build
--target aarch64-apple-darwin` and `--target all`, deduplicating package identities.
No registry dependency added. `palace-judge` now uses the existing palace-core and
serde_json dependencies; subprocess deadlines and UTC report names use std only.

M4b.1 re-measurement (2026-09-26): Cargo.lock **69** packages (6 local + 63 registry), unchanged;
`cargo audit` scanned 69 crate dependencies. No dependency added: the typed `Unavailable` error,
the citations table, finding dedupe and the per-call temp directory use std, rusqlite and
serde_json already in the graph.

M4c re-measurement (2026-09-26): Cargo.lock **69** packages (6 local + 63 registry), unchanged; no
manifest or lock change since M4b.1 (`git diff 1780e36 -- Cargo.lock Cargo.toml crates/*/Cargo.toml`
empty); host normal+build registry crates **43**; duplicate versions **0**. `palace regress`, the
diary-only step and the M4c doctor use std, serde_json and the existing MCP client. The diary-only
step runs a 20-line Python program (`crates/palace-mempalace/src/diary_only.py`, on stdin) with the
pinned MemPalace tool's own interpreter and its chromadb; that is part of the pinned runtime
dependency, not a new one.

M4b.2 re-measurement (2026-09-26): Cargo.lock **70** packages = **7 local** + **63 registry**. The
one new entry is the internal path crate `palace-testkit` (M4b.2 L-6: the ported Python watch
cases and their in-memory diary / table judges moved out of the domain crate); it depends only on
palace-core and serde_json, which were already in the graph, so the registry set is unchanged
(`git diff Cargo.lock` adds only `palace-testkit` and its three dependents' references). It does not
count toward the external-crate ceiling. PUB-1a item 3 (2026-10-09) removed it again: the watch
cases became private doctor modules of palace-cli and the fake judge joined the in-memory diary
in palace-core (`diary_memory`), so Cargo.lock has **69** packages (**6 local** + the same **63
registry**). `cargo audit` scans 70 crate dependencies (0
vulnerabilities); `cargo deny --offline check` ok. Host normal+build registry packages: **42** by
`cargo tree --workspace --offline --edges normal,build --target aarch64-apple-darwin --prefix none`
deduplicated on `name version` (the earlier "43" counted one non-package line; the package list is
identical); all-target normal+build **54** by the same method; duplicate versions **0**
(`cargo tree --workspace -d --target all` prints nothing).
