use crate::Error;
use palace_core::ids::{length_prefixed, sha256};
use rusqlite::{Connection, OptionalExtension, params};
fn hash(sequence: i64, previous: &str, payload: &str) -> String {
    sha256(length_prefixed(&[&sequence.to_string(), previous, payload]).as_bytes())
}
pub fn append(c: &Connection, payload: &str) -> Result<(), Error> {
    let (sequence, previous): (i64, String) = c
        .query_row(
            "SELECT sequence,hash FROM write_log ORDER BY sequence DESC LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?
        .unwrap_or((0, String::new()));
    let sequence = sequence + 1;
    c.execute(
        "INSERT INTO write_log VALUES (?1,?2,?3,?4)",
        params![
            sequence,
            payload,
            previous,
            hash(sequence, &previous, payload)
        ],
    )?;
    Ok(())
}
pub fn verify(c: &Connection) -> Result<(), Error> {
    let mut s =
        c.prepare("SELECT sequence,payload,previous_hash,hash FROM write_log ORDER BY sequence")?;
    let mut rows = s.query([])?;
    let mut previous = String::new();
    let mut sequence = 1;
    while let Some(r) = rows.next()? {
        let actual: i64 = r.get(0)?;
        let payload: String = r.get(1)?;
        let parent: String = r.get(2)?;
        let digest: String = r.get(3)?;
        if actual != sequence || parent != previous || digest != hash(sequence, &previous, &payload)
        {
            return Err(format!("write_log chain broken at {actual}").into());
        }
        previous = digest;
        sequence += 1;
    }
    Ok(())
}
