# ROLLUP: Beacon logger design — state as of 2031-04-06

## Claims
- C1. Ilse Varga | chose | SQLite for the beacon logger — 2031-04-05 — @SOURCE@

## Current consolidated truth
The beacon logger stores its readings in SQLite, a single-file store that is easy to copy, chosen by Ilse Varga. Retries stay off until the radio driver, the main source of instability, is stable.

## Supersedes
@SOURCE@

## Open items
- None.
