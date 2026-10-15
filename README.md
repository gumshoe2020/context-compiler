# context-compiler

A memory compiler for coding and chat assistants, in Rust. It sits on top of
[MemPalace](https://pypi.org/project/mempalace/) (pinned at 3.5.0), which stores and searches a
diary of conversations, and decides what gets *compiled* from that diary:

- **distillation**: when a conversation ends, its new text becomes diary entries, typed claims
  (specifics such as dates and amounts, and concepts) and graph relations;
- **rollups**: scattered entries on one thread are consolidated into a claims-first summary, each
  claim quoting its source;
- **a fidelity watch**: every rollup and distillation is checked against its sources (missing
  sources, reversed claims, quotes that do not carry the claim), with findings the person can
  confirm, correct or undo;
- **a knowledge graph** with a write gate (evidence required, audit-logged, hash-chained);
- **per-turn context**: before each prompt, a short block of the relevant specifics, graph
  relations, the thread's latest rollup and newer entries, within a token budget.

No model is required by the core: the commands that need one (distill, rollup, the judge) take a
`--llm`/`--judge` spec, either a command that reads the prompt on stdin or a recorded table.

**Status: alpha.** Built for one user's setup (macOS on Apple silicon, Codex or Claude Code,
MemPalace 3.5.0). It runs there daily; anywhere else, expect rough edges. Issues welcome,
support is best effort.

## Install

Needs macOS on Apple silicon, [uv](https://docs.astral.sh/uv/), `jq`, a Rust toolchain (1.89 or
later) to build from source, and Codex or Claude Code on PATH, logged in. The installer pins
MemPalace with uv, creates a palace under `~/.context-compiler` (`CC_HOME`), registers the hooks
and the MCP server with your assistant, and schedules the settle and daily jobs with launchd.

For Codex:

```
dist/codex/install.sh --from-source --dry-run    # what it would change
dist/codex/install.sh --from-source
```

Then start Codex, type `/hooks` and trust the three context-compiler hooks (Codex skips a new or
changed hook until you do).

For Claude Code:

```
dist/claude-code/install.sh --from-source --dry-run
dist/claude-code/install.sh --from-source
```

`--from-source` builds the release binaries with cargo. Alternatively, `dist/build-release.sh`
packs a tarball (`target/dist/context-compiler-<version>-aarch64-apple-darwin.tar.gz`) whose
`dist/codex/install.sh` and `dist/claude-code/install.sh` install from the packed binaries.

Useful options (both installers): `--person <name>` and `--assistant <name>` (the two roles the
memory records: you correct it and answer its asks; the assistant keeps the queue of reversals),
`--wing <name>` (this memory's MemPalace wing, default `memory`), `--exclude-wing <name>` (a
private wing never recalled automatically), `--no-load` (write the launchd jobs without loading
them). `dist/README.md` has the full reference, removal included (`dist/common/uninstall.sh`).

### With Nix

With Nix and flakes enabled, `nix build` produces `result/bin/palace` and
`result/bin/palace-mcp`. `nix develop` provides the pinned Rust toolchain and
development tools; `nix flake check` runs the sandboxed checks. Apple silicon
macOS is tested; the other declared platforms are evaluated only. See
[docs/NIX.md](docs/NIX.md) for test exclusions and updating the lock. Nix builds
the binaries; installing MemPalace and assistant integrations is a separate step.

## Commands

The `palace` binary (`palace <command> --help` for each). A build from this tree has no default
names: every command except `context` and `init` needs `--person <name> --assistant <name>` (the
installers pass them).

| Command | What it does |
|---|---|
| `init` | Create a palace and mine a diary into it |
| `context` | The per-turn context block for one message (read-only, fails open) |
| `distill` | One transcript's new text into entries, typed claims and relations |
| `rollup` | Consolidate one thread into a claims-first rollup |
| `watch` | The fidelity watch over rollups (floor, depth, canaries, claims) |
| `fidelity` | Recall at both compression points (observe only) |
| `findings`, `depends` | The claim store's open findings; the provenance around one claim |
| `improve` | Apply one verb (confirm, weaken, contradict, retract, …) or undo one; `--auto` |
| `terms`, `lexicon` | Recurring terms and the lexicon compiled from them |
| `traverse`, `compile` | Graph context for a question (breadth-first, token budget, seven tasks) |
| `propose-batch`, `kg`, `stress`, `retire-entity` | Gated graph writes and graph maintenance |
| `regress` | Frozen queries through the pinned MemPalace against frozen answers |
| `doctor` | Audit-chain verify, divergence checks, settling an interrupted write |

`palace-mcp` serves the graph and diary tools over MCP (stdio). `scripts/palace-cron.sh` wraps
the scheduled jobs; `dist/common/settle.sh` runs a settle when a conversation ends.

## Architecture in brief

A Cargo workspace of six crates, dependencies pointing inward:

- `palace-core`: the domain, with no I/O: claims, rollups, the fidelity rules, the write gate,
  traversal, context blocks, transcripts;
- `palace-store`: the claim store and the graph (SQLite, opened in place next to MemPalace's
  knowledge graph, every write logged and hash-chained);
- `palace-mempalace`: the adapter to the pinned MemPalace (version check, MCP client, diary,
  mining);
- `palace-judge`: model adapters for the judge and the writer (`command:`, `table:`, `replay`);
- `palace-cli` (`palace`) and `palace-mcp`: the two binaries.

`docs/ARCHITECTURE.md` describes the design, `docs/DEPS.md` the dependency budget (63 registry
crates, audited with `cargo deny` and `cargo audit`), `docs/RUNBOOK.md` operations. Prompts the
models receive are plain files in `prompts/`.

## Development

```
cargo build --workspace --locked
cargo test --workspace --locked
cargo clippy --workspace --all-targets -- -D warnings
```

The tests need no network, no model and no MemPalace install (stand-ins play MemPalace and the
models). `testdata/` holds the synthetic fixtures. This repository is an export of a
private one (which also holds acceptance tests on a real palace); history starts at the first
export.

## Background

The idea for compiling context, rather than just storing and searching it, came from el gato
malo's essay [The geometry of relational intelligence](https://boriquagato.substack.com/p/the-geometry-of-relational-intelligence).
It argues that effective capacity is working memory × compression × fidelity, and that a compressed
concept becomes an axis other concepts are hung on, so a small error rotates everything built on
it. That is why this compiler keeps every exact fact as a verbatim quote with its source, and checks
every summary against those sources before accepting it.

## License

MIT, see `LICENSE`.
