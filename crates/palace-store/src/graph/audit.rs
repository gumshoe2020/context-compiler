use super::Graph;
use crate::{Error, write_log};
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::Path,
};
pub fn record(
    c: &Connection,
    wal: &Path,
    op: &str,
    args: &Value,
    result: &Value,
    timestamp: &str,
) -> Result<(), Error> {
    let mut params = args.clone();
    params["via"] = json!("palace-rs");
    let line = json!({"timestamp":timestamp,"operation":format!("kg_{op}"),"params":params,"result":result}).to_string();
    write_log::append(c, &line)?;
    c.execute(
        "INSERT INTO graph_wal_outbox SELECT max(sequence),?1,?2 FROM write_log",
        params![wal.to_str().ok_or("non-UTF8 WAL path")?, line],
    )?;
    Ok(())
}
impl Graph {
    pub(super) fn flush_wal(&self) -> Result<(), Error> {
        let mut s = self
            .store
            .connection
            .prepare("SELECT sequence,path,line FROM graph_wal_outbox ORDER BY sequence")?;
        let rows = s
            .query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        for (sequence, path, line) in rows {
            let path = Path::new(&path);
            let parent = path.parent().ok_or("WAL has no parent")?;
            let mut builder = fs::DirBuilder::new();
            builder.recursive(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            builder.create(parent)?;
            let existing = match fs::read_to_string(path) {
                Ok(s) => s,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
                Err(e) => return Err(e.into()),
            };
            if !existing.lines().any(|s| s == line) {
                let mut options = OpenOptions::new();
                options.create(true).append(true);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::OpenOptionsExt;
                    options.mode(0o600);
                }
                let mut file = options.open(path)?;
                let torn = !existing.is_empty() && !existing.ends_with('\n');
                if torn {
                    write_log::append(&self.store.connection, &json!({"operation":"wal_separator_recovery","path":path,"sequence":sequence}).to_string())?;
                }
                append_line(&mut file, &line, torn)?;
                file.sync_all()?;
            }
            self.store
                .connection
                .execute("DELETE FROM graph_wal_outbox WHERE sequence=?", [sequence])?;
        }
        Ok(())
    }
}

/// One append call, including the delimiter. A short write remains pending; never retry its tail.
fn append_line(writer: &mut impl Write, line: &str, torn: bool) -> std::io::Result<()> {
    let bytes = format!("{}{line}\n", if torn { "\n" } else { "" });
    if writer.write(bytes.as_bytes())? != bytes.len() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::WriteZero,
            "short WAL append; outbox retained",
        ));
    }
    Ok(())
}
pub fn verify_single_write() -> Result<(), Error> {
    struct Spy {
        calls: usize,
        short: bool,
        bytes: Vec<u8>,
    }
    impl Write for Spy {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.calls += 1;
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len() - usize::from(self.short))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    for short in [false, true] {
        let mut spy = Spy {
            calls: 0,
            short,
            bytes: vec![],
        };
        let failed = append_line(&mut spy, "{}", true).is_err();
        if failed != short || spy.calls != 1 || spy.bytes != b"\n{}\n" {
            return Err("WAL append was not one complete write".into());
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn wal_line_is_one_write_including_newline_and_short_writes_fail() {
        super::verify_single_write().unwrap();
    }
}
