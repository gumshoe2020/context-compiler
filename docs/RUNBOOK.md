# RUNBOOK — context-compiler after M5

For the operator (Fergus) and for a reader who has never seen the system. Every command marked
with an output below was run by the builder on copies of the fixtures under `target/runbook/`
(2026-09-29, M5b-4), `target/runbook5/` (2026-09-29, M5b-5), `target/runbook6/` (2026-09-29,
M5b-6: §3 step 6; NOTES.md), `target/runbook7/` (2026-09-29, M5b-7: §2's signal case, §3
step 6's hand run, §11's stderr lines) and `target/runbook8/` (2026-09-29, M5b-8: §2's process
group case, §3 step 5's contract check and the job's PATH), and the quoted line is
its real output; paths are shortened to `<…>`, and reply tokens and event ids differ per run. A step that touches a live scheduled job or the backup
cannot be rehearsed on a copy: it says **not rehearsed** and gives the command as documented
(`openclaw automations --help`, `restic --help`).

Names used below: `<release>` is the pinned checkout the jobs run (today
`~/.openclaw/palace-shadow/release`); `<palace>` the live palace directory, `<kg>` its
`knowledge_graph.sqlite3`, `<diary-root>` the parent of the live `diary/`; the claim store is
always the `compiler.sqlite` beside `<kg>`.

## 1. What runs when (America/New_York)

Until the flip (the old path is in charge; M5-CUTOVER-PLAN Phase 1):

| time | job | what |
|---|---|---|
| 04:20 | d023e326 | prep: builds the compression manifest |
| 04:30 every third day | 912c690b | compression agent turn (the old rollup) |
| 05:00 | b4a8f9a9 | post: applies relations through the TypeScript substrate |
| 05:10 | ca78d337 | Python fidelity watcher |
| 05:30 | 4269b784 `palace-shadow-nightly` | `workspace/scripts/palace-shadow-nightly.sh` → `<release>/scripts/palace-shadow.sh` on nightly copies; observes only; log `~/.openclaw/palace-shadow/logs/` |

After the flip: the four old jobs are disabled (not deleted) and one command job runs
`<release>/scripts/palace-cron.sh` (section 3). Its time and the compression cadence are set at
the flip; the proposal is 04:30 daily, with `--rollup --llm …` on the old job's every-third-day
nights (a second job, or the same job's command on those days).

## 2. Summary lines and what to do

### `SHADOW …` (palace-shadow.sh; until the flip)

| line | exit | meaning | do |
|---|---|---|---|
| `SHADOW OK date=<d> rollups=<n> agree=<n> …` | 0 | old and new agree on every fresh rollup | nothing |
| `SHADOW QUIET date=<d>` | 0 | nothing fresh on either side | nothing (the first night, 2026-09-29, printed `SHADOW QUIET date=2026-09-29`) |
| `SHADOW DIFF date=<d> disagreements=<n> first="…"` | 1 | old and new disagree | read `<night>/REPORT.md` and `report.json`; find the cause before the flip |
| `SHADOW UNCOMPARED date=<d> reason="…"` | 1 | a fresh rollup or canary was not compared (no fresh Python report, a missing judge row, unclassified names) | read the reason; not agreement |
| `SHADOW UNAVAILABLE date=<d> step=<lock\|copy\|space>` | 3 | retry later: a lock held, a source busy, a slow copy, too little free space | the nightly wrapper retries three times, then sends one line; if `step=lock` persists, see section 6 |
| `SHADOW BROKEN date=<d> step=<s> exit=<n> …` | 2 | a step failed, or a signal (`step=signal`) | read `<night>/copy.err` or `<night>/steps/<s>.err` |

### `CRON …` (palace-cron.sh; after the flip)

The line is the last thing the wrapper prints. `--send` (Callum's channel) only ever receives an
ask line or the digest; `--page` (Fergus's) only ever receives a RETRY, BROKEN, ROLLUP INVALID or
UNDELIVERED line. Each stage's output of the last run is in `<state>/last/<stage>.out` and `.err`.

| line | exit | meaning | do |
|---|---|---|---|
| `CRON OK asks=<n> digest=<0\|1> failed=<k>` | 0 | `n` asks delivered and marked sent; the digest delivered or not; `k` runs of `--send` this run that did not deliver (exit not 0, or stopped after `--send-timeout`) | `failed=0`: nothing. `failed=1` or more: the job's stderr has `palace-cron: --send failed (exit <n>); left for the next run` per failure; what failed waits and is tried again next run. From the second such run in a row you are paged (next row) |
| `CRON QUIET` | 0 | nothing to send and nothing failed | nothing |
| `CRON ROLLUP INVALID exit=4 kept=<state>/rollup-attempts` (page only) | — | a compression night's rollup failed its validation on every attempt (the first write, then repairs of the failed claim lines or one rewrite of the whole; three at most); nothing was written to the diary, the graph or the store; the night went on with watch, auto, asks and digest | read the kept attempts (below), nothing else: the next compression night selects and writes again |
| any line ending ` rollup=invalid` | that of the rest of the night | the night's rollup failed its validation (the page above); the rest of the line is the rest of the night, and a night with nothing sent is `CRON OK asks=0 digest=0 failed=0 rollup=invalid`, not `CRON QUIET` | the row of the line without ` rollup=invalid`; for the rollup, the row above |
| `CRON UNDELIVERED failed=<k> exit=<n> runs=<r>` (page only) | — | `--send` failed on `r` runs in a row (sent from `r=2` on, every run while it lasts); `n` is the last failure's exit code; names no ask and no digest | the delivery command is broken: read the job's stderr, run `--send` by hand with a test line, fix it. The first run whose deliveries all succeed resets the count (`<state>/send-failures`) and delivers what waited |
| `CRON RETRY stage=<s> tries=<n>` | 3 | stage `s` said "retry later" (exit 3: MemPalace unavailable, a database locked) on all `n` runs (`--retries` + 1), or the lock was held (`stage=lock`) | paged. Run again later; if it repeats, read `<state>/last/<s>.err`. `palace rollup` also maps a failing `mempalace mine` to 3, so three RETRY nights in a row on `stage=rollup` mean a bad input, not a busy server. `stage=lock` again and again: section 6 |
| `CRON BROKEN stage=state exit=2` | 2 | `--state` (or the lock beside the store) cannot be written: a read-only or full disk, wrong ownership or mode | paged at once (no retries). The job's stderr names the path (`palace-cron: cannot create <state>/last`, `cannot create the lock …`); fix the directory, run once by hand |
| `CRON BROKEN stage=<s> exit=<n>` | 2 | stage `s` exited `n` (anything but 0, 3, and watch's 1); later stages did not run. `stage=arguments`: bad arguments, `--state` missing included (not paged); `stage=signal`: the job was killed (SIGTERM 143, SIGINT 130, SIGHUP 129) | paged. Read `<state>/last/<s>.err` (`rollup` exit 4 is not here: it is `CRON ROLLUP INVALID`, above) |

Rehearsed (fixtures plus the claims rollup of fixtures/improve; stand-in `--send` and `--page`):

```
$ scripts/palace-cron.sh --palace <copy>/palace --diary-root <copy>/diary-root \
    --kg <copy>/palace/knowledge_graph.sqlite3 --palace-bin target/release/palace \
    --send <send> --page <page> --state <copy>/state
CRON OK asks=0 digest=1 failed=0     (first run; --send was given "rule:auto: 2 weakened, 1 expired, 1 re-grounded" / "queue=0 asks=0 unsent=0")
CRON OK asks=1 digest=0 failed=0     (second run; --send was given the ask "🎩 C13 in ROLLUP 09-22: claim "Callum booked a zeppelin tour of Friedrichshafen" — no source found. Reply: source: <file> / drop / fix: <text> #f_bce4fafc"; no event, so no digest)
CRON QUIET                           (third run; <state>/last/auto.out: "AUTO events=0 asks=0 skipped=4 vetoed=0", "SKIP 2026-09-22 C6 #f_6780becd repeat of e_5902d4e302c5", the same for C11 and C9, "SKIP 2026-09-22 C13 #f_bce4fafc unsupported: ask open")
CRON QUIET                           (plain fixtures)
$ … --palace-bin /usr/bin/false
CRON BROKEN stage=auto exit=1        (exit 2; --page was given "CRON BROKEN stage=auto exit=1": watch's exit 1 means "findings", so the first failing stage was auto)
$ …      (a --send that prints "send: registration expired" and exits 1; two runs after the first)
CRON OK asks=0 digest=0 failed=1     (stderr: "palace-cron: --send failed (exit 1); left for the next run"; not paged)
CRON OK asks=0 digest=0 failed=1     (--page was given "CRON UNDELIVERED failed=1 exit=1 runs=2")
$ …      (<copy>/state with mode 555)
CRON BROKEN stage=state exit=2       (exit 2 at once; stderr "palace-cron: cannot create <copy>/state/last"; --page was given the same line)
```

**A night with `CRON ROLLUP INVALID`: read the kept attempts, nothing else.** Each attempt of the
failed rollup is kept in `<state>/rollup-attempts` (0700; files 0600: they hold diary text) as
`<as-of>-<thread>-attempt-<n>.md`, the body as written, and `.json`, its validation report:
`problems` (every problem), `claim_problems` (for each claim whose quote failed: the claim line,
its source and `anchors`, the words of which the quote had to carry one), `attempt` and `mode`
(`write`, `repair` or `rewrite`). The 20 newest runs are kept; pruning removes only files of that
form (`<YYYY-MM-DD>-<thread>-attempt-<n>.md|json`), never another file in the directory. Nothing was written to the diary,
the graph or the store, and the night's other stages ran. Rehearsed on a copy under sh and dash
(target/m5b10/runbook-sh, -dash: three entries dated today, a stand-in writer through `--llm
command:<exe>` whose body is too short, no judge, stand-in `--send` and `--page`):

```
$ scripts/palace-cron.sh … --rollup --llm command:<writer>
CRON OK asks=0 digest=0 failed=0 rollup=invalid     (exit 0; --page was given "CRON ROLLUP INVALID exit=4 kept=<copy>/state/rollup-attempts"; watch, auto, asks and digest ran)
$ grep '^rollup: validate\|^rollup: kept' <state>/last/rollup.err
rollup: validate attempt=1 fail claims=0 relations=0 mode=write repaired=0
rollup: validate attempt=2 fail claims=0 relations=0 mode=rewrite repaired=0
rollup: kept 2 attempt(s) in <copy>/state/rollup-attempts
$ ls <state>/rollup-attempts
2026-09-29-cron-probe-attempt-1.json
2026-09-29-cron-probe-attempt-1.md
2026-09-29-cron-probe-attempt-2.json
2026-09-29-cron-probe-attempt-2.md
$ jq -r '.attempt, .mode, (.problems[])' <state>/rollup-attempts/2026-09-29-cron-probe-attempt-2.json
2
rewrite
size 66 chars; must be 2000–40000
missing section `## Claims`
…                                  (four more missing sections, "0 claims; at least 3 required", "`## Supersedes` resolves no source", "missing ```json relations block")
```

A rollup whose only problems are quotes (the case measured on 2026-09-29) is repaired, not
rewritten, and its kept report names each failed claim. From the M4b doctor's copy (a claim
whose quote names only its subject, repaired twice with the same line):

```
$ jq -r '.attempt, .mode, (.problems[])' <keep dir>/2026-09-22-halloran-thread-attempt-3.json
3
repair
C6: quote does not support claim
$ jq -c '.claim_problems[]' <keep dir>/2026-09-22-halloran-thread-attempt-3.json
{"anchors":["almanack","31 12"],"claim":"C6","line":"- C6. Callum | quoted | Almanack 31:12 in his round-one comment — 2026-09-21 — 2026-09-21-e9767460dfe3.md — quote: \"Callum will report if brackenridge replies\"","problem":"C6: quote does not support claim","source":"2026-09-21-e9767460dfe3.md"}
```

A repair changes a claim's quote and nothing else (M5b-11): a corrected line whose number,
subject, predicate, object, date, source or marker differs from the line asked for (compared
trimmed, in any case) is refused, and the claim stays as written. So a claim that no quote of its
source supports ends the rollup with exit 4, and the kept report shows its line and the words its
quote lacked: a person decides whether the claim or the source is wrong. Rehearsed on a copy
(target/m5b11/rb2: the same first write, then a stand-in whose two repairs turn C6's object into
"Luke 13:4 in his round-one comment" with a quote that would support it):

```
$ palace rollup … --thread halloran-thread --dry-run --llm command:<writer> --keep <keep dir>
(exit 4)
$ jq -r '.attempt, .mode, (.problems[])' <keep dir>/2026-09-22-halloran-thread-attempt-3.json
3
repair
C6: quote does not support claim
repair: C6 changed its object
repair: C6 was not returned
```

A finding the watch reports again on a claim whose automatic verb still stands (not undone; the
claim's status and confidence unchanged since) is not acted on again: no event, no diary entry,
no digest (`SKIP … repeat of e_…` in `auto.out`). An ask is marked sent only after `--send` exits
0. A delivered ask's token is written to `<state>/sent-unmarked` before `--mark-sent` and removed
after, so a mark that fails is paged and finished on the next run without a second send. The
digest covers everything since the last delivered digest (`<state>/digest-since`, UTC; the last
24 hours on the first run).

**The contract of `--send` and `--page`:** exit 0 only after the text is delivered; deliver
nothing after being ended (TERM or HUP ends it without delivering). What the wrapper guarantees
at a timeout: a `--send` or `--page` that runs past `--send-timeout` counts as failed, its whole
process tree is frozen and then ended with KILL (no process of it runs again), and the text is
sent again by the next run. The same holds when SIGTERM, SIGINT or SIGHUP is sent to the
wrapper's own process (its pid) while a `--send` or `--page` runs: that command's tree is frozen
and ended with KILL, even if it would finish its delivery on TERM; the run ends `CRON BROKEN
stage=signal exit=<n>` and the text waits for the next run. A running palace stage is ended with
TERM.

A signal to the job's whole process group (a scheduler that signals the group, a closed
terminal) is different: it reaches the running `--send` or `--page` directly, at the same moment
as the wrapper, before the wrapper can freeze it. The command itself must then stop without
delivering, as the contract says. A command that finishes its delivery on TERM or HUP then
delivers, the wrapper counts it as not delivered, and the next run sends the text again: it
arrives twice. The OpenClaw scheduler does signal the whole group. The installed docs do not say
so (`automation/cron-jobs/payloads.md` says only "bounded process cleanup"); OpenClaw 2026.9.6
was measured on 2026-09-29 with a test job whose parent and child both ignore TERM and record the
time (section 3, step 5, "The time limit"). So the real commands are checked once against the
contract before the job is created (section 3, step 5).

Delivery is at least once. A signal or a timeout that lands after the command delivered but
before the wrapper recorded it (the moment around the command's exit, plus any time the command
spends between delivering and exiting) makes the next run send the text again. The Opus 5.5
review of M5b-7 measured that moment at about 3 to 5 ms after the exit (not rehearsed here). An
ask is never marked sent without a delivery: its token is recorded only after `--send` exits 0.

A command that hands the text to a process outside its own tree (a daemon such as the Gateway, a
detached child) cannot be recalled that way: it delivers at least once, and a timeout or a signal
can then deliver the text twice. A SIGKILL to the wrapper with no signal before it cannot be
handled: a delivery running then can still finish (and is sent again by the next run), and the
lock is left (section 6). Rehearsed on a copy, under sh and dash: TERM to the wrapper's pid one
second after a `--send` started that appends its text to a record on TERM:

```
CRON BROKEN stage=signal exit=143    (exit 2; --page was given the same line; the record empty 2 s later; no lock, no process left)
CRON OK asks=1 digest=1 failed=0     (the next run, a working --send: the digest held at the TERM delivered once, and the ask)
```

And TERM to the wrapper's whole process group one second after the digest's `--send` started,
first with a `--send` that keeps the contract (TERM ends it before it delivers), then with one
that delivers in its TERM handler (target/runbook8, under sh and dash):

```
CRON BROKEN stage=signal exit=143    (keeps the contract: exit 2; nothing delivered during the run; one page; no lock, no process left)
CRON OK asks=1 digest=1 failed=0     (the next run: the digest delivered once in all)
CRON BROKEN stage=signal exit=143    (delivers on TERM: exit 2; one page; no lock; but the digest was delivered during the run)
CRON OK asks=1 digest=1 failed=0     (the next run: the digest delivered again, twice in all)
```

## 3. The flip (Phase 2; Callum's go; steps 4 to 7 in one sitting)

**What switches the corrections path.** The `conversation-corrections` skill decides "flipped" by
one test (its step 1): `test -f "<palace>/compiler.sqlite" && echo flipped`. While the file is
absent it holds each correction as a diary entry tagged `correction-pending-flip`; once it exists
it applies corrections with `palace improve` to the new store. The file first appears at the
first `palace-mcp` write (step 4, as soon as the substrate writes a relation through it) or at
the first wrapper run (step 6), whichever comes first; the warm-up call of step 4 does not create
it. Rehearsed on a copy:

```
$ ls <copy>/palace | grep compiler          (before, and again after the step 4 warm-up)
                                            (nothing, both times)
$ printf '%s\n' '<initialize>' '<notifications/initialized>' '<tools/call mempalace_kg_add {"subject":"runbook probe","predicate":"tested_by","object":"fergus"}>' | \
    target/release/palace-mcp --kg <copy>/palace/knowledge_graph.sqlite3 --palace <copy>/palace
{"id":2,"jsonrpc":"2.0","result":{"content":[{"text":"{\"confidence\":1.0,\"created\":true,\"fact\":\"runbook probe → tested_by → fergus\",…
$ ls <copy>/palace | grep compiler
compiler.sqlite
compiler.sqlite.lock
```

1. **Snapshot** — not rehearsed:
   `restic -r <repository> backup --tag m5-flip <the paths of the pre-m5 snapshot>`
2. **Disable the four old jobs** (do not delete) — not rehearsed:
   `openclaw automations disable 912c690b…`, and the same for `d023e326…`, `b4a8f9a9…`,
   `ca78d337…` (full ids from `openclaw automations list --all`); disable the shadow job
   `<session id>` too. Archive the shadow store
   (`~/.openclaw/palace-shadow/store/`); it is not promoted: the live store starts empty.
3. **Move the pinned release** to the accepted commit (section 9) and build nothing there.
4. **Point the substrate's write transport at `palace-mcp`** (`PGS_WRITE_TRANSPORT` in the
   substrate's environment) — not rehearsed. From its first write the store exists and the
   skill is flipped (above). Warm-up call, rehearsed on a copy:
   ```
   $ printf '%s\n' '<initialize>' '<notifications/initialized>' '<tools/list>' | \
       target/release/palace-mcp --kg <copy>/palace/knowledge_graph.sqlite3 --palace <copy>/palace
   {"id":1,"jsonrpc":"2.0","result":{"capabilities":{"tools":{}},"protocolVersion":"2024-11-05","serverInfo":{"name":"palace-mcp","version":"0.1.0"}}}
   ```
   then eight tools: `mempalace_kg_add`, `_query`, `_invalidate`, `_timeline`, `_stats`,
   `_entities`, `_set_entity`, `_retire_entity`. The write-log check comes after step 6: before
   the store exists it can only fail, and this output is expected:
   ```
   $ palace doctor --verify --kg <copy>/palace/knowledge_graph.sqlite3 --roots <copy>/palace
   palace: input <copy>/palace/compiler.sqlite: No such file or directory (os error 2)      (exit 1)
   ```
5. **Create the new job** — not rehearsed (the command it runs was, section 2).

   First, once, outside the job and before it is created: **check the real `--send` and `--page`
   against the contract** (section 2). Start each with a test line on its standard input, send
   it TERM while it runs, and again HUP; nothing may arrive. Tell Callum first: a `--send` that
   fails the check delivers the line `contract test` to him. For TERM:
   ```
   $ printf 'contract test\n' | <send command for Callum> & pid=$!; sleep 1; kill -TERM "$pid"; wait "$pid"; echo "exit $?"
   ```
   then the same with `kill -HUP`, then both again with `<page command for Fergus>`. Wait a
   minute and look at what arrived. Rehearsed on a copy (target/runbook8, under sh and dash)
   with two stand-ins that deliver 3.6 s after they start; with the real commands not rehearsed:
   ```
   exit 143          (a stand-in that keeps the contract, TERM; with HUP: exit 129; nothing arrived 6 s later)
   exit 0            (a stand-in that delivers in its TERM and HUP handler, both signals; "contract test" arrived at once)
   ```
   Before `exit 143` or `exit 129` the shell prints its own report of the ended command
   (`Terminated: 15` or `Hangup: 1`; bash adds the pipeline). The check passes when nothing
   arrived after either signal. `exit 0` after the signal means the command went on and
   delivered. If `kill` says there is no such process, the command had finished (and delivered)
   before the signal, and the check tested nothing: repeat it with a shorter pause (`sleep
   0.2`). A command that fails the check is fixed to exit without delivering on TERM and HUP, or
   kept knowing that a signal to the job's process group can then deliver a text twice.

   Then the job: a command job with no delivery (`--no-deliver`): the installed OpenClaw docs
   (`cli/cron.md`) say isolated jobs default to `--announce` and command jobs use the same
   delivery modes, so without it the `CRON …` line (or, after a kill, stderr) would be posted to
   chat every night. Syntax as documented there:
   ```
   openclaw automations create "30 4 * * *" --name palace-cron \
     --command-argv '["<release>/scripts/palace-cron.sh","--palace","<palace>","--diary-root","<diary-root>","--kg","<kg>","--palace-bin","<release>/target/release/palace","--send","<send command for Callum>","--page","<page command for Fergus>","--state","<state dir>","--judge","claude-cli:<Sonnet 5 model>","--retries","1","--retry-delay","60"]' \
     --command-env "PATH=<the directories of claude, mempalace and mempalace-mcp>:/usr/bin:/bin" \
     --timeout-seconds <limit> --no-deliver
   ```
   (on compression nights also `"--rollup","--llm","claude-cli:<Fable 5.1 model>"`). `--send`
   and `--page` are executables that deliver their standard input (the wrapper names no
   channel), under the contract of section 2: exit 0 only after the text is delivered, nothing
   delivered after being ended. At a timeout, and when the wrapper's own process is signalled
   while one runs, the wrapper ends the command's process tree with KILL and the next run sends
   the text again; a signal to the job's whole process group reaches the command itself, which
   must then stop without delivering (the check above). A command that hands the text to a daemon
   or a detached child delivers at least once, and a timeout or a signal can then deliver it
   twice. `--state <state dir>` is
   required, and step 6's hand run gives the same one; one run per store whatever the state: the
   lock is `<kg dir>/.palace-cron.lock`. **The state directory must lie outside the diary root and
   the palace** (symbolic links resolved; not equal to either): each commit runs `mempalace mine
   <diary-root>`, which would make the kept attempts in `<state>/rollup-attempts` memory, and
   `palace rollup --keep` refuses the same places. The wrapper refuses such a `--state` before it
   creates anything. Rehearsed on a copy (target/m5b11/rb4, stand-in `--send` and `--page`, the
   same under sh and dash):
   ```
   $ scripts/palace-cron.sh --palace <copy>/palace --diary-root <copy>/diary-root … --state <copy>/diary-root/state
   CRON BROKEN stage=arguments exit=2      (exit 2; stderr "palace-cron: --state <copy>/diary-root/state is inside --diary-root <copy>/diary-root; it must stay outside the diary root and the palace"; nothing created)
   $ … --state <link to the palace>/state
   CRON BROKEN stage=arguments exit=2      (exit 2; stderr "… is inside --palace <copy>/palace; …")
   ``` Set no "no output" limit
   (`--no-output-timeout-seconds`): the wrapper prints once, at the end.

   **The job's environment** — not rehearsed with the scheduler. The wrapper's stages need
   `mempalace` and `mempalace-mcp` on PATH, and the judge needs `claude` (`palace` looks for it
   on PATH, then in `$HOME/.local/bin`). The installed docs give `--command-env KEY=VALUE`
   (repeatable; `cli/cron.md`, `automation/cron-jobs/payloads.md`) and do not say what
   environment a command job otherwise gets, nor whether `--command-env` adds to it or replaces
   it; a Gateway started by launchd may have a short PATH. Measured on this install on
   2026-09-29 (OpenClaw 2026.9.6, the test job of "What the scheduler does at the limit", no
   `--command-env`): the job had HOME set and a PATH holding `/usr/bin`, `/bin`,
   `/opt/homebrew/bin` and `$HOME/.local/bin`, on which `claude`, `mempalace` and `mempalace-mcp`
   were all found. That is this machine on that day, not a rule. So the job is created with an
   explicit PATH, as above: the absolute directories that `command -v claude mempalace
   mempalace-mcp` prints in the operator's shell, then `/usr/bin:/bin` (the wrapper's own tools,
   `pgrep` included). A hand run in a terminal (step 6) uses the operator's environment and does
   not prove the job's. Rehearsed on a copy: the wrapper with a PATH that has a stand-in `claude`
   but no `mempalace` (target/runbook8, under sh and dash; `--retry-delay 1` instead of 60):
   ```
   $ PATH=<copy>/bin:/usr/bin:/bin scripts/palace-cron.sh … --judge claude-cli:claude-sonnet-5 --retries 1 --retry-delay 1
   CRON RETRY stage=auto tries=2        (exit 3; stderr "palace-cron: auto: retry later (exit 3); run 2 in 1 s"; --page was given "CRON RETRY stage=auto exit=3 tries=2")
   $ ls <state>/last; cat <state>/last/auto.err
   auto.err  auto.out  watch.err  watch.out
   palace improve --auto: MemPalace unavailable: No such file or directory (os error 2)
   ```
   The watch ran (its report is written), `improve --auto` could not reach MemPalace, and the
   night is a `CRON RETRY` that counts toward the scheduler's ten (below). After the first
   night, read `<state dir>/last/watch.out`: its `error=0` means every judge call returned. A
   judge that could not run shows only there, as `error=<n>`; `watch.err` does not show it and
   the line is still `CRON OK`. Rehearsed on a copy, the full PATH with a stand-in `claude`
   first, then with one that exits 127 first:
   ```
   palace-fidelity-watch: 11 rollup(s), fail=4 warn=2 error=0 — <state>/watch-reports/<time>.json      (the stand-in judge answered, 7 calls)
   palace-fidelity-watch: 11 rollup(s), fail=4 warn=2 error=5 — <state>/watch-reports/<time>.json      (the failing claude: CRON OK asks=0 digest=1 failed=0, nothing on stderr)
   ```
   `error=<n>` above 0 on the job but not on step 6's hand run: the job's PATH (or HOME) is not
   the operator's; fix `--command-env` (`openclaw automations edit <job-id> --command-env …`,
   not rehearsed; the docs list `--command-env` for `create`, and do not say whether `edit`
   takes it, so if not, remove and create the job again).

   Then check, once, that the job has no delivery — not rehearsed (`automations show` previews
   the resolved delivery route, `cli/cron.md`):
   ```
   openclaw automations show <job-id>
   ```
   It must show delivery `none` (no announce, no channel, no webhook). If it does not:
   `openclaw automations edit <job-id> --no-deliver`, then `show` again.

   In the same `show`, read the job's failure-alert route. The installed docs (`cli/cron.md`,
   "Failure delivery"; `automation/cron-jobs/delivery.md`, "Failure notifications") say a command
   job's non-zero exit, signal or timeout records `error` and can send a failure alert, by default
   after 2 consecutive failures; the route is the job's `failureAlert`, else its
   `delivery.failureDestination` over the global `cron.failureAlert`, else its announce target
   (none here); a global `cron.failureAlert` activates it even without a route. A night cut short
   at the time limit is reported by this alert and by nothing else ("What the scheduler does at
   the limit", below), so the alert is turned on for this job, from the first failure, and
   routed to where the operator's pages go — not rehearsed, flags as `openclaw automations edit
   --help` lists them; then `show` again:
   ```
   openclaw automations edit <job-id> --failure-alert --failure-alert-after 1 \
     --failure-alert-mode announce --failure-alert-channel <the pages' channel> --failure-alert-to <the pages' destination>
   ```
   A night that ends `CRON RETRY` or `CRON BROKEN` is then reported twice, by the wrapper's page
   (which names the stage) and by the scheduler's alert; a night cut short, by the alert alone.
   On an install where pages and asks share one chat (this one, as set up on 2026-09-29; Callum
   approved routing the alert to his chat), both reach Callum, marked as alerts.
   After the first night that ends non-zero (`CRON RETRY`, exit 3, or `CRON BROKEN`, exit 2),
   check the job's next run time in `openclaw automations show <job-id>` — not rehearsed: the
   docs say recurring jobs back off after consecutive errors (30s, 1m, 5m, 15m, 60m) and return
   to the schedule after the next success. A run then is an ordinary run (nothing delivered is
   sent again), but each one that fails pages Fergus again; if they repeat, fix the cause (the
   page names the stage) or disable the job (`openclaw automations disable <job-id>`, as in
   step 2).

   **Ten failures in a row disable the job** — not rehearsed. The installed docs
   (`automation/cron-jobs/delivery.md`, "Failure notifications") say a time-based recurring job
   is auto-disabled after 10 consecutive execution failures, and a successful run resets that
   streak. It holds with no failure alert too: `--no-failure-alert` leaves "the auto-disable
   safety notification" active, and the owning agent receives a notification with the cause and
   the recovery command. Every run that ends `CRON RETRY` (exit 3) or `CRON BROKEN` (exit 2)
   counts (`cli/cron.md`: a non-zero exit, a signal or a timeout records `error`); `CRON OK`,
   `CRON QUIET` and a night with `CRON UNDELIVERED` exit 0 and do not. Whether the backoff runs a daily job again before its next scheduled time the docs do
   not say; if it does, a lasting cause (a stale lock: `CRON RETRY stage=lock` on every run,
   section 6) can reach ten within hours. A disabled job no longer runs and so pages no one, and
   the default list hides it. After fixing the cause, find it and enable it (commands as
   documented in `delivery.md` and `cli/cron.md`; enabling clears the recorded reason and the
   failure streaks):
   ```
   openclaw automations list --all
   openclaw automations enable <job-id>
   ```

   **The time limit.** Command jobs default to 10 minutes. The limit must be above the worst
   case, which with `--retries R --retry-delay D --send-timeout S` is

   worst = (R + 1) × T + (6 + a) × R × D + (a + 4) × S

   - `T`: the hand run of step 6 (`time …`: every stage once, no retries); a stage retried `R`
     times takes at most `R + 1` times its own run.
   - `(6 + a) × R × D`: the waits between retries at the lock, rollup, watch, auto, unsent and
     digest, and at the `--mark-sent` of each of the night's `a` asks.
   - `(a + 4) × S`: each ask's `--send`, the digest's, the ROLLUP INVALID page (M5b-10: a rollup
     that fails its validation no longer ends the night), the UNDELIVERED page and the final
     page, each stopped at `S`.

   With `R=1, D=60, S=60` and a night of at most `a=10` asks: worst = 2T + 960 + 840 seconds.
   For example, if step 6 measures `T = 1200` s (an assumption until measured), worst = 4200 s:
   give `--timeout-seconds 4500`. Recompute when `T` or the flags change.

   Measured on 2026-09-29 on a copy of the live palace and diary (613 diary files, 12 rollups, an
   empty store; the job's flags, judge `claude-cli:claude-sonnet-5`, stand-in `--send` and
   `--page`; the live stores were not written):
   ```
   first night, no rollup        30 s    CRON OK asks=0 digest=1 failed=0     (4 events: 2 weakened, 2 expired; no ask)
   second night, no rollup       33 s    CRON QUIET
   compression night (--rollup --llm claude-cli:claude-fable-5-1)
                                709 s    CRON BROKEN stage=rollup exit=4      (the rollup failed validation twice; paged; later stages did not run)
   ```
   So `T` is about 35 s on an ordinary night, and a compression night is the rollup's time plus
   that: 709 s for a rollup written twice, about 750 s in all. With `T = 750`: worst = 1500 + 960
   + 840 = 3300 s, and `--timeout-seconds 4500` stands. The rollup's own deadlines bound it:
   `--write-timeout` 600 s for each of at most two writes, `--model-timeout` 120 s for each repair
   (M5b-10: at most two, three attempts in all, so at most 600 + 600 + 120 s of writing) and each
   judge call. Since M5b-10 that measured night would end `CRON OK … rollup=invalid` after a page
   `CRON ROLLUP INVALID exit=4 kept=…`, with watch, auto, asks and digest run (section 2). Measure again when the diary has grown much or the models change.

   **What the scheduler does at the limit** — measured on OpenClaw 2026.9.6, 2026-09-29, with a
   one-off command job (`--timeout-seconds 10`, since removed) whose parent and child ignore
   TERM and write the time every 20 ms; check it again after an OpenClaw update:
   ```
   1790719560.235 parent pid=37356 pgrp=37356 start ppid=1866     (the job leads its own process group; its parent is the Gateway)
   1790719560.235 child  pid=37360 pgrp=37356 start ppid=37356
   1790719570.224 child  pid=37360 pgrp=37356 got SIGTERM         (10 s: TERM to parent and child in the same millisecond: the group)
   1790719570.224 parent pid=37356 pgrp=37356 got SIGTERM
   last sign of life: parent 1790719570.516, child 1790719570.535  (SIGKILL to the group 0.3 s after the TERM)
   run history: status error, "command timed out", signal:SIGKILL, durationMs 10350
   ```
   So at the limit the scheduler sends SIGTERM to the job's whole process group and SIGKILL to
   the group 0.3 s later (its code: `killGraceMs`, default 300 ms). Whether a Gateway stop during
   a run does the same was not measured. In those 0.3 s the wrapper cannot page: a page takes
   seconds. Rehearsed on a copy with this install's `--send` and `--page` (TERM to the group,
   KILL to the group 0.3 s later), during a slow watch and during a running `--send`:
   ```
   exit 137, no summary line, no page, the lock released, no process left, nothing delivered
   ```
   The lock was released in time in both rehearsals; a slower machine or a larger process tree
   may leave it (section 6). Nothing is lost: the wrapper starts no delivery after the signal,
   and an ask or digest that was being sent counts as not delivered and is sent by the next run.
   Callum has it once if the command kept the contract, and twice if it delivered on the signal,
   had handed the text to a daemon or a detached child, or the signal came at the moment of its
   exit (section 2). But nobody is told by the wrapper that the night was cut short. Two things
   follow:
   - the limit is set well above the worst case, so that it is reached only when something
     hangs;
   - the job's own failure alert is what reports a night cut short: it is turned on and routed
     (the check after creation, above).

   The wrapper's own handling of a signal (the running stage ended with TERM, a running `--send`
   or `--page` frozen and ended with KILL, the lock released, `CRON BROKEN stage=signal
   exit=143` paged within 5 s and printed) is what happens when the SIGTERM reaches the wrapper
   alone and no kill follows within seconds: a person's `kill <pid>`, not the scheduler. A kill
   with no SIGTERM first cannot be handled: section 2's SIGKILL case, and section 6.
6. **The first run, by hand.** It runs `improve --auto` over every standing finding of the whole
   diary (the store is empty) and sends every ask it raises at once, so read its plan first.
   On a copy of the live palace and diary root (a scratch directory, `MEMPALACE_PALACE_PATH` =
   the copy's palace), with the job's judge, run from `<release>`: the judge reads
   `prompts/judge-settings.json` relative to the working directory, and it must be under
   `--roots`:
   ```
   $ cd <release>
   $ target/release/palace --roots=<copy> --roots=<release>/prompts --roots=<release>/fixtures --roots=<release>/scripts \
       --mempalace-check <release>/scripts/mempalace-check.sh \
       watch --diary <copy>/diary-root/diary --all --store <copy>/palace/compiler.sqlite \
       --reports <copy>/reports --canaries <release>/fixtures/canaries.json --judge claude-cli --model <Sonnet 5 model>
   palace watch: warning: 2026-09-01-a8712aae1c30 names 2026-08-22-53afb95122ad in its Supersedes section outside both the superseded list and every referenced-only region (unclassified)      (stderr; expected, section 11)
   palace-fidelity-watch: 11 rollup(s), fail=4 warn=2 error=0 — <copy>/reports/20260929T175525Z.json      (exit 1: findings)
   $ target/release/palace --roots=<copy> --roots=<release>/prompts --roots=<release>/fixtures --roots=<release>/scripts \
       --mempalace-check <release>/scripts/mempalace-check.sh \
       improve --auto --dry-run --kg <copy>/palace/knowledge_graph.sqlite3 --palace <copy>/palace \
       --diary <copy>/diary-root --store <copy>/palace/compiler.sqlite --judge claude-cli:<Sonnet 5 model>
   AUTO DRY-RUN events=4 asks=0 skipped=0 vetoed=0
   PLAN add 2026-09-22 C11 #f_161e1215 found_source=2026-09-21-c0ad0aed1741.md
   PLAN expire 2026-09-22 C9 #f_64b6f8e1
   PLAN weaken 2026-09-22 C6 #f_bee4c3dd
   PLAN weaken 2026-09-22 C13 #f_f354a03e reason="grounding attempt failed"
   ```
   (rehearsed on the fixtures plus the claims rollup with `<Sonnet 5 model>` = `claude-sonnet-5`
   and a stand-in `claude` first on PATH that answers `{}`: seven calls, each `claude -p --model
   claude-sonnet-5 … --settings <release>/prompts/judge-settings.json`; the dry run wrote no
   event: 0 before, 0 after. Without `--roots=<release>/prompts` watch stops at once: `palace
   watch: prompts/judge-settings.json: input must stay under --roots`, exit 2.) Each `PLAN` line
   is an event the first run makes, each `ASK` line a message to Callum; the judge flags are the
   wrapper's (`--judge claude-cli:<model>` becomes watch's `--judge claude-cli --model <model>`).
   If the plan is not what Callum expects, stop here. Otherwise run the job's command once by
   hand, under `time`, on the live paths, with the job's arguments and the same `--state <state
   dir>` (this run delivers to Callum; the job's first night then carries on from its state);
   check the CRON line, what Callum received, and the write log:
   ```
   $ cd <release>
   $ time <release>/scripts/palace-cron.sh --palace <palace> --diary-root <diary-root> --kg <kg> \
       --palace-bin <release>/target/release/palace --send <send command for Callum> \
       --page <page command for Fergus> --state <state dir> --judge claude-cli:<Sonnet 5 model> \
       --retries 1 --retry-delay 60
   CRON OK asks=0 digest=1 failed=0
   $ MEMPALACE_PALACE_PATH=<palace> <release>/target/release/palace doctor --verify --kg <kg> --roots <palace>
   VERIFY OK chain=ok
   ```
   (The output above is from a copy, not the live palace: fixtures plus the claims rollup,
   stand-in `--send`, `--page` and `claude`, under sh and dash, 3 s; the digest went to the
   stand-in. On the live store expect the plan's `ASK` lines as asks and a digest of its `PLAN`
   events.)
7. **Replay the corrections held for the flip.** Every diary entry tagged
   `correction-pending-flip` is one correction Callum made in conversation while the old path
   was in charge. Use the skill's values (`REL`, `PALACE`, `DIARY_ROOT`, `P`, and
   `ROOTS="--roots $PALACE $DIARY_ROOT $REL/scripts"`, `PIN="--mempalace-check
   $REL/scripts/mempalace-check.sh"`, `MEMPALACE_PALACE_PATH=$PALACE`) and keep a replay log
   (`<state dir>/replay.log`: one line per entry, the entry file and the event id, or why it
   was skipped). Rehearsed on a copy after its first two wrapper runs, with three held entries
   (one tagged `tags:` in lower case, one about a graph relation):
   ```
   $ grep -il 'correction-pending-flip' "$DIARY_ROOT"/diary/*.md
   <copy>/diary-root/diary/2026-09-29-0badc0de0002.md
   <copy>/diary-root/diary/2026-09-29-abad1dea0003.md
   <copy>/diary-root/diary/2026-09-29-f00dfeed0001.md
   $ for f in $(grep -il 'correction-pending-flip' "$DIARY_ROOT"/diary/*.md); do
       printf '%s %s\n' "$(sed -n 's/^\([0-9]\{4\}-[0-9][0-9]-[0-9][0-9] [0-9][0-9]:[0-9][0-9]\).*/\1/p' "$f" | head -1)" "$f"
     done | sort
   2026-09-29 09:40 <copy>/diary-root/diary/2026-09-29-0badc0de0002.md
   2026-09-29 10:05 <copy>/diary-root/diary/2026-09-29-f00dfeed0001.md
   2026-09-29 10:20 <copy>/diary-root/diary/2026-09-29-abad1dea0003.md
   ```
   The order is the time written inside each entry (the file name after the date is a hash, so
   it is not the order). Then, oldest first, for each entry:
   - already in the replay log (`grep -F <entry file> <state dir>/replay.log`): skip it.
   - about a graph relation, not a claim `<date> Cn`: the skill applied it through
     palace-graph-wiring when it was made; log `<entry file> skipped: relation (applied when
     made)` and do not replay it.
   - otherwise check the claim first; it must be live and must not already carry this
     correction by callum:
     ```
     $ $P depends "2026-09-22 C6" --kg $PALACE/knowledge_graph.sqlite3 --roots $PALACE
     DEPENDS c_9a0c69689f0a Callum | quoted | Almanack 31:12 in his round-one comment status=live confidence=0.45
     …
     events: e_7c675963f417 weaken by rule:auto
     ```
     then apply the entry's verb with Callum's words, as Callum, with the roots and pin, and log
     the event id:
     ```
     $ $P improve weaken "2026-09-22 C6" "That is overstated." --by callum --kg $PALACE/knowledge_graph.sqlite3 --palace $PALACE --diary $DIARY_ROOT $ROOTS $PIN
     IMPROVE OK weaken 2026-09-22 C6 event=e_81349b5bc387 reversible_by=e_c7ab943aeba5 to_claim=- entry=2026-09-29-cdbd31156ae5.md
     $ $P improve retract "2026-09-22 C13" "That never happened, drop it." --by callum …      (the next entry, same form)
     IMPROVE OK retract 2026-09-22 C13 event=e_ed6d37452303 reversible_by=e_bb62d4b83467 to_claim=- entry=2026-09-29-c7657a780bc7.md
     $ cat <state dir>/replay.log
     <copy>/diary-root/diary/2026-09-29-0badc0de0002.md e_81349b5bc387
     <copy>/diary-root/diary/2026-09-29-f00dfeed0001.md e_ed6d37452303
     <copy>/diary-root/diary/2026-09-29-abad1dea0003.md skipped: relation (applied when made)
     ```
   A replay is not safe to repeat (a second `weaken` is applied again); the log and the
   `depends` check are what keep an entry from being applied twice. The classifier
   (`$P improve --classify="<his words>" --claim "<date> Cn" --store $PALACE/compiler.sqlite
   --roots $PALACE`) is a second opinion only; Fergus reads the message and names the verb
   (section 11). A wrong one is undone with the event id:
   ```
   $ palace improve undo e_05454588df4c --by callum --kg <kg> --palace <palace> --diary <diary-root>
   IMPROVE OK undo 2026-09-22 C13 event=e_f6164aabd1a1 reversible_by=e_e85e6a081243 to_claim=- entry=2026-09-29-6fcb16693b7d.md
   ```
   (M5b-4 rehearsal; with the skill's `$ROOTS $PIN` when the paths are outside the release.)

## 4. Rollback

The jobs — not rehearsed (live jobs): `openclaw automations disable <the new job>`, then
`openclaw automations enable` each of the four old jobs, and point `PGS_WRITE_TRANSPORT` back.
Then, rehearsed on the copy of section 3 step 7 (flipped, two corrections replayed):

a. **Move the store aside**, with its `-wal`, `-shm` and `.lock`. This is the corrections
   skill's switch: without `<palace>/compiler.sqlite` it holds corrections as
   `correction-pending-flip` entries again. No wrapper run may be active (no
   `<kg dir>/.palace-cron.lock`; section 6).
   ```
   $ mkdir <palace>-store-rolled-back && mv <palace>/compiler.sqlite <palace>/compiler.sqlite-wal <palace>/compiler.sqlite-shm <palace>/compiler.sqlite.lock <palace>-store-rolled-back/
   $ test -f <palace>/compiler.sqlite && echo flipped
                                            (nothing: diary-tag mode)
   ```
b. **List every correction applied since the flip** from the moved store, and re-file each one
   as a `correction-pending-flip` diary entry (the skill's step 1 form: Callum's words verbatim
   in double quotes, the claim, the verb; tags `callum, correction, correction-pending-flip`)
   for the next flip. The re-filing goes through `mempalace_diary_write` into the live diary —
   not rehearsed. The list (the moved store is only read; `sqlite3` without `-readonly`, which
   cannot open a WAL database whose shared-memory file it may not create):
   ```
   $ sqlite3 <palace>-store-rolled-back/compiler.sqlite "SELECT e.at, e.by, e.verb, n.date||' '||n.number, e.text, e.id FROM improvement_events e LEFT JOIN claim_numbers n ON n.claim_id=e.from_claim WHERE e.by IN ('callum','fergus') AND e.at >= '<flip time, UTC>' GROUP BY e.id ORDER BY e.rowid"
   2026-09-29T15:58:27.352668Z|callum|weaken|2026-09-22 C6|That is overstated.|e_81349b5bc387
   2026-09-29T15:58:27.368427Z|callum|retract|2026-09-22 C13|That never happened, drop it.|e_ed6d37452303
   ```
   (`palace improve --digest --since <flip time> --max 1000 --store <moved store>` lists the
   automatic events only — `rule:auto: 2 weakened, 1 expired, 1 re-grounded` here — not these.)
   The flip time is the first event's: `sqlite3 <moved store> "select min(at) from
   improvement_events"`.
c. **Move the pinned release back** (section 9) only if the release itself is the problem.
d. **The wrapper's state directory**: keep it as it is for the next flip, or remove its journal
   and digest window together, never one of them:
   ```
   $ rm -f <state dir>/sent-unmarked <state dir>/digest-since
   ```

What these steps keep: every correction applied after the flip stays in the moved store, with
its hash-chained write log, and is re-filed for the next flip by (b); what the new path wrote to
the graph (`<palace>/wal/write_log.jsonl`) and its `IMPROVE` diary entries stay where they are
and are read by the old path as ordinary graph facts and diary entries. If the next flip moves
the old store back instead of starting a new one, its corrections are already applied: skip the
re-filed entries (step 7's `depends` check shows them). Check the moved store's chain:
```
$ palace doctor --verify --audit <palace>-store-rolled-back/compiler.sqlite
VERIFY OK chain=ok
```

## 5. A job that did not finish

`CRON RETRY` / `SHADOW UNAVAILABLE`: nothing to repair; the next run retries. `CRON BROKEN`:
read `<state>/last/<stage>.err`, fix the cause, run the job once by hand.

## 6. A stale lock

A run killed with SIGKILL (or a lost machine) leaves its lock directory. Neither script ever
breaks a lock by itself; the message says whether the recorded pid is running. Rehearsed:

```
$ scripts/palace-cron.sh … --state <copy>/state --retries 0      (a lock left by pid 99999)
palace-cron: <copy>/palace/.palace-cron.lock is held by pid 99999, not running: if no run is active, remove it: rm -r '<copy>/palace/.palace-cron.lock'
CRON RETRY stage=lock tries=1
$ scripts/palace-shadow.sh … --shadow-root <copy>/shadow …      (the same for the shadow)
palace-shadow: the shadow root's lock <copy>/shadow/.lock is held by pid 99999, which is not running: a stale lock; check that no shadow run is active, then remove it: rm -r '<copy>/shadow/.lock'
SHADOW UNAVAILABLE date=2026-09-23 step=lock
```

Check that no run is active (`pgrep -fl 'palace-cron.sh|palace-shadow.sh'`), then run the
`rm -r` the message gives. After it the next run went through: `CRON QUIET`. The shadow also
has `<store dir>/.store-lock`, handled the same way.

Every run blocked by the lock ends `CRON RETRY stage=lock` (exit 3), and ten failed runs in a row
disable the scheduled job (section 3, step 5). After removing the lock, check the job — not
rehearsed: `openclaw automations list --all` shows it even when disabled; if it is,
`openclaw automations enable <job-id>`.

## 7. A diverged row

A crash between the stores' writes leaves a recovery intent; the next writing command settles
it, or exits 2 naming a DIVERGED row, which only a person settles. Rehearsed (a clean copy):

```
$ palace doctor --check-divergence --kg <kg>
DIVERGENCE none
$ palace doctor --settle i_00000000 --keep-current --by fergus --kg <kg>
palace: refused: i_00000000 is not an unsettled intent          (exit 1)
```

On a real divergence `--check-divergence` prints `PENDING …` and `DIVERGED <table> <key>
<column>` lines (exit 1). Decide per intent: `--keep-current` (the rows stay as they are now) or
`--restore-before` (every row's logged `before` is written back), `--by callum|fergus`; it prints
`SETTLED <intent> <choice> by <who> (<n> row(s))` and writes an audited `recovered_manual` line.
The whole path (a real DIVERGED row settled) is the m4d2 doctor's `settle=ok` gate.

## 8. A broken write-log chain

```
$ palace doctor --verify --kg <kg>
VERIFY OK chain=ok
$ palace doctor --verify --audit <a copy of compiler.sqlite with one write_log row edited>
palace: write_log chain broken at 23          (exit 1)
```

A broken chain means a row of the store's write log was changed or removed after it was
written (the table is append-only by trigger; editing it needs the trigger dropped). Stop the
job (`openclaw automations disable …`, not rehearsed), keep the damaged file for inspection, and
restore `compiler.sqlite` from the latest snapshot that verifies (section 10).

## 9. Moving the pinned release to a new commit

The jobs run binaries from a git worktree of this repository, so work here cannot change what
runs at night. After an acceptance, rehearsed on a worktree under target/:

```
$ git -C <release> checkout --detach <commit>
HEAD is now at 08d4d32 M5b-4 item 2: palace doctor --acceptance m5b4 (…)
$ cp target/release/palace target/release/palace-mcp <release>/target/release/
$ git -C <release> log --oneline -1; <release>/target/release/palace --help | head -1
08d4d32 M5b-4 item 2: palace doctor --acceptance m5b4 (stand
Usage: palace [OPTIONS] <COMMAND>
```

Copy the binaries that passed the acceptance (built from that commit); do not build in the
release. The next night's line is the check.

Copy `target/release`, never `target/release-dist` (PKG-1.2 review L5). The release-dist build
(`dist/build-release.sh`, cargo features `doctors` and `owner-defaults` off) resolves its root
from its own directory, `<release>/target`, so the default `--roots`, prompts and pin script
would be wrong for the hook shadow and the corrections skill, which pass no `--mempalace-check`;
and it has no default names, so every command but `context` and `init` would refuse without
`--person` and `--assistant`. Moving the release past PKG-1.3 also changes the rollup prompt
(prompts/rollup.md, read from the release's checkout): the same steps and rules, invented
examples, no design history; `palace rollup` fills in Callum and Fergus where it names the
person and the assistant.

## 10. Restoring from the `pre-m5` snapshot

Not rehearsed (the backup is outside the project). Snapshot `75c7ddeb`, tags `fergus-record` and
`pre-m5` (Phase 0: 534 diary files and the graph restored byte-identical in the drill):

```
restic -r <repository> snapshots --tag pre-m5
restic -r <repository> restore 75c7ddeb --target <scratch dir> --include <path to restore>
```

Restore into a scratch directory, compare hashes, then stop the jobs and move files into place.
A WAL-mode database restored from backup opens read-write, or read-only with `immutable=1`; a
plain read-only open fails because it cannot create the shared-memory file.

## 11. Known limits

- **The classifier** (`palace improve --classify`) proposes; it is frozen at M5c.3 (119/119 on
  Fergus's oracle) and Fergus reads Callum's message and names the verb. It gets wrong: a
  confirm followed by a qualifier ("That's right, other than the date"), hedged or reported cues
  ("I doubt it's backwards", "Ida says it's backwards"), deferred imperatives ("Once we verify,
  drop it"), a refusal after "No," ("No, keep it, we'll drop it later" → retract), sarcasm,
  idiom, other languages, a negation after a cue. Every applied verb has an undo id.
- **Excerpts differ from the old hook's by design**: `palace context` uses MemPalace's MCP
  search (fifteen candidates, closet boosts, a 250-character query); the hook's sidecar the CLI
  search re-ranked with BM25 (400 characters). Nodes and relations match exactly.
- **A caller that kills `palace context` early** (before `--timeout-ms` + 300 ms) leaves the
  MemPalace server running; a caller's kill limit must be at least `--timeout-ms` + 500 ms.
- **`palace rollup --llm`** is one model for select, write and the rollup's own canary judge;
  `--judge` of the wrapper sets the judge of watch and `improve --auto` only.
- **A failed `--send`** shows as `failed=<k>` in the line and on the job's stderr, and what failed
  waits for the next run; it is paged only from the second failed run in a row (`CRON
  UNDELIVERED`). A `--send` or `--page` run is ended with its process tree (KILL) after
  `--send-timeout` (60 s) and counts as failed; the signal page is bounded to 5 s. A command
  that hands its text to a daemon or a detached child can deliver it twice (section 2).
- **A lock left by SIGKILL** without a SIGTERM first stays until a person removes it (section 6).
  A SIGTERM (the scheduler's time limit) releases the lock before the page, so a KILL that follows
  leaves none.
- **The wrapper prints one line, at the end.** A scheduler "no output" limit shorter than the
  job's time limit would kill every run.
- **Expected lines on stderr.** After a `--send-timeout`, or a signal while a `--send` or
  `--page` runs, the job's stderr carries the shell's report of the command it ended with KILL,
  with no text of the delivery in it; under bash (macOS /bin/sh) `scripts/palace-cron.sh: line
  <n>: <pid> Killed: 9  ( trap - HUP INT TERM; printf '%s\n' "$2" | "$1" ) < /dev/null >
  /dev/null 9>&-`, under dash `Killed: 9` (rehearsed on copies: bash printed it after a timeout
  and after a signal, dash after a timeout only). The watch prints `palace watch: warning: … (unclassified)`
  (a Supersedes mention it cannot classify; in `<state>/last/watch.err` for the job) on the
  fixtures, and may on the live diary: expected, it changes nothing.
