All people, dates and facts here are invented. The unnamed person is Elora;
Niven is a second synthetic person used to test isolation. Nothing is private.

replies.json is an items table stand-in keyed by inclusive physical-line windows.
model.py captures its prompt next to the script and returns that window from
replies.json. Copy the fixtures under target/init1b/ before executing it.
These deterministic replies test plumbing and validation, not model judgment.

Example (from the repository root; use a fresh scratch store each replay):

```sh
mkdir -p target/init1b/example
cp testdata/items/conversation.jsonl testdata/items/replies.json target/init1b/example/
target/debug/palace items replay --person elora \
  --transcript target/init1b/example/conversation.jsonl \
  --store target/init1b/example/compiler.sqlite \
  --llm table:target/init1b/example/replies.json
target/debug/palace items list --store target/init1b/example/compiler.sqlite \
  --person elora --status open --json
```

Expected replay: created=3, updated=1, closed=1, expired=1, rejected=1,
open=1, windows=4. The one remaining item is pastoral. The rejection is the
intentionally fabricated quote. These scripted model choices do not measure
missed closures or semantic duplicates on real conversations.

Settle is unchanged until `SETTLE_ITEMS=1` is explicitly configured. It uses
`PERSON` for the default `WING`. For a different resolved wing, `ITEMS_PERSON_MAP`
is an optional TSV file with `wing<TAB>person` rows; absent mappings are logged
and skipped. An item update failure is logged and does not stop settle.
