# MemPalace Memory 0.3.0

Repository build for the assistant's operator to install at D4. Exports memory_search, mempalace_search,
memory_get, mempalace_diary_write and mempalace_status.

Configure `palacePath`, `storePath` (that palace's compiler.sqlite), and `peopleMap`
explicitly. Diary writes also require `diaryRoot`, the parent of the main `diary/`
directory. `palaceBin` defaults to `palace` on PATH; `mempalaceMcp` optionally selects
the search adapter; `mempalace` optionally selects the mining binary for diary writes.
Legacy `wing`, `agentName`, and `diaryDir` config keys are accepted but unused.
Tool wing/room arguments are ignored; diary writes always use the main diary.

Diary writes use `palace diary-write --conversation=<host key>` or `--owner` through
the shared MCP marker-aware writer. They return the filed basename and mining state;
`mined: false` means the file remains for nightly mining, so do not refile it.
Owner writes carry an owner-only private marker; conversation writes carry the resolved
conversation marker. Missing/unknown identity writes nothing, with a one-line notice.
Status is a null tool outside owner-private contexts. It uses `palace status --owner`
for read-only counts of marked entries, Chroma drawers, claims, findings and improvement
events; no entry text or raw backend response is returned.

Missing configuration or unproven host audience returns no data. No identity is
inferred from model parameters, session-key shapes, requester names, or owner flags.

Tool factories request context version 2. Both currency guards run before release,
including empty/error results. A stale guard throws and prevents any result release.
Backend stderr and exceptions are never returned. memory_get searches by id/query
through the same marker-aware command, with limit 1; it never opens a raw file.

`node --test` uses a stub binary and synthetic data under this repository's target/.
It does not load OpenClaw, install the plugin, or contact any live service. Host
loading and per-session live differential checks remain D4/D6 deployment work.

## Native provider runtime

Requires OpenClaw 2026.10.7 or newer. Registering `providerRuntime` lets the selected
memory slot receive host-minted audiences. Session callers with an owner-private
audience search owner-visible records; conversation audiences search only records
visible to that host conversation. Missing or cross-agent audiences deny reads.
Operator and host callers can check health only; neither grants memory access.
Tools continue reading `context.memoryAudience` with their existing schemas.

The caller-bound handle supports memory search and reference retrieval, without
pagination, project filters, or automatic-recall candidates. References use palace
drawer IDs (entry basenames when no drawer ID exists). Retrieval requires an exact
ID match in the marker-aware search result. Scores use `clamp(1 - distance / 2, 0, 1)`;
hits without a distance cannot satisfy a requested minimum score.
Health probes only binary readiness, returning no counts, text, or paths.
Closing a handle releases that caller alone. Currency checks fence every provider
backend execution and result release. Provider errors contain only fixed notices.
