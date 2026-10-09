//! M5b-4: what `palace doctor --acceptance m5b4` reads back from the stand-in `--send` and
//! `--page` commands of `scripts/palace-cron.sh`. Each stand-in run appends its standard input
//! and then the line [`END`]; nothing here does IO.

/// The line a stand-in appends after the text it was given.
pub const END: &str = "--- end of delivery ---";

/// The texts delivered, one per run of the stand-in, each without its trailing newline.
pub fn deliveries(log: &str) -> Vec<String> {
    let mut texts = Vec::new();
    let mut current: Vec<&str> = Vec::new();
    for line in log.lines() {
        if line == END {
            texts.push(current.join("\n"));
            current.clear();
        } else {
            current.push(line);
        }
    }
    texts
}

/// The reply token `#f_<8 hex>` that ends an ask line, as the wrapper finds it (the last one).
pub fn ask_token(line: &str) -> Option<&str> {
    let start = line.rfind("#f_")?;
    let token = line.get(start..start + 11)?;
    token[3..]
        .bytes()
        .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        .then_some(token)
}

/// A page is one line naming the stage and the exit code: `CRON RETRY stage=<s> exit=3
/// tries=<n>` or `CRON BROKEN stage=<s> exit=<n>`, the rollup's failed validation: `CRON ROLLUP
/// INVALID exit=4 kept=<dir>` (M5b-10 item 4), or counting failed deliveries: `CRON UNDELIVERED
/// failed=<k> exit=<n> runs=<r>`. Anything else (an ask, the digest) is not.
pub fn is_page_line(text: &str) -> bool {
    if let Some(dir) = text.strip_prefix(ROLLUP_INVALID) {
        return !dir.is_empty() && !dir.contains('\n');
    }
    let words: Vec<&str> = text.split(' ').collect();
    let stage = |w: &str| w.strip_prefix("stage=").is_some_and(|s| !s.is_empty());
    let number = |w: &str, key: &str| {
        w.strip_prefix(key)
            .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
    };
    match words.as_slice() {
        ["CRON", "RETRY", s, "exit=3", t] => stage(s) && number(t, "tries="),
        ["CRON", "BROKEN", s, e] => stage(s) && number(e, "exit="),
        ["CRON", "UNDELIVERED", f, e, r] => {
            number(f, "failed=") && number(e, "exit=") && number(r, "runs=")
        }
        _ => false,
    }
}

/// The page of a rollup that failed its validation, before the kept attempts' directory.
pub const ROLLUP_INVALID: &str = "CRON ROLLUP INVALID exit=4 kept=";

/// The `runs=<r>` of a `CRON UNDELIVERED` page (M5b-5 item 3), else `None`.
pub fn undelivered_runs(text: &str) -> Option<u32> {
    let rest = text.strip_prefix("CRON UNDELIVERED ")?;
    let runs = rest.split(' ').find_map(|w| w.strip_prefix("runs="))?;
    runs.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deliveries_split_on_the_end_line() {
        let log = format!(
            "rule:auto: 1 weakened\nqueue=0 asks=0 unsent=0\n{END}\nask #f_0123abcd\n{END}\npartial"
        );
        assert_eq!(
            deliveries(&log),
            [
                "rule:auto: 1 weakened\nqueue=0 asks=0 unsent=0",
                "ask #f_0123abcd"
            ]
        );
        assert!(deliveries("").is_empty());
    }

    #[test]
    fn the_token_is_the_last_one_on_the_line() {
        let line = "C13: claim \"x #f_ffffffff\" — no source. Reply: drop #f_238a245a";
        assert_eq!(ask_token(line), Some("#f_238a245a"));
        assert_eq!(ask_token("no token here"), None);
        assert_eq!(ask_token("short #f_12ab"), None);
        assert_eq!(ask_token("upper #f_ABCDEF12"), None);
    }

    #[test]
    fn only_stage_and_exit_lines_are_pages() {
        assert!(is_page_line("CRON RETRY stage=watch exit=3 tries=4"));
        assert!(is_page_line("CRON BROKEN stage=auto exit=2"));
        assert!(is_page_line("CRON BROKEN stage=signal exit=143"));
        assert!(is_page_line("CRON UNDELIVERED failed=2 exit=1 runs=3"));
        assert!(!is_page_line("CRON UNDELIVERED failed=2 exit=1"));
        assert_eq!(
            undelivered_runs("CRON UNDELIVERED failed=2 exit=1 runs=3"),
            Some(3)
        );
        assert_eq!(undelivered_runs("CRON BROKEN stage=auto exit=2"), None);
        assert!(!is_page_line("CRON OK asks=1 digest=1"));
        assert!(is_page_line(
            "CRON ROLLUP INVALID exit=4 kept=/s/rollup-attempts"
        ));
        assert!(!is_page_line("CRON ROLLUP INVALID exit=4 kept="));
        assert!(!is_page_line(
            "CRON ROLLUP INVALID exit=4 kept=/s\n- C6. Callum | quoted | x"
        ));
        assert!(!is_page_line("CRON BROKEN stage= exit=2"));
        assert!(!is_page_line("rule:auto: 1 weakened"));
        assert!(!is_page_line(
            "CRON BROKEN stage=auto exit=2\nask #f_0123abcd"
        ));
    }
}
