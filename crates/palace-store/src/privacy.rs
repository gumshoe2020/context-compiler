//! Compiler-owned privacy provenance. Never changes the MemPalace schema.
use crate::Error;
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub struct Identity {
    pub people: Option<People>,
    pub conversation: Option<String>,
    /// A deliberate local CLI/API write has operator provenance. An unidentified MCP model
    /// call has no authenticated source, even though deliberate MCP searches see everything.
    pub local_operator: bool,
}
impl Default for Identity {
    fn default() -> Self {
        Self {
            people: None,
            conversation: None,
            local_operator: true,
        }
    }
}
static IDENTITY: std::sync::OnceLock<Identity> = std::sync::OnceLock::new();
thread_local! { static CALL_CONVERSATION: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) }; }
pub fn conversation_id() -> Option<String> {
    CALL_CONVERSATION
        .with(|c| c.borrow().clone())
        .or_else(|| identity().conversation.clone())
}
/// Adapter metadata is scoped to one synchronous MCP call and cannot spill into the next.
pub fn in_conversation<T>(session: Option<&str>, call: impl FnOnce() -> T) -> T {
    struct Reset(Option<String>);
    impl Drop for Reset {
        fn drop(&mut self) {
            CALL_CONVERSATION.with(|c| *c.borrow_mut() = self.0.take());
        }
    }
    let _reset = Reset(CALL_CONVERSATION.with(|c| c.replace(session.map(str::to_owned))));
    call()
}
pub fn configure(identity: Identity) -> Result<(), Error> {
    IDENTITY
        .set(identity)
        .map_err(|_| "privacy identity already configured".into())
}
pub fn identity() -> &'static Identity {
    static EMPTY: std::sync::OnceLock<Identity> = std::sync::OnceLock::new();
    IDENTITY
        .get()
        .unwrap_or_else(|| EMPTY.get_or_init(Identity::default))
}
pub fn conversation(session: &str, _wing: &str) -> Marker {
    identity()
        .people
        .as_ref()
        .map_or_else(|| Marker::quarantine(session), |p| p.resolve(session, None))
}
pub fn register(c: &Connection, locator: &str, marker: &Marker) -> Result<(), Error> {
    c.execute(
        "INSERT OR REPLACE INTO privacy_sources(locator,marker) VALUES (?1,?2)",
        params![locator, marker.json().to_string()],
    )?;
    Ok(())
}
fn register_sticky(c: &Connection, locator: &str, marker: &Marker) -> Result<(), Error> {
    let old: Option<String> = c
        .query_row(
            "SELECT marker FROM privacy_sources WHERE locator=?1",
            [locator],
            |r| r.get(0),
        )
        .optional()?;
    let marker = match old {
        Some(old) => {
            let old = from_json(&serde_json::from_str(&old)?)?;
            if !marker.private {
                old
            } else {
                Marker::meet(&[old.clone(), marker.clone()], &old.source)
            }
        }
        None => marker.clone(),
    };
    register(c, locator, &marker)
}

fn narrow(c: &Connection, kind: &str, id: &str, restriction: &Marker) -> Result<(), Error> {
    if restriction.private {
        let old_markers = markers(c, kind, id)?;
        // Independent graph supports remain independent. Legacy provenance that cannot
        // identify a support is conservatively narrowed, rather than leaving a shared leak.
        let matched =
            kind == "triple" && old_markers.iter().any(|m| m.source == restriction.source);
        for old in old_markers
            .into_iter()
            .filter(|m| !matched || m.source == restriction.source)
        {
            let marker = Marker::meet(&[old.clone(), restriction.clone()], &old.source);
            put(c, kind, id, &marker)?;
        }
    }
    Ok(())
}
/// Existing local/operator commands remain shared; identified conversation writes fail closed.
pub fn origin(c: &Connection, locator: &str) -> Result<Marker, Error> {
    if !identity().local_operator && conversation_id().is_none() {
        return Ok(Marker::quarantine("unidentified:mcp"));
    }
    let mut m = source(c, locator)?;
    let mut registered=c.prepare("SELECT 1 FROM privacy_sources WHERE ?1=locator OR substr(?1,1,length(locator)+1)=locator||':'")?.exists([locator])?;
    if !registered && m.quarantine {
        let claims = markers(c, "claim", locator)?;
        if let Some(first) = claims.first() {
            m = Marker::meet(&claims, &first.source);
            registered = true;
        }
    }
    if let Some(session) = &conversation_id() {
        let current = conversation(session, "");
        return Ok(if registered || !m.quarantine {
            Marker::meet(&[m, current], session)
        } else {
            current
        });
    }
    if !m.quarantine {
        return Ok(m);
    }
    if registered {
        return Ok(m);
    }
    if identity().people.is_some() {
        return Ok(m);
    }
    Ok(Marker::shared("local:operator"))
}
pub fn inherit(
    c: &Connection,
    kind: &str,
    id: &str,
    source_kind: &str,
    source_id: &str,
) -> Result<(), Error> {
    let inputs = markers(c, source_kind, source_id)?;
    if inputs.is_empty() {
        put(c, kind, id, &origin(c, source_id)?)?;
    } else {
        for m in inputs {
            put(c, kind, id, &m)?;
        }
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Marker {
    pub source: String,
    pub people: BTreeSet<String>,
    pub audience: BTreeSet<String>,
    pub private: bool,
    pub quarantine: bool,
}
impl Marker {
    pub fn quarantine(source: &str) -> Self {
        Self {
            source: source.into(),
            people: BTreeSet::new(),
            audience: BTreeSet::new(),
            private: true,
            quarantine: true,
        }
    }
    pub fn shared(source: &str) -> Self {
        Self {
            private: false,
            quarantine: false,
            ..Self::quarantine(source)
        }
    }
    pub fn meet(inputs: &[Self], source: &str) -> Self {
        if inputs.is_empty() {
            return Self::quarantine(source);
        }
        let restricted: Vec<_> = inputs.iter().filter(|m| m.private).collect();
        let mut audience = restricted
            .first()
            .map_or_else(BTreeSet::new, |m| m.audience.clone());
        for input in restricted.iter().skip(1) {
            audience.retain(|p| input.audience.contains(p));
        }
        Self {
            source: source.into(),
            people: inputs
                .iter()
                .flat_map(|m| m.people.iter().cloned())
                .collect(),
            audience,
            private: !restricted.is_empty(),
            quarantine: inputs.iter().any(|m| m.quarantine),
        }
    }
    pub fn admits(&self, readers: &[String]) -> bool {
        !self.private
            || (!self.quarantine
                && !readers.is_empty()
                && readers.iter().all(|r| self.audience.contains(r)))
    }
    pub fn annotation(&self) -> String {
        let people = self.people.iter().cloned().collect::<Vec<_>>().join(",");
        if self.private {
            let audience = self.audience.iter().cloned().collect::<Vec<_>>().join(",");
            format!(
                "[private: {}; source: {}]",
                if self.quarantine {
                    "quarantine"
                } else {
                    &audience
                },
                self.source
            )
        } else {
            format!("[from: {people}]")
        }
    }
    pub fn json(&self) -> Value {
        json!({"source":self.source,"people":self.people,"audience":self.audience,"private":self.private,"quarantine":self.quarantine})
    }
}

/// One rollup audience. Shared is the default; private sets use comma-separated person IDs.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct Audience(BTreeSet<String>);
impl std::str::FromStr for Audience {
    type Err = String;
    fn from_str(value: &str) -> Result<Self, String> {
        if value == "shared" {
            return Ok(Self::default());
        }
        let mut people = BTreeSet::new();
        for id in value.split(',') {
            if id.is_empty()
                || id == "shared"
                || !id.as_bytes()[0].is_ascii_lowercase()
                || !id
                    .bytes()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
            {
                return Err(
                    "audience must be shared or comma-separated lowercase person IDs".into(),
                );
            }
            people.insert(id.to_owned());
        }
        Ok(Self(people))
    }
}
impl Audience {
    /// Discover only exact, non-quarantined audiences among the supplied diary entries.
    /// Shared always runs first; remaining audiences are ordered by their sorted person sets.
    pub fn discover(c: &Connection, entries: &[String]) -> Result<Vec<Self>, Error> {
        let mut found = BTreeSet::from([Self::default()]);
        for entry in entries {
            let id = entry.strip_suffix(".md").unwrap_or(entry);
            for marker in markers(c, "entry", id)? {
                if marker.quarantine || (marker.private && marker.audience.is_empty()) {
                    continue;
                }
                let audience = Self(if marker.private {
                    marker.audience
                } else {
                    BTreeSet::new()
                });
                if audience.admits_entry(c, id)? {
                    found.insert(audience);
                }
            }
        }
        Ok(found.into_iter().collect())
    }
    pub fn private(&self) -> bool {
        !self.0.is_empty()
    }
    pub fn marker(&self, source: &str) -> Marker {
        Marker {
            private: self.private(),
            audience: self.0.clone(),
            ..Marker::shared(source)
        }
    }
    pub fn matches(&self, marker: &Marker) -> bool {
        !marker.quarantine
            && marker.private == self.private()
            && (!marker.private || marker.audience == self.0)
    }
    /// Every active support must agree. Conflicting supports and absent metadata fail closed.
    pub fn admits_entry(&self, c: &Connection, entry: &str) -> Result<bool, Error> {
        let id = entry.strip_suffix(".md").unwrap_or(entry);
        let active = markers(c, "entry", id)?
            .into_iter()
            .filter_map(|m| {
                let retired = c
                    .prepare(
                        "SELECT 1 FROM privacy_retired WHERE kind='entry' AND id=?1 AND source=?2",
                    )
                    .and_then(|mut q| q.exists(params![id, m.source]));
                match retired {
                    Ok(true) => None,
                    Ok(false) => Some(Ok(m)),
                    Err(e) => Some(Err(e)),
                }
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(!active.is_empty() && active.iter().all(|m| self.matches(m)))
    }
}

/// The identity map, and its `private_tags` keys in file order (a `Value` sorts them).
#[derive(Clone, Debug)]
pub struct People(Value, Vec<String>);
/// `private_tags` keys in the order the file lists them.
fn tag_order(text: &str) -> Vec<String> {
    use serde::{
        Deserialize, Deserializer,
        de::{IgnoredAny, MapAccess, Visitor},
    };
    struct Keys(Vec<String>);
    impl<'de> Deserialize<'de> for Keys {
        fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
            struct Collect;
            impl<'de> Visitor<'de> for Collect {
                type Value = Keys;
                fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                    f.write_str("an object")
                }
                fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Keys, A::Error> {
                    let mut keys = Vec::new();
                    while let Some((key, IgnoredAny)) = map.next_entry::<String, IgnoredAny>()? {
                        keys.push(key);
                    }
                    Ok(Keys(keys))
                }
            }
            d.deserialize_map(Collect)
        }
    }
    #[derive(Deserialize)]
    struct File {
        private_tags: Option<Keys>,
    }
    serde_json::from_str::<File>(text)
        .ok()
        .and_then(|file| file.private_tags)
        .map_or_else(Vec::new, |keys| keys.0)
}
impl People {
    pub fn parse(text: &str) -> Result<Self, Error> {
        let value: Value = serde_json::from_str(text)?;
        let people = value["people"]
            .as_object()
            .ok_or("people must be an object")?;
        let mut handles = BTreeSet::new();
        for (id, person) in people {
            if id.is_empty()
                || !id
                    .bytes()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
                || !id.as_bytes()[0].is_ascii_lowercase()
            {
                return Err("person id must be a lowercase slug".into());
            }
            let list = person["handles"]
                .as_array()
                .ok_or("handles must be an array")?;
            for handle in list {
                let h = handle
                    .as_str()
                    .filter(|h| h.contains(':'))
                    .ok_or("invalid handle")?;
                if !handles.insert(h) {
                    return Err("duplicate handle".into());
                }
            }
            for key in ["owner", "private"] {
                if person.get(key).is_some_and(|v| !v.is_boolean()) {
                    return Err("person flags must be booleans".into());
                }
            }
        }
        let rows = value["conversations"]
            .as_array()
            .ok_or("conversations must be an array")?;
        let mut matches = BTreeSet::new();
        for row in rows {
            let key = row["match"]
                .as_str()
                .filter(|s| !s.is_empty())
                .ok_or("conversation match required")?;
            if !matches.insert((key, row["prefix"] == true)) {
                return Err("duplicate conversation match".into());
            }
            let members = row["people"]
                .as_array()
                .filter(|v| !v.is_empty())
                .ok_or("conversation people required")?;
            if members
                .iter()
                .any(|p| p.as_str().is_none_or(|p| !people.contains_key(p)))
            {
                return Err("unknown conversation person".into());
            }
            if row.get("prefix").is_some_and(|v| !v.is_boolean()) {
                return Err("prefix must be boolean".into());
            }
            if row.get("private").is_some_and(|v| !v.is_boolean()) {
                return Err("private must be boolean".into());
            }
        }
        for key in ["private_wings", "private_tags"] {
            if let Some(mapping) = value.get(key) {
                let mut names = BTreeSet::new();
                for (name, members) in mapping
                    .as_object()
                    .ok_or("privacy mapping must be an object")?
                {
                    if name.is_empty()
                        || (key == "private_tags"
                            && (name.trim().is_empty()
                                || name.trim() != name
                                || !names.insert(name.to_lowercase())))
                        || members
                            .as_array()
                            .filter(|v| !v.is_empty())
                            .is_none_or(|ms| {
                                ms.iter()
                                    .any(|p| p.as_str().is_none_or(|p| !people.contains_key(p)))
                            })
                    {
                        return Err(
                            "private wing/tag requires a unique name and known people".into()
                        );
                    }
                }
            }
        }
        Ok(Self(value, tag_order(text)))
    }
    /// The approved audience categories: the `private_tags` keys starting `audience-`, in file
    /// order, each with the people it admits. A map without any has none (nothing is invented).
    pub fn audience_categories(&self) -> Vec<(String, Vec<String>)> {
        let mut seen = BTreeSet::new();
        let mut categories = Vec::new();
        for tag in &self.1 {
            let name = tag.to_lowercase();
            let members = self.0["private_tags"][tag].as_array();
            if let (true, true, Some(members)) = (
                name.starts_with("audience-"),
                seen.insert(name.clone()),
                members,
            ) {
                let people = members.iter().filter_map(Value::as_str);
                categories.push((name, people.map(str::to_owned).collect()));
            }
        }
        categories
    }
    fn matching_row(&self, session: &str) -> Option<&Value> {
        let rows = self.0["conversations"].as_array().unwrap();
        rows.iter()
            .find(|r| r["prefix"] != true && r["match"] == session)
            .or_else(|| {
                rows.iter()
                    .filter(|r| {
                        r["prefix"] == true && session.starts_with(r["match"].as_str().unwrap())
                    })
                    .max_by_key(|r| r["match"].as_str().unwrap().len())
            })
    }
    /// Explicit exact/prefix policy, then authenticated handles; never display names.
    pub fn resolve(&self, session: &str, sender: Option<&str>) -> Marker {
        let row = self.matching_row(session);
        let mut people: BTreeSet<String> = row
            .map(|r| {
                r["people"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter_map(|p| p.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();
        if row.is_none() {
            // OpenClaw DM keys end in :<channel>:direct:<native id> (also accept :dm:).
            let handle = session
                .split_once(":direct:")
                .or_else(|| session.split_once(":dm:"))
                .map(|(prefix, id)| {
                    format!("{}:{id}", prefix.rsplit(':').next().unwrap_or_default())
                });
            let h = sender.or(handle.as_deref());
            if let Some(h) = h {
                for (id, p) in self.0["people"].as_object().unwrap() {
                    if p["handles"].as_array().unwrap().iter().any(|v| v == h) {
                        people.insert(id.clone());
                    }
                }
            }
        }
        if people.is_empty() {
            return Marker::quarantine(session);
        }
        let private = row.is_some_and(|r| r["private"] == true)
            || people
                .iter()
                .any(|p| self.0["people"][p]["private"] == true);
        Marker {
            source: session.into(),
            audience: people.clone(),
            people,
            private,
            quarantine: false,
        }
    }
    pub fn knows(&self, id: &str) -> bool {
        self.0["people"].get(id).is_some()
    }
    pub fn unique_owner(&self) -> Result<String, Error> {
        let owners: Vec<_> = self.0["people"]
            .as_object()
            .unwrap()
            .iter()
            .filter(|(_, person)| person["owner"] == true)
            .map(|(id, _)| id.clone())
            .collect();
        if owners.len() != 1 {
            return Err("--owner requires exactly one owner in people.json".into());
        }
        Ok(owners[0].clone())
    }
    pub fn owner(&self, id: &str) -> bool {
        self.0["people"][id]["owner"] == true
    }
    /// Legacy matching tags form one audience; audience-* tags are independent
    /// restrictions, met with that audience and existing privacy.
    pub fn tagged(&self, marker: &Marker, tags: &BTreeSet<String>) -> (Marker, Option<String>) {
        if let Some(tag) = tags.iter().find(|tag| {
            tag.starts_with("audience-") && self.0["private_tags"].get(tag.as_str()).is_none()
        }) {
            return (
                Marker::meet(
                    &[marker.clone(), Marker::quarantine(&marker.source)],
                    &marker.source,
                ),
                Some(format!("tag:{tag}")),
            );
        }
        let mut audience = BTreeSet::new();
        let mut matched = BTreeSet::new();
        let mut restrictions = vec![marker.clone()];
        if let Some(mapping) = self.0["private_tags"].as_object() {
            for (tag, members) in mapping {
                if tags.contains(&tag.to_lowercase()) {
                    matched.insert(tag.to_lowercase());
                    let members: BTreeSet<String> = members
                        .as_array()
                        .unwrap()
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect();
                    if tag.starts_with("audience-") {
                        restrictions.push(Marker {
                            audience: members,
                            private: true,
                            ..Marker::shared(&marker.source)
                        });
                    } else {
                        audience.extend(members);
                    }
                }
            }
        }
        let reason = matched.first().map(|tag| format!("tag:{tag}"));
        if reason.is_none() {
            return (marker.clone(), None);
        }
        if !audience.is_empty() {
            restrictions.push(Marker {
                audience,
                private: true,
                ..Marker::shared(&marker.source)
            });
        }
        (Marker::meet(&restrictions, &marker.source), reason)
    }
}

/// Settle emits `Tags: a, b.`; hand-filed notes also use YAML inline or block lists.
/// Only tag fields are interpreted, never prose mentions or model-provided audiences.
pub fn entry_tags(text: &str) -> BTreeSet<String> {
    let mut tags = BTreeSet::new();
    let mut block = false;
    for line in text.lines() {
        let line = line.trim();
        if matches!(line, "---" | "...") {
            block = false;
            continue;
        }
        let lower = line.to_lowercase();
        let values = if lower.starts_with("tags:") {
            let values = line[5..].trim().trim_end_matches('.');
            block = values.is_empty();
            Some(values.trim_start_matches('[').trim_end_matches(']'))
        } else if block && line.starts_with("- ") {
            Some(line[2..].trim())
        } else {
            if !line.is_empty() {
                block = false;
            }
            None
        };
        if let Some(values) = values {
            for value in values.split(',') {
                let value = value.trim().trim_matches(['\'', '"']).trim().to_lowercase();
                if !value.is_empty() {
                    tags.insert(value);
                }
            }
        }
    }
    tags
}

pub fn tagged_entry(marker: &Marker, text: &str) -> Marker {
    let tags = entry_tags(text);
    match &identity().people {
        Some(people) => people.tagged(marker, &tags).0,
        None if tags.iter().any(|t| t.starts_with("audience-")) => Marker::meet(
            &[marker.clone(), Marker::quarantine(&marker.source)],
            &marker.source,
        ),
        None => marker.clone(),
    }
}

fn transcript_source(locator: &str) -> &str {
    locator
        .rsplit_once(':')
        .filter(|(_, lines)| {
            !lines.is_empty() && lines.bytes().all(|c| c.is_ascii_digit() || c == b'-')
        })
        .map_or(locator, |(path, _)| path)
}

/// Evidence used only by the explicit legacy backfill. New writes still fail closed.
#[derive(Default)]
pub struct LegacyEvidence {
    /// Normalized diary entry name -> (wing, transcript locator).
    pub entries: BTreeMap<String, (String, String)>,
    pub tags: BTreeMap<String, BTreeSet<String>>,
    pub wing_map: serde_json::Map<String, Value>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LegacyRow {
    pub kind: String,
    pub id: String,
    pub marker: Marker,
    pub reason: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Classified {
    marker: Marker,
    reason: String,
}
impl Classified {
    fn sticky(mut self, c: &Connection, kind: &str, id: &str) -> Result<Self, Error> {
        let mut inputs = markers(c, kind, id)?;
        if let Some(first) = inputs.first() {
            let source = first.source.clone();
            inputs.push(self.marker.clone());
            self.marker = Marker::meet(&inputs, &source);
        }
        Ok(self)
    }
    fn tagged(mut self, people: &People, tags: &BTreeSet<String>) -> Self {
        let (marker, reason) = people.tagged(&self.marker, tags);
        self.marker = marker;
        if let Some(reason) = reason {
            self.reason = reason;
        }
        self
    }
    fn row(&self, kind: &str, id: String) -> LegacyRow {
        LegacyRow {
            kind: kind.into(),
            id,
            marker: self.marker.clone(),
            reason: self.reason.clone(),
        }
    }
    fn meet(inputs: &[Self], source: &str) -> Self {
        if let Some(first) = inputs
            .first()
            .filter(|first| inputs.iter().all(|m| m == *first))
        {
            return first.clone();
        }
        let marker = Marker::meet(
            &inputs.iter().map(|m| m.marker.clone()).collect::<Vec<_>>(),
            source,
        );
        let reason = inputs
            .iter()
            .find(|m| m.reason.starts_with("tag:"))
            .or_else(|| inputs.iter().find(|m| m.marker.quarantine))
            .or_else(|| inputs.iter().find(|m| m.marker.private))
            .or_else(|| inputs.first())
            .map_or("unknown", |m| m.reason.as_str())
            .to_owned();
        Self { marker, reason }
    }
}
impl LegacyEvidence {
    pub fn diary_wing(root: &std::path::Path) -> String {
        let parts: Vec<_> = root.iter().filter_map(|p| p.to_str()).collect();
        parts
            .windows(2)
            .rev()
            .find(|p| p[0] == "wings")
            .map_or("openclaw", |p| p[1])
            .to_owned()
    }
    fn transcript_wing<'a>(&self, source: &'a str) -> Option<&'a str> {
        source
            .split('/')
            .collect::<Vec<_>>()
            .windows(3)
            .find(|p| p[0] == "transcripts" && !p[1].is_empty() && !p[2].is_empty())
            .map(|p| p[1])
    }
    fn classify(&self, people: &People, source: &str, wing: Option<&str>) -> Classified {
        let resolved = people.resolve(source, None);
        let wing = wing
            .filter(|w| !w.is_empty())
            .or_else(|| self.transcript_wing(source));
        // A private wing cannot be widened by a shared source mapping.
        if let Some(wing) = wing {
            let mut audience: BTreeSet<String> = people.0["private_wings"][wing]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect();
            let sessions = people.0["conversations"]
                .as_array()
                .unwrap()
                .iter()
                .map(|r| r["match"].as_str().unwrap())
                .chain(self.wing_map.keys().map(String::as_str));
            for session in sessions {
                let m = people.resolve(session, None);
                if m.private
                    && palace_core::transcript::wing_for(&self.wing_map, session) == Some(wing)
                {
                    audience.extend(m.audience);
                }
            }
            if !audience.is_empty() {
                return Classified {
                    marker: Marker {
                        source: format!("legacy:{wing}"),
                        people: audience.clone(),
                        audience,
                        private: true,
                        quarantine: false,
                    },
                    reason: format!("wing:{wing}"),
                };
            }
        }
        if !resolved.quarantine {
            let reason = people.matching_row(source).map_or("handle", |r| {
                if r["prefix"] == true {
                    "prefix"
                } else {
                    "exact"
                }
            });
            return Classified {
                marker: resolved,
                reason: reason.into(),
            };
        }
        // Explicit legacy migration retains its generic wing evidence policy. Runtime
        // writes and reads never infer an audience from a wing or path.
        if let Some(wing) = wing {
            return Classified {
                marker: Marker::shared(&format!("legacy:{wing}")),
                reason: format!("wing:{wing}"),
            };
        }
        Classified {
            marker: Marker::quarantine(source),
            reason: "unknown".into(),
        }
    }
}

pub const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS privacy_markers(kind TEXT NOT NULL,id TEXT NOT NULL,source TEXT NOT NULL,
 people TEXT NOT NULL,audience TEXT NOT NULL,private INTEGER NOT NULL,quarantine INTEGER NOT NULL,
 PRIMARY KEY(kind,id,source));
CREATE INDEX IF NOT EXISTS privacy_source ON privacy_markers(source);
CREATE TABLE IF NOT EXISTS privacy_sources(locator TEXT PRIMARY KEY, marker TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS privacy_retired(kind TEXT NOT NULL,id TEXT NOT NULL,source TEXT NOT NULL,PRIMARY KEY(kind,id,source));
";

pub fn put(c: &Connection, kind: &str, id: &str, marker: &Marker) -> Result<(), Error> {
    let id = if kind == "entry" {
        id.strip_suffix(".md").unwrap_or(id)
    } else {
        id
    };
    c.execute("INSERT OR REPLACE INTO privacy_markers(kind,id,source,people,audience,private,quarantine) VALUES (?1,?2,?3,?4,?5,?6,?7)",
        params![kind,id,marker.source,serde_json::to_string(&marker.people)?,serde_json::to_string(&marker.audience)?,marker.private,marker.quarantine])?;
    Ok(())
}
pub fn markers(c: &Connection, kind: &str, id: &str) -> Result<Vec<Marker>, Error> {
    let id = if kind == "entry" {
        id.strip_suffix(".md").unwrap_or(id)
    } else {
        id
    };
    if !c
        .prepare("SELECT 1 FROM pragma_table_list WHERE name='privacy_markers'")?
        .exists([])?
    {
        return Ok(Vec::new());
    }
    let mut s = c.prepare("SELECT source,people,audience,private,quarantine FROM privacy_markers WHERE kind=?1 AND id=?2 ORDER BY source")?;
    let rows = s
        .query_map([kind, id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, bool>(3)?,
                r.get::<_, bool>(4)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    rows.into_iter()
        .map(|(source, p, a, private, quarantine)| {
            Ok(Marker {
                source,
                people: serde_json::from_str(&p)?,
                audience: serde_json::from_str(&a)?,
                private,
                quarantine,
            })
        })
        .collect()
}
pub fn source(c: &Connection, locator: &str) -> Result<Marker, Error> {
    let found: Option<String> = c.query_row("SELECT marker FROM privacy_sources WHERE ?1=locator OR substr(?1,1,length(locator)+1)=locator||':' ORDER BY length(locator) DESC LIMIT 1", [locator], |r| r.get(0)).optional()?;
    if let Some(text) = found {
        return from_json(&serde_json::from_str(&text)?);
    }
    Ok(Marker::quarantine(locator))
}
pub fn from_json(v: &Value) -> Result<Marker, Error> {
    Ok(Marker {
        source: v["source"].as_str().ok_or("marker source")?.into(),
        people: serde_json::from_value(v["people"].clone())?,
        audience: serde_json::from_value(v["audience"].clone())?,
        private: v["private"].as_bool().ok_or("marker private")?,
        quarantine: v["quarantine"].as_bool().unwrap_or(false),
    })
}
/// SQL admit predicate. Missing metadata is hidden; backfill stamps legacy rows.
/// Reader values are encoded as JSON and SQL-quoted, never interpolated raw.
pub fn visible(kind: &str, id_sql: &str, readers: &[String]) -> String {
    let normalized;
    let id_sql = if kind == "entry" {
        normalized = format!(
            "CASE WHEN substr({id_sql},-3)='.md' THEN substr({id_sql},1,length({id_sql})-3) ELSE {id_sql} END"
        );
        normalized.as_str()
    } else {
        id_sql
    };
    let readers = serde_json::to_string(readers).unwrap().replace('\'', "''");
    format!(
        "(EXISTS (SELECT 1 FROM privacy_markers pm WHERE pm.kind='{kind}' AND pm.id={id_sql} AND NOT EXISTS (SELECT 1 FROM privacy_retired pr WHERE pr.kind=pm.kind AND pr.id=pm.id AND pr.source=pm.source) AND (pm.private=0 OR (pm.quarantine=0 AND json_array_length('{readers}')>0 AND NOT EXISTS (SELECT 1 FROM json_each('{readers}') rr WHERE rr.value NOT IN (SELECT value FROM json_each(pm.audience)))))))"
    )
}
pub fn shared_only(kind: &str, id_sql: &str) -> String {
    visible(kind, id_sql, &[])
}
pub fn visible_in(
    kind: &str,
    id_sql: &str,
    readers: &[String],
    conversation: Option<&str>,
) -> String {
    let visible = visible(kind, id_sql, readers);
    let Some(source) = conversation.filter(|s| !s.is_empty()) else {
        return visible;
    };
    let source = source.replace('\'', "''");
    let id_sql = if kind == "entry" {
        format!(
            "CASE WHEN substr({id_sql},-3)='.md' THEN substr({id_sql},1,length({id_sql})-3) ELSE {id_sql} END"
        )
    } else {
        id_sql.into()
    };
    format!(
        "({visible} OR EXISTS (SELECT 1 FROM privacy_markers pm WHERE pm.kind='{kind}' AND pm.id={id_sql} AND pm.quarantine=1 AND pm.source='{source}' AND NOT EXISTS (SELECT 1 FROM privacy_retired pr WHERE pr.kind=pm.kind AND pr.id=pm.id AND pr.source=pm.source)))"
    )
}
pub fn private(c: &Connection, kind: &str, id: &str) -> Result<bool, Error> {
    let rows = markers(c, kind, id)?;
    Ok(!rows.is_empty() && rows.iter().all(|m| m.private))
}

/// Construct a complete backfill plan without writing anything. Unknown origins quarantine.
pub fn backfill(c: &Connection, people: &People) -> Result<Vec<(String, String, Marker)>, Error> {
    Ok(legacy_backfill(c, people, &LegacyEvidence::default())?
        .into_iter()
        .map(|r| (r.kind, r.id, r.marker))
        .collect())
}
pub fn legacy_backfill(
    c: &Connection,
    people: &People,
    evidence: &LegacyEvidence,
) -> Result<Vec<LegacyRow>, Error> {
    let mut plan = Vec::new();
    let mut entries = BTreeMap::new();
    let mut sources: BTreeMap<String, Classified> = BTreeMap::new();
    let mut s = c.prepare("SELECT payload FROM write_log WHERE json_extract(payload,'$.operation')='distill_commit' ORDER BY sequence")?;
    for text in s.query_map([], |r| r.get::<_, String>(0))? {
        let e: Value = serde_json::from_str(&text?)?;
        let session = e["session"].as_str().unwrap_or_default();
        let wing = e["wing"]
            .as_str()
            .filter(|w| !w.is_empty())
            .or_else(|| e["copy"].as_str().and_then(|p| evidence.transcript_wing(p)))
            .or_else(|| {
                e["transcript"]
                    .as_str()
                    .and_then(|p| evidence.transcript_wing(p))
            });
        let m = evidence.classify(people, session, wing);
        let mut input_markers = Vec::new();
        if let Some(files) = e["files"].as_array() {
            for (index, file) in files.iter().enumerate() {
                let Some(file) = file.as_str() else {
                    continue;
                };
                let name = file.strip_suffix(".md").unwrap_or(file);
                let mut tags = evidence.tags.get(name).cloned().unwrap_or_default();
                if let Some(reply_tags) = e["reply"]["entries"][index]["tags"].as_array() {
                    tags.extend(
                        reply_tags
                            .iter()
                            .filter_map(Value::as_str)
                            .map(str::to_lowercase),
                    );
                }
                if let Some(body) = e["reply"]["entries"][index]["body"].as_str() {
                    tags.extend(entry_tags(body));
                }
                let entry = m.clone().tagged(people, &tags).sticky(c, "entry", name)?;
                input_markers.push(entry.clone());
                entries.insert(name.to_owned(), entry);
            }
        }
        // Relations/items may cite the transcript rather than a particular entry. They
        // inherit the meet of everything distilled in that run, before any graph write.
        let source_marker = if input_markers.is_empty() {
            m
        } else {
            Classified::meet(&input_markers, session)
        };
        for key in ["copy", "transcript"] {
            if let Some(path) = e[key].as_str() {
                let marker = match sources.get(path) {
                    Some(old) => {
                        Classified::meet(&[old.clone(), source_marker.clone()], &old.marker.source)
                    }
                    None => source_marker.clone(),
                };
                sources.insert(path.to_owned(), marker);
            }
        }
    }
    let origin = |sources: &BTreeMap<String, Classified>, path: &str, diary_wing: Option<&str>| {
        let fallback = evidence.classify(people, path, diary_wing);
        let recorded = sources
            .iter()
            .filter(|(p, _)| path == p.as_str() || path.starts_with(&format!("{p}:")))
            .max_by_key(|(p, _)| p.len())
            .map(|(_, m)| m.clone());
        match recorded {
            Some(m) if !m.marker.quarantine || m.reason.starts_with("tag:") => {
                // A hand-filed entry may still cite a resolved distill source. Preserve that
                // provenance, narrowing it if the diary's own wing is private.
                if diary_wing.is_some() && fallback.marker.private && !fallback.marker.quarantine {
                    Classified::meet(&[fallback, m], path)
                } else {
                    m
                }
            }
            _ => fallback,
        }
    };
    let explicit: BTreeSet<_> = entries
        .iter()
        .filter(|(_, m)| !m.marker.quarantine)
        .map(|(name, _)| name.clone())
        .collect();
    for (name, (wing, locator)) in &evidence.entries {
        let key = name.trim_end_matches(".md").to_owned();
        if entries.get(&key).is_none_or(|m| m.marker.quarantine) {
            entries.insert(key.clone(), origin(&sources, locator, Some(wing)));
        }
        let old = entries[&key].clone();
        let entry = old
            .tagged(people, evidence.tags.get(&key).unwrap_or(&BTreeSet::new()))
            .sticky(c, "entry", &key)?;
        if !locator.is_empty() && entry.marker.private {
            let locators = if entry.reason.starts_with("tag:") {
                vec![locator.as_str(), transcript_source(locator)]
            } else {
                vec![locator.as_str()]
            };
            for locator in locators {
                let source = origin(&sources, locator, Some(wing));
                let provenance = source.marker.source.clone();
                sources.insert(
                    locator.to_owned(),
                    Classified::meet(&[entry.clone(), source], &provenance),
                );
            }
        }
        entries.insert(key, entry);
    }
    let transcript = if c
        .prepare("SELECT 1 FROM pragma_table_info('claims') WHERE name='transcript'")?
        .exists([])?
    {
        "COALESCE(NULLIF(transcript,''),source,'')"
    } else {
        "source"
    };
    let mut query = c.prepare(&format!(
        "SELECT id,rollup,{transcript} FROM claims ORDER BY id"
    ))?;
    let rows = query
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let mut claims = BTreeMap::new();
    // Follow source-entry lineage to a fixed point; cycles and missing sources quarantine.
    for _ in 0..=rows.len() {
        let mut groups: BTreeMap<String, Vec<Classified>> = BTreeMap::new();
        for (id, entry, path) in &rows {
            let entry = &entry.trim_end_matches(".md").to_owned();
            let m = if explicit.contains(entry) {
                entries[entry].clone()
            } else {
                entries.get(entry_key(path)).cloned().unwrap_or_else(|| {
                    if let Some((wing, locator)) = evidence.entries.get(entry) {
                        let source = if path.is_empty() { locator } else { path };
                        origin(&sources, source, Some(wing))
                    } else {
                        origin(&sources, path, None)
                    }
                })
            };
            let m = if let Some(entry_marker) = entries.get(entry).filter(|m| m.marker.private) {
                Classified::meet(&[entry_marker.clone(), m], entry)
            } else {
                m
            }
            .sticky(c, "claim", id)?;
            claims.insert(id.clone(), m.clone());
            groups.entry(entry.clone()).or_default().push(m);
        }
        let mut changed = false;
        for (entry, ms) in groups {
            if explicit.contains(&entry) {
                continue;
            }
            let mut ms = ms;
            if let Some(current) = entries.get(&entry).filter(|m| m.marker.private) {
                ms.push(current.clone());
            }
            let marker = Classified::meet(&ms, &entry).sticky(c, "entry", &entry)?;
            if entries.get(&entry) != Some(&marker) {
                entries.insert(entry, marker);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    for (id, m) in &claims {
        plan.push(m.row("claim", id.clone()));
    }
    for (entry, m) in entries {
        plan.push(m.row("entry", entry));
    }
    for (path, m) in &sources {
        plan.push(m.row("source", path.clone()));
    }
    let mut s = c.prepare("SELECT id,source_closet FROM kg.triples ORDER BY id")?;
    for row in s.query_map([], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
    })? {
        let (id, closet) = row?;
        let v: Value = closet
            .and_then(|v| serde_json::from_str(&v).ok())
            .unwrap_or_default();
        let m = if let Some(path) = v["transcript"].as_str() {
            let m = entries_marker(&plan, path).unwrap_or_else(|| origin(&sources, path, None));
            if m.marker.quarantine {
                v["claim_id"]
                    .as_str()
                    .and_then(|id| claims.get(id))
                    .cloned()
                    .unwrap_or(m)
            } else {
                m
            }
        } else if let Some(m) = v["claim_id"].as_str().and_then(|id| claims.get(id)) {
            m.clone()
        } else {
            evidence.classify(people, "", None)
        };
        plan.push(m.row("triple", id));
    }
    if crate::auto::has_table(c, "citations")? {
        let active = if c
            .prepare("SELECT 1 FROM pragma_table_info('citations') WHERE name='withdrawn_at'")?
            .exists([])?
        {
            "WHERE withdrawn_at IS NULL"
        } else {
            ""
        };
        let mut query = c.prepare(&format!(
            "SELECT triple_id,claim_id FROM citations {active} ORDER BY triple_id,claim_id"
        ))?;
        for row in query.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
            let (triple, claim) = row?;
            if let Some(m) = claims.get(&claim) {
                let mut m = m.clone();
                m.marker.source = format!("citation:{claim}:{}", m.marker.source);
                plan.push(m.row("triple", triple));
            }
        }
    }
    if crate::auto::has_table(c, "open_items")? {
        let mut s = c.prepare("SELECT id,source FROM open_items ORDER BY id")?;
        for row in s.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
            let (id, path) = row?;
            let m = entries_marker(&plan, &path).unwrap_or_else(|| origin(&sources, &path, None));
            plan.push(m.row("item", id));
        }
    }
    if crate::auto::has_table(c, "asks")? {
        let mut s = c.prepare("SELECT finding,claim FROM asks ORDER BY finding")?;
        for row in s.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
            let (id, claim) = row?;
            let m = claims
                .get(&claim)
                .cloned()
                .unwrap_or_else(|| evidence.classify(people, "", None));
            plan.push(m.row("ask", id));
        }
    }
    if crate::auto::has_table(c, "fidelity")? {
        let mut query = c.prepare("SELECT DISTINCT file FROM fidelity ORDER BY file")?;
        for file in query.query_map([], |r| r.get::<_, String>(0))? {
            let file = file?;
            let marker =
                entries_marker(&plan, &file).unwrap_or_else(|| origin(&sources, &file, None));
            plan.push(marker.row("fidelity", file));
        }
    }
    Ok(plan)
}

// Restriction propagation accepts diary locators with numeric line/range suffixes.
// Basename collisions conservatively narrow; they can never authorize another entry.
fn entry_key(locator: &str) -> &str {
    let path = transcript_source(locator);
    let name = path.rsplit('/').next().unwrap_or(path);
    name.strip_suffix(".md").unwrap_or(name)
}
fn entries_marker(plan: &[LegacyRow], file: &str) -> Option<Classified> {
    plan.iter()
        .find(|r| r.kind == "entry" && entry_key(&r.id) == entry_key(file))
        .map(|r| Classified {
            marker: r.marker.clone(),
            reason: r.reason.clone(),
        })
}

impl crate::Store {
    pub fn apply_legacy_privacy_backfill(&mut self, plan: &[LegacyRow]) -> Result<(), Error> {
        apply_legacy_privacy_backfill(&mut self.connection, plan)
    }
    pub fn apply_privacy_backfill(
        &mut self,
        plan: &[(String, String, Marker)],
    ) -> Result<(), Error> {
        apply_privacy_plan(&mut self.connection, plan, &BTreeSet::new())
    }
}
/// Apply an already validated plan without migrations or ancillary lock-file writes.
/// Offline-copy callers back up the database before opening this write connection.
pub fn apply_legacy_privacy_backfill(
    connection: &mut Connection,
    plan: &[LegacyRow],
) -> Result<(), Error> {
    let narrow = plan
        .iter()
        .filter(|r| r.reason.starts_with("tag:"))
        .map(|r| {
            (
                r.kind.clone(),
                if r.kind == "entry" {
                    r.id.trim_end_matches(".md").to_owned()
                } else {
                    r.id.clone()
                },
                r.marker.source.clone(),
            )
        })
        .collect();
    apply_privacy_plan(
        connection,
        &plan
            .iter()
            .map(|r| (r.kind.clone(), r.id.clone(), r.marker.clone()))
            .collect::<Vec<_>>(),
        &narrow,
    )
}
fn apply_privacy_plan(
    connection: &mut Connection,
    plan: &[(String, String, Marker)],
    narrowing: &BTreeSet<(String, String, String)>,
) -> Result<(), Error> {
    let tx = connection.transaction()?;
    let existing: BTreeSet<(String, String)> = tx
        .prepare("SELECT DISTINCT kind,id FROM privacy_markers")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<Result<_, _>>()?;
    for (kind, id, marker) in plan {
        if kind == "source" {
            if narrowing.contains(&(kind.clone(), id.clone(), marker.source.clone()))
                || !tx
                    .prepare("SELECT 1 FROM privacy_sources WHERE locator=?1")?
                    .exists([id])?
            {
                register_sticky(&tx, id, marker)?;
            }
            continue;
        }
        // Existing decisions/supports are never widened by a repeated backfill.
        let normalized = if kind == "entry" {
            id.strip_suffix(".md").unwrap_or(id)
        } else {
            id
        };
        if !existing.contains(&(kind.clone(), normalized.to_owned())) {
            put(&tx, kind, id, marker)?;
        } else if narrowing.contains(&(kind.clone(), normalized.to_owned(), marker.source.clone()))
        {
            narrow(&tx, kind, id, marker)?;
        }
        if kind == "entry" {
            let inputs = markers(&tx, kind, id)?;
            let effective = Marker::meet(&inputs, &marker.source);
            register_sticky(&tx, id, &effective)?;
            register_sticky(&tx, id.strip_suffix(".md").unwrap_or(id), &effective)?;
            register_sticky(
                &tx,
                &format!("{}.md", id.strip_suffix(".md").unwrap_or(id)),
                &effective,
            )?;
        }
    }
    tx.commit()?;
    Ok(())
}

impl crate::claims::ClaimStore {
    pub fn mark_entry(&mut self, file: &str, marker: &Marker) -> Result<(), Error> {
        let tx = self.store.connection.transaction()?;
        let mut inputs = markers(&tx, "entry", file)?;
        inputs.push(marker.clone());
        let marker = &Marker::meet(&inputs, &marker.source);
        narrow(&tx, "entry", file, marker)?;
        put(&tx, "entry", file, marker)?;
        register_sticky(&tx, file, marker)?;
        register_sticky(&tx, file.strip_suffix(".md").unwrap_or(file), marker)?;
        tx.commit()?;
        Ok(())
    }
    pub fn privacy_markers(&self, kind: &str, id: &str) -> Result<Vec<Marker>, Error> {
        markers(&self.store.connection, kind, id)
    }
    pub fn privacy_source(&self, locator: &str) -> Result<Marker, Error> {
        origin(&self.store.connection, locator)
    }
}

pub fn distill_event(c: &Connection, e: &Value) -> Result<(), Error> {
    let session = identity()
        .conversation
        .as_deref()
        .unwrap_or_else(|| e["session"].as_str().unwrap_or_default());
    let marker = conversation(session, e["wing"].as_str().unwrap_or_default());
    let mut inputs = Vec::new();
    if let Some(files) = e["files"].as_array() {
        for (i, file) in files.iter().enumerate() {
            let Some(file) = file.as_str() else {
                continue;
            };
            let entry = &e["reply"]["entries"][i];
            let text = format!(
                "{}\nTags: {}",
                entry["body"].as_str().unwrap_or_default(),
                entry["tags"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            let mut ms = markers(c, "entry", file)?;
            ms.push(tagged_entry(&marker, &text));
            let marker = Marker::meet(&ms, &marker.source);
            narrow(c, "entry", file, &marker)?;
            put(c, "entry", file, &marker)?;
            register_sticky(c, file, &marker)?;
            register_sticky(c, file.strip_suffix(".md").unwrap_or(file), &marker)?;
            inputs.push(marker);
        }
    }
    let marker = if inputs.is_empty() {
        marker
    } else {
        Marker::meet(&inputs, session)
    };
    for key in ["copy", "transcript"] {
        if let Some(path) = e[key].as_str() {
            register_sticky(c, path, &marker)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rollup_audiences_are_exact_and_fail_closed() {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(SCHEMA).unwrap();
        let shared: Audience = "shared".parse().unwrap();
        let wren: Audience = "wren".parse().unwrap();
        let group: Audience = "ann,wren".parse().unwrap();
        assert_eq!(group, "wren,ann,wren".parse().unwrap());
        for bad in ["", "shared,wren", "Wren", "wren,", "wren, ann"] {
            assert!(bad.parse::<Audience>().is_err());
        }
        for (id, marker) in [
            ("s", shared.marker("s")),
            ("w", wren.marker("w")),
            ("g", group.marker("g")),
            ("q", Marker::quarantine("q")),
        ] {
            put(&c, "entry", id, &marker).unwrap();
        }
        for audience in [&shared, &wren, &group] {
            for id in ["s", "w", "g", "q", "missing"] {
                assert_eq!(
                    audience.admits_entry(&c, id).unwrap(),
                    (*audience == shared && id == "s")
                        || (*audience == wren && id == "w")
                        || (*audience == group && id == "g")
                );
            }
        }
        put(&c, "entry", "w", &shared.marker("conflicting")).unwrap();
        assert!(!wren.admits_entry(&c, "w").unwrap());
        assert!(!shared.admits_entry(&c, "w").unwrap());
        c.execute(
            "INSERT INTO privacy_retired VALUES ('entry','w','conflicting')",
            [],
        )
        .unwrap();
        assert!(wren.admits_entry(&c, "w.md").unwrap());
    }
    const LIVE_SHAPED: &str = r#"{"conversations":[],"people":{"orin":{"handles":[]},"mira":{"handles":[]},
        "nella":{"handles":[]},"torin":{"handles":[]},"wren":{"handles":[]}},
        "private_tags":{"audience-elira-health":["orin","mira","nella"],
        "audience-household":["orin","mira"],"audience-orin":["orin"],
        "wren-private":["wren"],"audience-nella":["orin","nella"],
        "audience-torin":["orin","torin"],"audience-wren":["orin","wren"],
        "audience-study":["orin","torin","nella"]}}"#;
    #[test]
    fn seven_live_shaped_tags_give_the_removed_constants_categories_in_order() {
        // The constant this replaced (palace-cli audience.rs CATEGORIES), names substituted.
        let removed: [(&str, &[&str]); 7] = [
            ("audience-elira-health", &["orin", "mira", "nella"]),
            ("audience-household", &["orin", "mira"]),
            ("audience-orin", &["orin"]),
            ("audience-nella", &["orin", "nella"]),
            ("audience-torin", &["orin", "torin"]),
            ("audience-wren", &["orin", "wren"]),
            ("audience-study", &["orin", "torin", "nella"]),
        ];
        let expected: Vec<(String, Vec<String>)> = removed
            .iter()
            .map(|(tag, people)| {
                let members = people.iter().map(|p| (*p).to_owned());
                ((*tag).to_owned(), members.collect())
            })
            .collect();
        let people = People::parse(LIVE_SHAPED).unwrap();
        assert_eq!(people.audience_categories(), expected);
    }
    #[test]
    fn a_map_without_audience_tags_offers_no_categories() {
        let bare = r#"{"conversations":[],"people":{"wren":{"handles":[]}},"private_tags":{"wren-private":["wren"]}}"#;
        assert!(
            People::parse(bare)
                .unwrap()
                .audience_categories()
                .is_empty()
        );
        let none = r#"{"conversations":[],"people":{"wren":{"handles":[]}}}"#;
        assert!(
            People::parse(none)
                .unwrap()
                .audience_categories()
                .is_empty()
        );
    }
    #[test]
    fn private_tags_parse_formats_union_and_never_widen() {
        let p = People::parse(r#"{"people":{"ann":{"handles":[]},"wren":{"handles":[]}},"conversations":[],"private_tags":{"wren-private":["wren"],"ann-private":["ann"]}}"#).unwrap();
        for text in [
            "# Invented\nTags: ordinary, WREN-private.\n",
            "---\ntags: [ordinary, 'wren-private']\n---\n# Invented",
            "---\ntags:\n - ordinary\n - \"wren-private\"\n---\n# Invented",
        ] {
            let (m, reason) = p.tagged(&Marker::shared("synthetic"), &entry_tags(text));
            assert_eq!(reason.as_deref(), Some("tag:wren-private"));
            assert!(m.private && m.admits(&["wren".into()]) && !m.admits(&["ann".into()]));
        }
        let tags = entry_tags("Tags: wren-private, ann-private.");
        let (m, _) = p.tagged(&Marker::shared("synthetic"), &tags);
        assert_eq!(m.audience, BTreeSet::from(["ann".into(), "wren".into()]));
        let private = p.tagged(&m, &entry_tags("Tags: wren-private.")).0;
        assert_eq!(
            p.tagged(&private, &tags).0.audience,
            BTreeSet::from(["wren".into()])
        );
        assert_eq!(p.tagged(&private, &BTreeSet::new()).0, private);
        assert!(
            !entry_tags("# Invented\nA prose mention of wren-private.").contains("wren-private")
        );
        for mapping in [
            r#"{"wren-private":[]}"#,
            r#"{"wren-private":["unknown"]}"#,
            r#"[]"#,
        ] {
            assert!(People::parse(&format!(r#"{{"people":{{"ann":{{"handles":[]}}}},"conversations":[],"private_tags":{mapping}}}"#)).is_err());
        }
    }
    fn people() -> People {
        People::parse(r#"{"people":{"ann":{"handles":["signal:+12025550100"],"owner":true},"wren":{"handles":["signal:+12025550101","email:wren@example.invalid"],"private":true}},"conversations":[{"match":"agent:main:cron:synthetic","people":["wren"],"private":true},{"match":"group:synthetic","people":["ann","wren"]}]}"#).unwrap()
    }
    #[test]
    fn identities_are_exact_and_cross_platform() {
        let p = people();
        assert!(
            p.resolve("agent:main:signal:direct:+12025550101", None)
                .private
        );
        assert_eq!(
            p.resolve("dm", Some("email:wren@example.invalid")).people,
            BTreeSet::from(["wren".into()])
        );
        assert!(
            p.resolve("agent:main:cron:synthetic", None)
                .admits(&["wren".into()])
        );
        assert!(p.resolve("Wren", None).quarantine);
        assert!(
            !p.resolve("agent:main:signal:direct:+12025550101", None)
                .admits(&["wren".into(), "ann".into()])
        );
    }
    #[test]
    fn discovery_orders_exact_sets_and_skips_conflicting_retired_and_quarantine() {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(SCHEMA).unwrap();
        for (id, audience) in [
            ("z", "wren"),
            ("a", "ann,wren"),
            ("retired", "cy"),
            ("mixed", "ann"),
        ] {
            put(
                &c,
                "entry",
                id,
                &audience.parse::<Audience>().unwrap().marker(id),
            )
            .unwrap();
        }
        put(&c, "entry", "mixed", &Marker::shared("another")).unwrap();
        put(&c, "entry", "quarantine", &Marker::quarantine("q")).unwrap();
        c.execute(
            "INSERT INTO privacy_retired VALUES ('entry','retired','retired')",
            [],
        )
        .unwrap();
        let names = ["z", "a", "retired", "mixed", "quarantine", "missing"].map(String::from);
        assert_eq!(
            Audience::discover(&c, &names).unwrap(),
            ["shared", "ann,wren", "wren"].map(|a| a.parse().unwrap())
        );
    }
    #[test]
    fn private_meets_and_sql_admission() {
        let private = people().resolve("agent:main:signal:direct:+12025550101", None);
        let derived = Marker::meet(&[Marker::shared("shared"), private], "derived");
        assert!(derived.private && derived.admits(&["wren".into()]));
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(SCHEMA).unwrap();
        put(&c, "triple", "t", &derived).unwrap();
        put(&c, "triple", "t", &derived).unwrap();
        for (readers, n) in [
            (vec![], 0),
            (vec!["ann".into()], 0),
            (vec!["wren".into()], 1),
            (vec!["wren".into(), "ann".into()], 0),
        ] {
            let count: i64 = c
                .query_row(
                    &format!(
                        "SELECT count(*) FROM (SELECT 't' AS id) t WHERE {}",
                        visible("triple", "t.id", &readers)
                    ),
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(count, n);
        }
        put(&c, "triple", "t", &Marker::shared("independent")).unwrap();
        assert!(!super::private(&c, "triple", "t").unwrap());
    }
    #[test]
    fn backfill_is_planned_read_only_and_idempotent() {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(include_str!("schema.sql")).unwrap();
        crate::claims::migrate(&c).unwrap();
        crate::claims::memory_types(&c, std::path::Path::new("unused"), false).unwrap();
        crate::items::migrate(&c).unwrap();
        c.execute_batch("ATTACH DATABASE ':memory:' AS kg; CREATE TABLE kg.triples(id TEXT,source_closet TEXT);
            INSERT INTO kg.triples VALUES ('legacy-private','{\"transcript\":\"/synthetic/transcripts/archive/test.jsonl:1\"}'),('unknown',NULL);").unwrap();
        let plan = backfill(&c, &people()).unwrap();
        assert_eq!(plan.len(), 2);
        assert!(!plan[0].2.private && plan[0].2.audience.is_empty());
        assert!(plan[1].2.quarantine);
        assert!(!crate::auto::has_table(&c, "privacy_markers").unwrap());
        c.execute_batch(SCHEMA).unwrap();
        for _ in 0..2 {
            for (kind, id, m) in &plan {
                if markers(&c, kind, id).unwrap().is_empty() {
                    put(&c, kind, id, m).unwrap();
                }
            }
        }
        assert_eq!(markers(&c, "triple", "legacy-private").unwrap().len(), 1);
    }
}
