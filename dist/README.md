# Installing the context compiler (dist/)

One script gives a Mac (Apple silicon) the memory loop: a palace, per-turn context, diary
search and writing through MCP, a settle run when a conversation ends (watch, automatic
improvements, asks in the daytime), a daily run at 05:00 (the asks and the digest, and a backstop
for a missed settle run) and consolidation by volume or age. Codex (`codex/install.sh`) and Claude Code
(`claude-code/install.sh`); OpenClaw and Railway follow.

## Codex

Needs macOS on Apple silicon, [uv](https://docs.astral.sh/uv/), `jq` (macOS 15 and later ship
`/usr/bin/jq`) and Codex on PATH, logged in.

```
dist/build-release.sh                       # once, in a checkout: target/dist/context-compiler-<version>-aarch64-apple-darwin.tar.gz
tar -xzf context-compiler-<version>-aarch64-apple-darwin.tar.gz
context-compiler-<version>-aarch64-apple-darwin/dist/codex/install.sh --dry-run   # what it would change
context-compiler-<version>-aarch64-apple-darwin/dist/codex/install.sh
```

In a checkout without a tarball: `dist/codex/install.sh --from-source` (cargo builds the
binaries). Both build them as released: `cargo build --release --no-default-features` (the
acceptance doctors, cargo feature `doctors`, are left out) with the home and the checkout
remapped out of compiled paths, into target/release-dist; `build-release.sh` packs nothing when
`dist/test/deny-list.sh` finds an owner's name, build path, canary or fixture text in the
binaries or in any other staged file (LICENSE's copyright line is the one allowed occurrence),
and removes the tarball when its bytes, headers included, hold one (a tree without that private
list, such as the public repository, skips the check and says so); its entries belong to
root:wheel and carry no extended attributes. Then start Codex, type `/hooks` and trust the three context-compiler hooks (user-prompt.sh,
session-start.sh, session-end.sh), and again after any install that changed a hook: Codex trusts a
hook by its exact contents and silently skips a new or changed one. The installer's last line, a
dry run's too, says so.

Options: `--llm <spec>` and `--judge <spec>` (the settle and daily jobs' models; default
`command:$CC_HOME/bin/codex-llm.sh`, which runs `codex exec --ignore-user-config --ignore-rules
--disable hooks --disable apps --disable plugins --disable multi_agent --disable
image_generation -c web_search="disabled" --sandbox read-only` with the prompt on stdin, so the
unattended run has none of the user's MCP servers, rules, hooks, account tools (the ChatGPT
account's apps such as Gmail) or web search), `--codex-model
<model>` (the model codex-llm.sh pins; default: the top-level `model` of config.toml at install,
else Codex's own default), `--wing <name>` (the MemPalace wing of this memory, default
`memory`: diary entries, `palace init` and the rollups file in it), `--exclude-wing
<name>` (repeatable: a private wing never recalled automatically; the per-turn hook searches
every wing but these, with `palace context --all-wings --exclude-wing …`), `--person <name>`
and `--assistant <name>` (the two roles the memory records by name: you correct it and answer
its asks, the assistant keeps the queue of reversals; lowercase; default: an earlier install's,
else asked when the installer runs at a terminal, else your login name and `codex`; passed to
every command and job the installer writes: the per-turn hook's `palace context`, `palace
init`, `cc-graph` and the two jobs. The release `palace` has no default names: run by hand,
every command but `context` and `init` needs `--person <name> --assistant <name>`), `--no-load` (write the launchd
plists under `$CC_HOME/launchd/` without loading them). `CC_HOME` (default `~/.context-compiler`) and `CODEX_HOME`
(default `~/.codex`) move the two places it writes.

Wings, plainly: only this memory's own wing (`--wing`) is compiled. Its diary entries live in
`$CC_HOME/diary-root/diary/`, which the settle and daily jobs read: rollups, the claims, the graph and
the asks come from it alone. An entry `palace_diary_write` files in any other wing is written to
`$CC_HOME/wings/<wing>/diary/` and mined from there into that wing only, so it is searchable but
never compiled: no rollup, claim or graph relation is ever made from it, even when its mine
failed. Per turn, the hook recalls excerpts from every wing except the excluded ones
(`--exclude-wing`): an excluded wing is never recalled automatically, another wing is recalled
but not compiled. No other wing's entry feeds the graph behind the `[GRAPH]` block.

## Claude Code

Needs macOS on Apple silicon, uv, `jq` and Claude Code on PATH (`claude`), logged in.

```
context-compiler-<version>-aarch64-apple-darwin/dist/claude-code/install.sh --dry-run
context-compiler-<version>-aarch64-apple-darwin/dist/claude-code/install.sh
```

The same core, jobs, wings, names and rules as the Codex installer (`--from-source`,
`--no-load`, `--wing`, `--exclude-wing`, `--person`, `--assistant`; the assistant's default name
is `claude`). Claude Code's own files: `CLAUDE_CONFIG_DIR` (default `~/.claude`) holds
`settings.json` and `CLAUDE.md`; its user MCP servers live in `$CLAUDE_CONFIG_DIR/.claude.json`,
or `~/.claude.json` when `CLAUDE_CONFIG_DIR` is unset. The installer adds:

- to `settings.json`: a `UserPromptSubmit` hook (`$CC_HOME/hooks/user-prompt.sh`) and a
  `SessionStart` hook (`session-start.sh`, matcher `startup|resume`), timeout 5 s each, printing
  `hookSpecificOutput.additionalContext` JSON after the same framing line; a `SessionEnd` hook
  (`session-end.sh claude-code`, timeout 3 s) that asks for a settle run and prints nothing; and
  `permissions.allow` entries for exactly the tools the Codex installer approves
  (`mcp__cc-memory__<tool>` for the read tools, `mcp__cc-graph__<tool>` for the graph reads,
  `palace_diary_write` and `palace_reply`); every other key and entry is kept;
- user MCP servers `cc-memory` and `cc-graph` (`claude mcp add -s user`, the same commands as the
  Codex installer's, cc-graph with `--agent claude-code`); a user server of either name that the
  installer did not add stops the install before any change;
- the memory protocol (`claude-code/CLAUDE-memory.md`, the Codex block with Claude Code's tool
  names) between two marker lines at the end of `CLAUDE.md`.

What it added is recorded in `$CC_HOME/install-state` (`claude-mcp-server`, `claude-permission`)
and `$CC_HOME/claude-env.sh`. The nightly model is palace's own `claude-cli:claude-sonnet-5`
(`--llm`/`--judge` override it): `claude -p --model <model> --tools "" --strict-mcp-config
--setting-sources project --settings $CC_HOME/prompts/judge-settings.json` (`disableAllHooks`)
from an empty directory, so the unattended run has no tools, no MCP servers and none of the
user's settings or hooks. `--bare` is not used: it reads no OAuth login. The job's environment
gets `CLAUDE_CONFIG_DIR` (when set, so `claude` uses the person's login) and
`CLAUDE_CODE_SAFE_MODE=1` (what `claude --safe-mode` sets: no CLAUDE.md, skills or plugins), and
its PATH claude's directory (and node's for an npm install); `$CC_HOME/bin/claude`, first on that
PATH, links to the `claude` the installer found, so no other `claude` (say one beside MemPalace)
runs at night. Then: start (or restart) Claude Code
so it loads the hooks and servers; if it asks to review the hooks (`/hooks`), accept the three.

## What it changes

Everything lives in two places, plus two launchd agents:

| where | what |
|---|---|
| `$CC_HOME` (mode 700) | `install-state` (the MCP servers and approval tables it added), `bin/` (palace, palace-mcp, notify-local.sh, settle.sh, codex-llm.sh), `scripts/` (palace-cron.sh, mempalace-check.sh), `prompts/`, `share/canaries.json` (empty), `hooks/`, `dist/` (these scripts, for uninstall), `palace/`, `diary-root/diary/`, `wings/<wing>/diary/` (entries filed in another wing), `state/` (`edits`, `cron/`, `inbox-read`, `settle.request` and `settle.request.taken`, `last-settle`, `last-compression`), `launchd/` (with `--no-load`), `logs/`, `inbox.md`, `env.sh`, `codex-env.sh`, `install.log` |
| `$CODEX_HOME` | `hooks.json`: a `UserPromptSubmit` hook (`$CC_HOME/hooks/user-prompt.sh`, timeout 5 s), a `SessionStart` hook (`session-start.sh`, matcher `startup\|resume`) and a `SessionEnd` hook (`session-end.sh codex`, timeout 3 s, Codex's most), other hooks kept; `config.toml`: MCP servers `cc-memory` (`mempalace-mcp --palace $CC_HOME/palace`, with `MEMPALACE_MCP_ALLOW_PEER_WRITER=1`: MemPalace's server otherwise takes the palace's mine lock on its first status call and keeps it for the session, so no entry could be indexed while Codex runs; its own writes still lock one at a time) and `cc-graph` (`palace-mcp --kg … --palace … --diary-root $CC_HOME/diary-root --wing <wing> --agent codex --mempalace … --mempalace-check … --wings-root $CC_HOME/wings --person <person> --assistant <assistant> --palace-bin $CC_HOME/bin/palace`: the graph tools, `palace_reply` (the user's answer to a question of the nightly job, applied as `palace improve --reply <reply> --token <token> --by <person>`) and `palace_diary_write`, which files a diary entry as a file in `$CC_HOME/diary-root/diary/` (in another wing: `$CC_HOME/wings/<wing>/diary/`) and mines it into the palace; a mine refused because another process holds the palace's lock is tried again after 1, 2 and 4 s; a mine still running after 20 s in all is ended with its descendants, the file stays and the reply says so), added by `codex mcp add` (an earlier install's server with other arguments is removed and added again) (a server of either name that the installer did not add stops the install; a PKG-1 install's `mempalace` and `palace-graph` are removed in favour of the new names), and `approval_mode = "approve"` tables (`[mcp_servers.<server>.tools.<tool>]`) for their read tools, `palace_diary_write` and `palace_reply` (the graph's writes, and `cc-memory`'s `mempalace_diary_write`, keep asking; an earlier install's approval of the latter is removed), both recorded in `$CC_HOME/install-state`; `AGENTS.md`: the memory protocol (`codex/AGENTS-memory.md`) between two marker lines at the end |
| `~/Library/LaunchAgents` | `com.context-compiler.settle.plist` (runs when `$CC_HOME/state/settle.request` changes) and `com.context-compiler.daily.plist` (05:00 daily); with `--no-load` they are written to `$CC_HOME/launchd/` instead and nothing is loaded; an earlier install's `com.context-compiler.nightly` and `com.context-compiler.compression` agents are booted out and removed |

MemPalace: a `mempalace-mcp` on PATH at the pin (the `PIN=` of `scripts/mempalace-check.sh`,
today 3.5.0) is used as it is. One at another version stops the install before any change, with
the command to run: a MemPalace is never replaced. With none on PATH, uv installs the pin into
`$CC_HOME/uv` (`UV_TOOL_DIR=$CC_HOME/uv/tools UV_TOOL_BIN_DIR=$CC_HOME/uv/bin uv tool install
mempalace==<pin>`), so nothing goes in `~/.local/bin`; uninstall removes it.

Outside `$CC_HOME` and `$CODEX_HOME` (Claude Code: `$CLAUDE_CONFIG_DIR` and its `.claude.json`), an install changes only these:

| what | why | undone by uninstall? |
|---|---|---|
| `~/Library/LaunchAgents/com.context-compiler.settle.plist` and `com.context-compiler.daily.plist`, loaded with `launchctl bootstrap gui/<uid>` | the settle and daily jobs (not with `--no-load`) | yes, booted out and removed |
| `<file>.cc-backup-<time>` beside each edited file in `$CODEX_HOME`, `$CLAUDE_CONFIG_DIR` (and `~/.claude.json`) | the copy taken before each edit (and before uninstall's own strip) | no, kept by design; a `config.toml` or `.claude.json` backup may hold MCP servers' env secrets |
| `~/.cache/uv/` | uv's cache, when uv installs MemPalace | no |
| `~/.cache/chroma/onnx_models/` | MemPalace's embedding model, downloaded once by the first `palace init` on an account that lacks it (the install needs the network then) | no |
| `~/.mempalace/locks/mine_palace_<hash>.lock` | MemPalace's own lock, written by every `mempalace mine` (`palace init`, a compression night), keyed by the palace path; harmless, and on a machine with its own MemPalace it lands in that directory | no |
| `~/Library/Caches/com.apple.python/` | `/usr/bin/python3`'s cache, when `mempalace-check.sh` runs it | no |
| Codex's record that the three hooks are trusted (`/hooks`) | the one manual step | unknown (to check with real Codex) |
| a "background item" entry in System Settings → General → Login Items | macOS lists every launchd agent there | goes with the agent |

The installers touch no other palace.

Both jobs run `$CC_HOME/bin/settle.sh` (the daily one with `--daily`) with `--llm <llm> --`
and palace-cron.sh's command line: `$CC_HOME/scripts/palace-cron.sh --palace $CC_HOME/palace
--diary-root $CC_HOME/diary-root --kg $CC_HOME/palace/knowledge_graph.sqlite3 --palace-bin
$CC_HOME/bin/palace --send $CC_HOME/bin/notify-local.sh --page $CC_HOME/bin/notify-local.sh
--state $CC_HOME/state/cron --judge <judge> --retries 1 --retry-delay 60 --canaries
$CC_HOME/share/canaries.json --wing <wing> --person <person> --assistant <assistant>`
(docs/RUNBOOK.md section 3, step 5; `--wing`: the rollup is mined into this memory's wing; the
names go to every `palace` command of the run). The scheduler is reached only through
`common/schedule-<os>.sh` (launchd on macOS; an OS without one is refused).

- The settle job: each SessionEnd hook appends a line (`<source>`, a tab, the transcript's path)
  to `$CC_HOME/state/settle.request` and exits; launchd starts settle.sh, which waits until the
  file has been still for `SETTLE_DEBOUNCE_SECONDS` (60), takes the requests and
  distills each transcript's new text (`palace distill`: the user's and the assistant's
  messages only, never tool calls or tool output, copied first to
  `$CC_HOME/transcripts/<wing>/<source>/<session>.jsonl`, then filed by `--llm` as diary
  entries, typed claims and graph relations; fewer than `DISTILL_MIN_TURNS` (3) exchanges files
  nothing; positions in `state/distill.positions`). Each claim is a *specific* (an exact date,
  name, number, decision or quote: it carries a verbatim quote and a `<copy>:<line>` locator,
  and a changed value replaces the old one) or a *concept* (an idea, a reason, a lesson). When
  no diary entry is new since the last settle run (`state/last-settle`), it stops with
  `SETTLE QUIET`.
  Otherwise it mines `$CC_HOME/diary-root` into the wing and each `$CC_HOME/wings/<wing>` into
  its own (as env.sh's `AGENT`; a mine refused because another process holds the palace's lock
  is tried twice more, 30 s apart), then runs palace-cron.sh: watch and improve --auto, the asks
  only inside `ASKS_WINDOW` (08:00-21:00 local; outside it they wait), never the digest. It
  adds `--rollup --llm <llm>` when consolidation is due: `SETTLE_COMPRESS_ENTRIES` (40) entries
  since the last one, or `SETTLE_COMPRESS_MAX_DAYS` (3) days and at least one new entry
  (`state/last-compression`, the first run starting the count). Nothing is recorded unless
  palace-cron.sh succeeds, so the next trigger or the daily run retries a failed run. A rollup
  never restates a specific: it copies the value and cites the claim's id, or fails its
  validation. After each distill and each rollup, `palace fidelity` measures what the
  compression kept (specifics by exact match, up to `FIDELITY_SAMPLE` (10) concepts by the
  `--judge`), records it in the claim store and pages on a dropped specific, a contradiction
  or concept recall under `FIDELITY_FLOOR` (0.7); the digest carries one `F:` line. It never
  changes the memory. After a distill it samples only messages of `DISTILL_RECALL_MIN_WORDS`
  (8) or more words, shows the judge each with up to `DISTILL_RECALL_FOLLOWING` (6) later
  messages so it judges what the conversation finally settled, leaves out messages with no
  lasting content, and only logs: it never pages (env.sh may set either value).
- The daily job (05:00): the same settle work when anything is pending (a request, a
  transcript with text not yet distilled, or entries new since the last settle run: a backstop
  for a crash or a closed laptop), then the asks and the digest. A Mac asleep at 05:00 runs it on waking.

The per-turn hook prints `palace context --layered` (`[SPECIFICS]` when the prompt asks for an
exact fact, `[GRAPH]`, `[THREAD]`, `[NEWER]`); env.sh's `CONTEXT_LAYERED=0` gives the earlier
two blocks (`[GRAPH]`, `[PALACE CONTEXT]`).

The settings live in `$CC_HOME/env.sh`; a later install keeps the values found there. One run at
a time: settle.sh's lock (`state/settle.lock`) and palace-cron.sh's beside the store. Asks, the
digest and failures go to `$CC_HOME/inbox.md` with a macOS notification; the SessionStart hook
shows the lines not yet shown, once. Logs: `$CC_HOME/logs/settle.out`, `.err`, `daily.out`,
`.err`, `settle-mine.log`, `settle-distill.log`, `settle-fidelity.log`. An install over an earlier
one migrates the claim store once (every claim a concept), keeping a copy beside it
(`palace/compiler.sqlite.before-memory-types`).

## The rules every script keeps

- `--dry-run` prints each change (`would: …`) and makes none; a real run prints each change
  (`change: …`) before making it and logs it in `$CC_HOME/install.log`.
- A second run changes nothing and says so (`<script>: nothing to change`).
- A file it edits that is not its own is first copied to `<file>.cc-backup-<time>`, and the
  edit is recorded in `$CC_HOME/state/edits`. Its own files (under `$CC_HOME`, and the two
  plists) are replaced when they differ.
- `set -eu`, POSIX sh (also run under dash), ShellCheck-clean (`shellcheck -x`).

## Uninstall

```
$CC_HOME/dist/common/uninstall.sh --dry-run
$CC_HOME/dist/common/uninstall.sh            # keeps $CC_HOME/palace, diary-root, wings and inbox.md
$CC_HOME/dist/common/uninstall.sh --purge    # removes all of $CC_HOME too
```

Boots out and removes the agents (through the scheduler adapter). Each edited file that is unchanged since the last install
gets its bytes from before the first install back from the first backup (or is removed, when
the installer created it), provided each later install's backup equals what the install before
it wrote; a file changed since, or between installs, loses only what was added (the hooks under `$CC_HOME/hooks`, the approval
tables and the servers recorded in `$CC_HOME/install-state`, the marked block; for Claude Code,
settings.json's hooks under `$CC_HOME/hooks` and its recorded `permissions.allow` entries, the
recorded user servers by `claude mcp remove -s user`, the block in CLAUDE.md). A server or
permission the installer did not add, whatever its name, is never removed. Backups are kept.
One uninstall serves both installers (`$CC_HOME/codex-env.sh`, `$CC_HOME/claude-env.sh`).

## Not verified yet

Checked against stand-ins only: the repository's acceptance test (not shipped), with the real
MemPalace 3.5.0 in fake homes and a stand-in model, ends `SETTLE1C OK`. A test with real Codex in a fresh account comes
next. Shown with real Codex 0.154 before: the UserPromptSubmit hook's JSON reaches the model;
MCP tools need the approval tables; `--ignore-user-config` alone leaves the account's apps on
(hence codex-llm.sh's four `--disable` flags). Still open: SessionStart's JSON as context;
whether Codex accepts the appended `[mcp_servers.<server>.tools.<tool>]` tables next to what
`codex mcp add` writes, and whether `codex mcp remove` drops them; codex-llm.sh's `--disable
hooks` and `-c web_search="disabled"` (names not in `--help`), whether `$CODEX_HOME/AGENTS.md`
still reaches the nightly model, and that only the last message reaches stdout; where Codex
keeps hook trust; `palace_diary_write` and `palace_reply` called by real Codex. Claude Code
(2.1.280, flags from its `--help` only): that settings.json's hooks print context the model sees
and the permissions let the tools run without asking; that `CLAUDE_CODE_SAFE_MODE=1` alone keeps
CLAUDE.md out of the nightly `claude -p` and that the run still uses the login of
`CLAUDE_CONFIG_DIR`; whether Claude Code asks to review the new hooks. The settle runs: that
launchd starts the settle job again when the request file changes during a run (the tests
play launchd's part); that both tools run the SessionEnd hook when a session ends (Codex also
after 30 minutes idle) and pass `transcript_path`, Codex within its 3 s.

Known limits: `cc-memory`'s own `mempalace_diary_write` stores a drawer but writes no diary
file, so the nightly job never sees it: the AGENTS.md block names `palace_diary_write` instead,
and the installer leaves the former unapproved. A lock left by a crashed run: each later
run pages `CRON RETRY stage=lock`, and `$CC_HOME/logs/daily.err` names the one `rm -r` to
run once no run is active.
