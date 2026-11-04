//! INIT-1b: additive open-items ledger in the claim store.
use crate::{Error, Refused, claims::ClaimStore, write_log};
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};

pub(crate) fn migrate(c: &rusqlite::Connection) -> Result<(), Error> {
    c.execute_batch(
        "CREATE TABLE IF NOT EXISTS open_items (
        id TEXT PRIMARY KEY, person TEXT NOT NULL,
        owner TEXT NOT NULL CHECK(owner IN ('assistant','person','other')),
        kind TEXT NOT NULL CHECK(kind IN ('task','decision','question','pastoral','topic')),
        title TEXT NOT NULL, due TEXT,
        status TEXT NOT NULL DEFAULT 'open' CHECK(status IN ('open','done','dropped','expired')),
        opened_at TEXT NOT NULL, touched_at TEXT NOT NULL, closed_at TEXT,
        source TEXT NOT NULL, quote TEXT NOT NULL, close_quote TEXT,
        times_raised INTEGER NOT NULL DEFAULT 0 CHECK(times_raised >= 0));
        CREATE INDEX IF NOT EXISTS open_items_person_status ON open_items(person,status);
        CREATE TABLE IF NOT EXISTS initiative_state (
            person TEXT PRIMARY KEY, pokes INTEGER NOT NULL DEFAULT 0 CHECK(pokes >= 0),
            last_poke_at TEXT, last_check_at TEXT, next_due_at TEXT);",
    )?;
    let schema: String = c.query_row(
        "SELECT sql FROM sqlite_master WHERE type='table' AND name='open_items'",
        [],
        |r| r.get(0),
    )?;
    if !schema.contains("'topic'") {
        // SQLite cannot alter a CHECK in place. Rebuild atomically, preserving rowids too.
        let tx = c.unchecked_transaction()?;
        let updated = schema.replace(
            "'task','decision','question','pastoral'",
            "'task','decision','question','pastoral','topic'",
        );
        if updated == schema {
            return Err("unrecognized open_items kind CHECK; migration refused".into());
        }
        let objects = {
            let mut query = tx.prepare(
                "SELECT sql FROM sqlite_master WHERE tbl_name='open_items' AND sql IS NOT NULL AND type IN ('index','trigger')",
            )?;
            query
                .query_map([], |r| r.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?
        };
        tx.execute_batch("ALTER TABLE open_items RENAME TO open_items_init2_old")?;
        tx.execute_batch(&updated)?;
        tx.execute_batch(
            "INSERT INTO open_items(rowid,id,person,owner,kind,title,due,status,opened_at,touched_at,closed_at,source,quote,close_quote,times_raised) SELECT rowid,* FROM open_items_init2_old ORDER BY rowid;
             DROP TABLE open_items_init2_old;",
        )?;
        for object in objects {
            tx.execute_batch(&object)?;
        }
        tx.commit()?;
    }
    Ok(())
}
impl ClaimStore {
    pub fn item_model_inputs(&mut self, marker: crate::privacy::Marker) {
        self.item_inputs = Some(marker);
    }
    /// SQL filters are applied before rows leave the store. Stable insertion order.
    pub fn items(&self, person: Option<&str>, status: Option<&str>) -> Result<Vec<Value>, Error> {
        let readers = person
            .filter(|p| {
                crate::privacy::identity()
                    .people
                    .as_ref()
                    .is_some_and(|map| map.knows(p))
            })
            .map(|p| vec![p.to_owned()])
            .unwrap_or_default();
        self.items_for_readers(person, status, &readers)
    }
    pub fn items_for_readers(
        &self,
        person: Option<&str>,
        status: Option<&str>,
        readers: &[String],
    ) -> Result<Vec<Value>, Error> {
        let admitted = crate::privacy::visible("item", "open_items.id", readers);
        let mut s = self.store.connection.prepare(&format!(
            "SELECT id,person,owner,kind,title,due,status,opened_at,touched_at,closed_at,source,quote,close_quote,times_raised
             FROM open_items WHERE (?1 IS NULL OR person=?1) AND (?2 IS NULL OR status=?2) AND {admitted} ORDER BY rowid"))?;
        Ok(s.query_map(rusqlite::params![person,status], |r| {
            let t = |i| r.get::<_, Option<String>>(i);
            Ok(json!({"id":t(0)?,"person":t(1)?,"owner":t(2)?,"kind":t(3)?,"title":t(4)?,"due":t(5)?,"status":t(6)?,
                "opened_at":t(7)?,"touched_at":t(8)?,"closed_at":t(9)?,"source":t(10)?,"quote":t(11)?,"close_quote":t(12)?,"times_raised":r.get::<_,i64>(13)?}))
        })?.collect::<Result<_,_>>()?)
    }
}

impl ClaimStore {
    /// Apply one shape/quote-checked action atomically with its before/after audit record.
    /// Targets are selected only within this person's open rows; global id collisions expose no row.
    pub fn apply_item(
        &mut self,
        person: &str,
        action: &Value,
        transcript: &str,
    ) -> Result<(), Error> {
        self.apply_item_at(person, action, transcript, None)
    }
    /// The caller supplies the validated cited transcript timestamp when available.
    pub fn apply_item_at(
        &mut self,
        person: &str,
        action: &Value,
        transcript: &str,
        cited_at: Option<&str>,
    ) -> Result<(), Error> {
        let tx = self.store.connection.transaction()?;
        let id = action["id"].as_str().ok_or("missing id")?;
        let verb = action["action"].as_str().ok_or("missing action")?;
        let before: Option<String> = tx.query_row(
            "SELECT json_object('id',id,'person',person,'owner',owner,'kind',kind,'title',title,'due',due,'status',status,'opened_at',opened_at,'touched_at',touched_at,'closed_at',closed_at,'source',source,'quote',quote,'close_quote',close_quote,'times_raised',times_raised) FROM open_items WHERE id=?1 AND person=?2 AND status='open'",
            params![id,person], |r|r.get(0)).optional()?;
        if verb == "create" {
            if tx
                .prepare("SELECT 1 FROM open_items WHERE id=?1")?
                .exists([id])?
            {
                return Err(Refused::error("create id already exists"));
            }
        } else if before.is_none() {
            return Err(Refused::error(
                "target is not an existing open id for this person",
            ));
        }
        let before: Value = before
            .map(|s| serde_json::from_str(&s))
            .transpose()?
            .unwrap_or(Value::Null);
        let at = cited_at
            .map(str::to_owned)
            .map(Ok)
            .unwrap_or_else(|| crate::graph::now(&tx))?;
        let mut after = if verb == "create" {
            json!({"id":id,"person":person,"owner":action["owner"],"kind":action["kind"],"title":action["title"],"due":action["due"],"status":"open","opened_at":at,"closed_at":null,"close_quote":null,"times_raised":0})
        } else {
            before.clone()
        };
        after["touched_at"] = json!(at);
        if verb == "create" || verb == "update" {
            for key in ["owner", "kind", "title", "due"] {
                if let Some(v) = action.get(key) {
                    after[key] = v.clone();
                }
            }
            after["source"] = json!(format!("{transcript}:{}", action["line"]));
            after["quote"] = action["quote"].clone();
        } else {
            after["status"] = if verb == "expire" {
                json!("expired")
            } else {
                action["status"].clone()
            };
            after["closed_at"] = json!(at);
            after["close_quote"] = action["close_quote"].clone();
        }
        let t = |key: &str| after[key].as_str();
        tx.execute("INSERT INTO open_items(id,person,owner,kind,title,due,status,opened_at,touched_at,closed_at,source,quote,close_quote,times_raised)
            VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)
            ON CONFLICT(id) DO UPDATE SET owner=excluded.owner,kind=excluded.kind,title=excluded.title,due=excluded.due,status=excluded.status,touched_at=excluded.touched_at,closed_at=excluded.closed_at,source=excluded.source,quote=excluded.quote,close_quote=excluded.close_quote",
            params![id,person,t("owner"),t("kind"),t("title"),t("due"),t("status"),t("opened_at"),t("touched_at"),t("closed_at"),t("source"),t("quote"),t("close_quote"),after["times_raised"].as_i64()])?;
        write_log::append(&tx,&json!({"operation":"items_action","person":person,"action":action,"transcript":transcript,"before":before,"after":after,"at":at}).to_string())?;
        let incoming = crate::privacy::origin(&tx, transcript)?;
        let mut inputs = crate::privacy::markers(&tx, "item", id)?;
        inputs.push(incoming);
        if let Some(m) = &self.item_inputs {
            inputs.push(m.clone());
        }
        let combined = crate::privacy::Marker::meet(&inputs, transcript);
        tx.execute(
            "DELETE FROM privacy_markers WHERE kind='item' AND id=?1",
            [id],
        )?;
        crate::privacy::put(&tx, "item", id, &combined)?;
        tx.commit()?;
        Ok(())
    }
}

impl ClaimStore {
    pub fn initiative_state(&self, person: &str) -> Result<Value, Error> {
        let row: Option<String> = self.store.connection.query_row(
            "SELECT json_object('pokes',pokes,'last_poke_at',last_poke_at,'last_check_at',last_check_at,'next_due_at',next_due_at) FROM initiative_state WHERE person=?1",
            [person], |r| r.get(0)).optional()?;
        Ok(row
            .map(|s| serde_json::from_str(&s))
            .transpose()?
            .unwrap_or(json!({"pokes":0})))
    }
    /// A check records the effective backoff reset and next due time, without counting a poke.
    pub fn initiative_checked(
        &mut self,
        person: &str,
        pokes: i64,
        at: &str,
        due: &str,
    ) -> Result<(), Error> {
        let tx = self.store.connection.transaction()?;
        tx.execute("INSERT INTO initiative_state(person,pokes,last_check_at,next_due_at) VALUES (?1,?2,?3,?4)
            ON CONFLICT(person) DO UPDATE SET pokes=excluded.pokes,last_check_at=excluded.last_check_at,next_due_at=excluded.next_due_at",
            params![person,pokes,at,due])?;
        write_log::append(&tx, &json!({"operation":"initiative_checked","person":person,"pokes":pokes,"at":at,"next_due_at":due}).to_string())?;
        tx.commit()?;
        Ok(())
    }
}

impl ClaimStore {
    /// A successful delivery and its item counter are committed together with the audit event.
    pub fn initiative_poked(&mut self, person: &str, item: &str, at: &str) -> Result<(), Error> {
        palace_core::items::timestamp_millis(at)?;
        let tx = self.store.connection.transaction()?;
        let before: Option<i64> = tx.query_row(
            "SELECT times_raised FROM open_items WHERE id=?1 AND person=?2 AND status='open' AND kind!='pastoral'",
            params![item,person], |r| r.get(0)).optional()?;
        let before = before.ok_or_else(|| {
            Refused::error("poke target is not an eligible open item for this person")
        })?;
        tx.execute(
            "UPDATE open_items SET times_raised=times_raised+1 WHERE id=?1 AND person=?2",
            params![item, person],
        )?;
        tx.execute("INSERT INTO initiative_state(person,pokes,last_poke_at,last_check_at) VALUES (?1,1,?2,?2)
            ON CONFLICT(person) DO UPDATE SET pokes=pokes+1,last_poke_at=excluded.last_poke_at,last_check_at=excluded.last_check_at,next_due_at=NULL",
            params![person,at])?;
        write_log::append(&tx, &json!({"operation":"initiative_poked","person":person,"item":item,"at":at,"times_raised_before":before,"times_raised_after":before+1}).to_string())?;
        tx.commit()?;
        Ok(())
    }
    pub fn initiative_skipped(
        &mut self,
        person: &str,
        at: &str,
        base_millis: i64,
    ) -> Result<(), Error> {
        let next = palace_core::items::timestamp_rfc3339(
            palace_core::items::timestamp_millis(at)?
                .checked_add(base_millis)
                .ok_or("base hours exceeds timestamp range")?,
        );
        let tx = self.store.connection.transaction()?;
        tx.execute("INSERT INTO initiative_state(person,last_check_at,next_due_at) VALUES (?1,?2,?3)
            ON CONFLICT(person) DO UPDATE SET last_check_at=excluded.last_check_at,next_due_at=excluded.next_due_at",
            params![person,at,next])?;
        write_log::append(
            &tx,
            &json!({"operation":"initiative_skipped","person":person,"at":at,"next_due_at":next})
                .to_string(),
        )?;
        tx.commit()?;
        Ok(())
    }
}
