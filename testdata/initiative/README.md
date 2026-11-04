# Synthetic INIT-1c fixtures

All people (Elora and Niven), dates, tasks, facts and phone numbers here are invented.
The file-only delivery stand-in records argv and stdin in its copied CC_HOME; it
has no sender or network operation. The model is a scripted reply, not a model.

Run `dist/test/initiative.sh`. It puts all scratch, command logs and build output
under `target/init1c/`. `PALACE_INITIATIVE_BIN` may name an already built binary.

To wire a sweep later, install initiative.sh and initiative.py together under
CC_HOME/bin and the prompt under CC_HOME/prompts. Set CC_HOME explicitly or use
the installed script's parent directory. Supply --transcript (or the environment
variable INITIATIVE_TRANSCRIPT) for exactly this person; it never guesses from a
shared position file. The default store is CC_HOME/palace/compiler.sqlite.
Model specs are table:<one JSON reply> or command:<stdin/stdout executable>.
An operator can supply a model adapter with command:; no model is selected by default.
The deliver executable defaults to CC_HOME/bin/palace-cron-deliver and is called
as `send --target <number>` with message text on stdin. --deliver overrides that
executable for adapters/tests. --palace-bin, --store, --now and --tz are also
explicit overrides. No command string is shell-evaluated.

initiative.sh defaults to --dry-run. Both dry and send reserve the same
[person,item,pokes] key before delivery and advance backoff afterwards. Failed or
uncertain sends retain their reservation: manual investigation is required to
retry one. A model decline and an empty candidate set defer four hours. Errors
only enter logs/initiative.log; an allowlist refusal exits 77. Python 3.9+ with
standard-library zoneinfo and an IANA timezone database is required. No schedule
or live configuration is installed by this change.
