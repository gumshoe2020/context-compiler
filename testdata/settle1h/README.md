# testdata/settle1h

Synthetic fixtures for the SETTLE-1h acceptance (dist/test/settle1h.sh): repair mends a cited
live specific's missing value. Every person, place and object here is invented. The checks reuse
the SETTLE-1g quarry transcripts (testdata/settle1g: the live specifics `Mira | found | the
brass compass` and `Mira | found | a silver thimble`). `rollup.md` is the rollup the stand-in
model writes; its line `@PROSE@` is replaced by the one line of a `prose-*.txt` file:
- `prose-paraphrase.txt`: cites the compass's handle with a paraphrase (mended, then passes);
- `prose-exact.txt`: cites it with its exact value (untouched, byte for byte);
- `prose-unknown.txt`: cites a handle the claim store does not hold (not mended);
- `prose-two.txt`: cites both handles, each with a paraphrase (both mended).
`@AS_OF@`, `@A@`, `@B@`, `@C@`, `@ID_A@` and `@ID_C@` are filled in by the test.
