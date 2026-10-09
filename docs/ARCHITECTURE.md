# context-compiler — architecture v3

Describes `main` after M4b (commit 41944fc). PLAN.md v3 is authoritative for scope and milestones,
SPEC-claims-and-improvement.md v1.0 for claims, DEPS.md for the dependency tree.

## 1. Purpose

MemPalace 3.5.0 is the memory store: it mines the diary, embeds, searches and keeps the knowledge
graph file. The context compiler is Callum's value-add on top of that pinned store. It decides what
may enter the graph (the write gate), compiles graph context for a question (traverse and the seven
compile tasks), compresses scattered diary threads into claims-first rollups, and checks those
rollups against their sources (the fidelity watch and the claims check). Portability and security
are the chief interests: one small Rust workspace, auditable, MIT-licensed, a measured dependency
tree, no required model, and nothing that writes to live data except through a gate that logs it.

## 2. Boundaries

MemPalace owns, and the compiler never reimplements:

- the ChromaDB palace (`chroma.sqlite3` + HNSW segments, drawers, closets, `mempalace_embedder.json`);
- mining, chunking, embedding, search, halls/rooms/hallways;
- `knowledge_graph.sqlite3`'s schema (MemPalace creates it; the compiler opens it in place and never
  runs DDL or changes its journal mode, NN 3);
- `mempalace-mcp`, which keeps serving search to the OpenClaw context engine.

The compiler owns:

- the **write gate** and evidence filter for proposed graph relations;
- **traverse/compile** over the KG (BFS, token budget, seven tasks, retire, stress);
- the **fidelity watch** (floor, depth, canary) and the **claims check**;
- the **claim store** (claims, findings, improvement events) and derived confidence;
- the **rollup pipeline** (`palace rollup`) and `palace init`;
- `compiler.sqlite` (hash-chained write log, WAL outbox, claim store) and the MemPalace-shaped WAL;
- **`palace-mcp`**, a stdio MCP server exposing only the eight `mempalace_kg_*` graph tools.

The link between the two is narrow: the compiler talks to MemPalace through `mempalace-mcp`
(JSON-RPC on stdio), the `mempalace` CLI (`--version`, `mine`) and the KG file. Nothing imports
MemPalace's Python.

## 3. Crate map

```
                    palace-core   (domain; no IO; sha2 + serde only)
                  ▲      ▲       ▲       ▲
                  │      │       │       │  implement core traits
        palace-store  palace-mempalace  palace-judge
        (rusqlite)    (rusqlite, std::process)  (std::process)
             ▲    ▲         ▲                  ▲
             │    └─────────┼──────────────────┤
          palace-mcp     palace-cli  ─────────┘   (wiring; the two binaries)
```

Dependencies point inward. `palace-core` defines the traits — `GraphStore` (+ `GraphHistory`,
`GraphMutation`), `Searcher`, `EvidenceSearch`, `DiaryReader`, `Judge`, `Model`, `Clock` — and every
rule that can be tested without a file or a process. Adapters implement them; binaries wire them.

- **palace-core** — entities (entity, triple, proposal, evidence, search hit, rollup entry, claim,
  canary), ids (`ids.py` v3 recipe), write gate + evidence filter, traverse/compile/retire/stress,
  fidelity floor/depth, canary and claims checks, rollup prep/select/cluster/validate, diary file
  recipe. No IO.
- **palace-store** — the KG opened in place (`Graph` for writes with a per-request advisory lock,
  `GraphReader` read-only and lock-free), `compiler.sqlite` (write log, WAL outbox, `ClaimStore`),
  the single-write WAL line, chain verification.
- **palace-mempalace** — the `mempalace-mcp` client (`Searcher`, 5 s startup / 20 s per request),
  the version pin (CLI `--version` and `initialize.serverInfo.version`), the confined `DiaryReader`,
  `mempalace mine` with the pinned child environment, the drawer count for `init`, KG creation
  through MemPalace's own `mempalace_kg_stats`.
- **palace-judge** — `Judge` and `Model` backends: `replay` (archived answers), `claude-cli`
  (clean argv, prompt on stdin) and `command:<exe>`; 120 s deadline; response = first flat
  `{…}` with a known verdict, else `unclear`.
- **palace-mcp** — binary `palace-mcp --kg <sqlite> [--wal <path>] [--palace <dir>]`; `--kg`
  defaults to `<palace>/knowledge_graph.sqlite3`. Reads (`query`, `entities`, `timeline`, `stats`)
  use `GraphReader`; writes (`add`, `invalidate`, `set_entity`, `retire_entity`) open `Graph`, which
  refuses on a broken chain. No search tool.
- **palace-cli** — binary `palace`: `propose-batch`, `kg {query|invalidate|entities|set-entity|
  retire-entity|timeline|stats}`, `traverse`, `compile --task`, `retire-entity`, `retire-fixtures`,
  `stress {record|gaps}`, `watch`, `rollup`, `init`, `doctor [--verify] [--acceptance m3a|m3b|m3c|
  m4a|m4b]`; global `--roots <dir>...`.

## 4. Data flow

### 4a. Per-turn context hook (traverse / compile)

```
 OpenClaw turn ─► context engine (TS, outside this repo; must never break if the compiler is down, NN 12)
                    ├─ search ───────► mempalace-mcp --palace <live>          ─► [PALACE CONTEXT]
                    └─ graph  ───────► palace-graph-substrate (TS) graph tools ─► [GRAPH]

 compiler read path (same algorithm, parity-proven 32/32 traverse, 56/56 compile):
   palace traverse <entity> --kg <sqlite> [--max-hops 2] [--token-budget 800] [--include-retired]
   palace compile  <entity> --kg <sqlite> --task support|contradict|boundary|analogies|
                                                  anti-analogies|dependencies|provenance
        │
        ▼
   GraphReader (KG mode=ro, no locks, no audit rows) ─► palace-core::traverse
        BFS over live triples, retired entities hidden unless asked
        order = JS localeCompare reproduced without ICU
        cost  = ceil(UTF-16 length of the row's JSON / 4), min 1; stop at the budget
        ─► palace-core::compile (task filter over the neighbourhood) ─► JSON on stdout
```

Today the hook's graph half is still the TS substrate. PLAN M5 points the substrate's graph
transport at `palace-mcp`; graph reads through `palace-mcp` take no writer locks, so a busy writer
or a broken chain never blocks the turn.

### 4b. Rollup pipeline (`palace rollup`)

```
palace rollup --diary <root> --palace <dir> --kg <sqlite> [--since 3d] [--as-of D] [--thread T]
              [--llm replay|claude-cli[:model]|command:<exe>] [--prompts prompts] [--dry-run]

 1 prep     Rust   entries in [as-of − since, as-of]; ROLLUPs excluded; no-compress flagged
                   < 3 substantive ─► "Palace compression: quiet period" exit 0
 2 select   Model  Rust offers ≤ 8 candidate threads (tags on ≥ 2 entries, entries × days);
                   Model returns {thread, reason, candidate_ids} | {bail, reason}  (skipped by --thread)
 3 pull     Rust   prior rollups' Supersedes primaries, then MemPalace top-10 per thread/tag query;
                   rollups never members; no-compress = reference-only; ≤ 60k chars, whole entries
 4 write    Model  prompts/rollup.md + cluster (name + full text) ─► rollup body
 5 validate Rust   title, ## Claims (pinned grammar, ≥ 3, sources are cluster primaries, quotes
                   verbatim), ## Supersedes, relations cite claims and pass the gate dry run,
                   floor/depth (+ canaries with a judge), size 2k–40k
                   fail ─► one retry with the report appended ─► still failing: exit 4, nothing written
 6 commit   Rust   --dry-run: print body + report, exit 0
                   else: write diary file (recipe §5) ─► write-log "rollup_commit"
                         ─► mempalace mine ─► propose relations through the gate
                         ─► watch the new rollup + record claims in the store
                         ─► "ROLLUP written <file> claims=<k> relations=<n> watch=<status>"
 MemPalace unavailable at pull or commit ─► exit 3
```

The model does exactly two typed jobs (select, write) through the `Model` trait; everything that
can be checked is Rust. Backends: `replay` answers select with the archived thread and write with
`fixtures/rollups/valid-replay-rollup.md`, so the whole pipeline runs offline with no model;
`claude-cli[:model]` and `command:<exe>` receive the prompt on stdin. No specific model is required
(NN 8); replay runs with no judge, so canaries are skipped and claims are checked mechanically.

### 4c. Fidelity watch (`palace watch`)

```
palace watch [--diary D] [--canaries J] [--reports R] [--no-judge] [--store compiler.sqlite]
             (--since-minutes N | --all | --rollups DIR)
             [--judge replay|claude-cli|command:<exe>] [--model sonnet] [--judge-settings F]

 rollups ─► floor   no superseded source carries no-compress; dangling names reported
         ─► depth   primary 0, rollup 1 + max source depth; depth ≥ 2 with no primaries = fail,
                    with some = warn; chain, primary_share (half-even, as Python round())
         ─► canary  in scope when entity aliases + a topic term appear; Judge: preserved|reversed|unclear
         ─► claims  parse ## Claims (malformed line = malformed_claim finding, never a crash)
                    mechanical: unsupported (quote absent and entities not within 600 chars),
                    stale (later live claim, same subject+predicate, other object), absent-source;
                    reversal: Judge called when subject and object are near each other in the source
         ─► report  <reports>/<UTC YYYYMMDDTHHMMSSZ>.json + one stdout line
         ─► --store claims upserted, one finding per failing claim, confidence re-derived (§5)
```

`claude-cli` runs clean: `claude -p --model <m> --tools "" --strict-mcp-config --setting-sources
project --settings prompts/judge-settings.json` (`{"disableAllHooks": true}`) from an empty temp
directory, so the user's palace-context hook never feeds the judge. Passage cap 12,000 chars.

### 4d. Write gate (`palace propose-batch`, `palace-mcp` add, rollup commit)

```
proposal {from, to, type, from_type, to_type, evidence:[{query}], confidence, supplied_by, claim?}
   │  checks in the pinned order (verbatim reasons, parity 16/16 with the TS gate):
   ├─ from_type / to_type ∈ {person, project, decision, fact, document, system, thread, event}
   ├─ direct first-order evidence present; every reference has a query (≤ 250 chars)
   ├─ each query ─► MemPalace search (limit 3, max_distance 1.5)
   │      hit ─► DiaryReader: first 800 chars of source_path, confined to the diary root
   │             (fallback: the hit text) ─► bookkeeping title/tag? ─► "circular"
   ├─ confidence finite; supplied_by present
   ▼
 --dry-run: print {ok, reason}; graph never opened
 else Graph::open: advisory lock ─► verify chain (refuse on break) ─► one SQLite transaction:
      triple (provenance JSON in source_closet: confidence, supplied_by, evidence_refs,
      assessed_at, written_via, claim, claim_id) + write_log row + WAL outbox row
      ─► WAL line: one write(2), O_APPEND, fsync ─► outbox row cleared
```

A content failure from MemPalace (error object, over-long query, `vector_disabled`) rejects that
proposal alone; a transport failure (spawn, handshake, pin, timeout) makes `propose-batch` exit 3.

## 5. Persistent state

**`compiler.sqlite`** sits beside the KG. It is the compiler's own database (journal WAL, foreign
keys on) and is attached to the KG connection for graph writes, so a triple, its log row and its
outbox row commit together. A `*.sqlite.lock` advisory lock file sits beside each database.

```
write_log(sequence PK, payload, previous_hash, hash)       append-only by trigger
    hash = sha256(lp(sequence, previous_hash, payload)); operations: kg_*, init,
    rollup_commit, claims_record, wal_separator_recovery
graph_wal_outbox(sequence PK, path, line)                  pending WAL lines until appended
claims(id PK, number, rollup, subject, predicate, object, date, source, quote, status,
       superseded_by, confidence_base, checks, survived, first_seen, last_checked)
findings(id PK, claim, rollup, kind, evidence, proposal, checker, checked_at, decision)
improvement_events(id PK, at, by, verb, from_claim, to_claim, finding, text,
       axis_improved, reversible_by)                       append-only by trigger; empty until M4d
```

Claim id = `c_` + sha256(lp(lower subject, predicate with spaces → `_`, lower object, date,
source))[:12]; finding id = `f_` + sha256(lp(claim or rollup:line, kind, checked_at))[:12].
Confidence is derived, never declared: base 0.80 with a verified quote, 0.60 without, plus 0.05 per
survived check, minus 0.15 per finding, clamped to [0.05, 0.99] (SPEC §5).

**WAL** `wal/write_log.jsonl` beside the KG, i.e. inside the palace (`--wal` overrides): one MemPalace-shaped JSON line per graph
operation, `{timestamp, operation: "kg_<op>", params, result}`; new dirs 0700, files 0600.

**Reports** `<reports>/<UTC YYYYMMDDTHHMMSSZ>.json` = `{summary:{rollups, fail, warn, error},
results:[{check: floor|depth|canary|claim, rollup, status, …}]}`, the archived Python shape.

**Diary file recipe** (pinned from OpenClaw's diary-write plugin): `day` = America/New_York date
of now (DST rules hand-coded); `hash = sha256("{ISO-8601 UTC with ms}\n{title}\n{text}")[:12]`;
file `<root>/diary/<day>-<hash>.md`, created exclusively; content `# {title}\n\n{text.trim()}\n`;
then `mempalace --palace <p> mine <root> --wing openclaw --agent openclaw`. The ISO timestamp is
kept in the `rollup_commit` log row, so the name can be re-derived from the content.

## 6. Security posture

- **Fence.** The live palace, diary and MemPalace WAL are read-only for the whole build (NN 1);
  tests and doctors run on fixtures copied under `target/`. Every MemPalace child spawned against a
  palace gets `MEMPALACE_PALACE_PATH=<that palace>` (plus `HF_HUB_OFFLINE=1`,
  `TRANSFORMERS_OFFLINE=1`, `ANONYMIZED_TELEMETRY=False`); `HOME` is inherited unchanged. MemPalace
  places hallway files beside `config.palace_path`, not `--palace`, so without that variable a
  scratch mine overwrites live derived data. Known residue: lock files under `~/.mempalace/locks/`,
  keyed on the target path.
- **Paths.** Inputs are canonicalised under `--roots` (default: the project directory); outputs
  additionally refuse `fixtures/`, `..` and symlinks. `source_file`, `wing`, `room` are validated
  as strings and never opened, except the 800-char evidence header under the diary root.
- **Integrity.** Hash-chained append-only write log; `doctor --verify`; graph writes refuse on a
  chain break while reads still work. Append-only data: retire = mark, invalidate = `valid_to`.
- **Code.** `#![forbid(unsafe_code)]` in all six crates. No shell strings: `Command` + args,
  child input on stdin. No secrets in the repo.
- **Supply chain.** `Cargo.lock` committed; `cargo build --locked`; `cargo deny --offline check`
  and `cargo audit` against a pre-fetched advisory DB with fetching disabled; no advisory
  exceptions; duplicates denied. **69** packages in `Cargo.lock` (6 workspace + 63 registry;
  43 in the shipped normal+build graph) against a **ceiling of 100**. No ONNX, tokenizer or weights.
- **No required model.** Model calls exist only behind `Judge`/`Model`; mine, search and graph
  never call one, and `replay` runs every pipeline offline.
- **Deferred.** Encryption at rest (PLAN Decided, after M5); palace-store's `encrypted-store`
  feature is an empty placeholder.

## 7. Operations

`scripts/mempalace-check.sh [--install] [--palace <dir>] [--diary <dir>] [--quiet]` checks uv,
the exact 3.5.0 pin (CLI and `mempalace-mcp` serverInfo), palace layout, diary count, KG
`integrity_check`, and an MCP handshake plus smoke search. It is read-only unless `--install`,
which (re)installs the pin with `uv tool install`. `palace init` runs it.

Acceptance doctors (full blocks in PLAN.md; every gate runs on fresh copies under `target/`):

```
palace doctor --acceptance m3c --palace target/m3c/palace --diary fixtures/diary \
  --kg target/m3c/palace/knowledge_graph.sqlite3 --rollups fixtures/rollups \
  --expected-gate fixtures/rollups/EXPECTED.json \
  --expected-traverse fixtures/graph/EXPECTED-traverse.json --mcp target/release/palace-mcp
M3c OK mempalace=3.5.0 tests=<n>/<n> batch_parity=16/16 traverse_parity=32/32 compile_modes=56/56 retire=ok stress=ok mcp_kg_tools=8 chain=ok crates=<n>

palace doctor --acceptance m4a … --reports fixtures/fidelity-reports --canaries fixtures/canaries.json \
  --expected-claims fixtures/rollups/EXPECTED-claims.json
M4a OK tests=<n>/<n> reports_match=5/5 exit_codes_match=5/5 claims=<k>/<k> header_reads=<n>>0 version_pin=cli+server batch_isolated=ok kg_default=ok

palace init target/m4b/diary-root target/m4b/palace-init --mempalace-check scripts/mempalace-check.sh
INIT OK mempalace=3.5.0 entries=452 drawers=<n> kg=… compiler=…
palace doctor --acceptance m4b --diary target/m4b/diary-root … --prompts prompts
M4b OK bail=ok select=halloran-thread pull=<n> validate=pass claims=12 relations=1 dryrun=ok commit=ok recipe=ok malformed=finding grammar=ok reversal=flagged mempalace_down=exit3 judge_clean=ok store=ok
```

`--acceptance m3a|m3b` still run the retained graph/gate/traverse checks and print their lines
(store-era fields report `not-applicable`). `palace doctor --verify --kg <sqlite>` prints
`VERIFY OK chain=ok`.

| Command | 0 | 1 | 2 | 3 | 4 |
| --- | --- | --- | --- | --- | --- |
| `palace` (any) | ok | error (`palace: …`) | usage (clap) | — | — |
| `palace watch` | no findings / no rollups | any warn, fail or error | the run broke | — | — |
| `palace propose-batch` | batch evaluated (rejections included) | graph/other error | usage | MemPalace unavailable | — |
| `palace rollup` | written, quiet, bail, dry run | error | usage | MemPalace unavailable | invalid after retry |
| `mempalace-check.sh` | healthy | a check failed | usage / tooling missing | — | — |

## 8. Roadmap (details in PLAN.md)

- **M4c** — `scripts/mempalace-regression.sh`: 50 frozen queries through `mempalace-mcp` against
  `fixtures/mempalace-search-50.json`, plus the M3c doctor; the procedure for changing the pin.
- **M4d** — the improvement loop: `palace improve <verb>`, `palace findings`, `palace depends`,
  Signal reply parsing, error → canary promotion (SPEC §3, §6, §7).
- **M5** — cutover by Fergus: substrate graph transport → `palace-mcp`; crons → `palace
  propose-batch`, `palace watch`, `palace rollup`; one week shadow; Callum flips.
- **M6** — portability: Linux/aarch64 build, `mempalace-check.sh --install` and `palace init`
  end to end on a fresh box.

## 9. Changelog

v3 — 2026-09-26, replaces v2 (store era, see archive branch)
