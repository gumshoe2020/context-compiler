CREATE TABLE IF NOT EXISTS write_log(sequence INTEGER PRIMARY KEY, payload TEXT NOT NULL,
 previous_hash TEXT NOT NULL, hash TEXT NOT NULL);
CREATE TRIGGER IF NOT EXISTS write_log_no_update BEFORE UPDATE ON write_log
 BEGIN SELECT RAISE(ABORT,'write_log is append-only'); END;
CREATE TRIGGER IF NOT EXISTS write_log_no_delete BEFORE DELETE ON write_log
 BEGIN SELECT RAISE(ABORT,'write_log is append-only'); END;
CREATE TABLE IF NOT EXISTS claims(id TEXT PRIMARY KEY, number TEXT NOT NULL, rollup TEXT NOT NULL,
 subject TEXT NOT NULL, predicate TEXT NOT NULL, object TEXT NOT NULL, date TEXT NOT NULL,
 source TEXT NOT NULL, quote TEXT, status TEXT NOT NULL DEFAULT 'live', superseded_by TEXT,
 confidence_base REAL NOT NULL, checks INTEGER NOT NULL DEFAULT 0, survived INTEGER NOT NULL DEFAULT 0,
 first_seen TEXT NOT NULL, last_checked TEXT);
CREATE TABLE IF NOT EXISTS findings(id TEXT PRIMARY KEY, claim TEXT, rollup TEXT NOT NULL,
 kind TEXT NOT NULL, evidence TEXT NOT NULL, proposal TEXT, checker TEXT NOT NULL,
 checked_at TEXT NOT NULL, decision TEXT, last_seen TEXT, seen_count INTEGER NOT NULL DEFAULT 1);
CREATE INDEX IF NOT EXISTS findings_claim ON findings(claim);
CREATE TABLE IF NOT EXISTS improvement_events(id TEXT PRIMARY KEY, at TEXT NOT NULL, by TEXT NOT NULL,
 verb TEXT NOT NULL, from_claim TEXT, to_claim TEXT, finding TEXT, text TEXT NOT NULL,
 axis_improved TEXT NOT NULL, reversible_by TEXT);
CREATE TRIGGER IF NOT EXISTS improvement_events_no_update BEFORE UPDATE ON improvement_events
 BEGIN SELECT RAISE(ABORT,'improvement_events is append-only'); END;
CREATE TRIGGER IF NOT EXISTS improvement_events_no_delete BEFORE DELETE ON improvement_events
 BEGIN SELECT RAISE(ABORT,'improvement_events is append-only'); END;
CREATE TABLE IF NOT EXISTS citations(triple_id TEXT NOT NULL, claim_id TEXT NOT NULL,
 rollup TEXT NOT NULL, cited_at TEXT NOT NULL, PRIMARY KEY(triple_id, claim_id, rollup));
CREATE TABLE IF NOT EXISTS claim_numbers(rollup TEXT NOT NULL, number TEXT NOT NULL, claim_id TEXT NOT NULL,
 date TEXT NOT NULL, thread TEXT NOT NULL, PRIMARY KEY(rollup, number));
CREATE INDEX IF NOT EXISTS claim_numbers_date ON claim_numbers(date, number);
CREATE TABLE IF NOT EXISTS asks(finding TEXT PRIMARY KEY, claim TEXT NOT NULL, kind TEXT NOT NULL,
 token TEXT NOT NULL, line TEXT NOT NULL, asked_at TEXT NOT NULL, answered_at TEXT, answer TEXT);
CREATE TABLE IF NOT EXISTS replies(token TEXT NOT NULL, text TEXT NOT NULL, event TEXT,
 at TEXT NOT NULL, PRIMARY KEY(token, text));
CREATE TABLE IF NOT EXISTS auto_marks(finding TEXT NOT NULL, mark TEXT NOT NULL, claim TEXT,
 detail TEXT, at TEXT NOT NULL, PRIMARY KEY(finding, mark));
CREATE TABLE IF NOT EXISTS survived_checks(claim TEXT NOT NULL, checker TEXT NOT NULL,
 source_sha256 TEXT NOT NULL, at TEXT NOT NULL, PRIMARY KEY(claim, checker, source_sha256));
CREATE TABLE IF NOT EXISTS fidelity(point TEXT NOT NULL, file TEXT NOT NULL, type TEXT NOT NULL,
 sampled INTEGER NOT NULL, supported INTEGER NOT NULL, missing INTEGER NOT NULL,
 contradicted INTEGER NOT NULL, unjudged INTEGER NOT NULL, rollup_depth INTEGER, primary_share REAL,
 judge TEXT NOT NULL, items TEXT NOT NULL, at TEXT NOT NULL, PRIMARY KEY(point, file, type));
