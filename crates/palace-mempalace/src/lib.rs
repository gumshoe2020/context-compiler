#![forbid(unsafe_code)]
pub mod client;
pub mod diary;
pub mod diary_only;
pub mod drawers;
pub mod mine;
pub mod version;
use palace_core::{
    search_hit::SearchHit,
    traits::{DiaryReader, EvidenceSearch, Searcher},
};
use serde_json::json;
use std::{cell::RefCell, path::Path};
pub type Error = Box<dyn std::error::Error + Send + Sync>;

/// MemPalace itself is unavailable (spawn, handshake, pin, transport, or `mine` failure), as
/// opposed to a broken run. Callers map it to exit 3 by type, never by message.
#[derive(Debug)]
pub struct Unavailable(pub String);
impl std::fmt::Display for Unavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "MemPalace unavailable: {}", self.0)
    }
}
impl std::error::Error for Unavailable {}
impl Unavailable {
    pub fn error(reason: impl std::fmt::Display) -> Error {
        Box::new(Self(reason.to_string()))
    }
    /// True when `error` is (or wraps nothing but) an `Unavailable`.
    pub fn is(error: &Error) -> bool {
        error.downcast_ref::<Self>().is_some()
    }
}

pub struct MemPalace {
    client: RefCell<client::Client>,
}
impl MemPalace {
    pub fn open(
        palace: &Path,
        executable: &Path,
        version_executable: &Path,
        allow_version: bool,
    ) -> Result<Self, Error> {
        version::check(version_executable, allow_version)?;
        let mut client = client::Client::spawn(
            executable,
            &["--palace".as_ref(), palace.as_os_str()],
            palace,
        )?;
        client.initialize("mempalace_search", allow_version)?;
        Ok(Self {
            client: RefCell::new(client),
        })
    }
    /// A search-only connection with no pin script and no version gate (`palace context`): the
    /// server in its own process group, every request ending by `deadline`, the group killed
    /// on a failure or when the connection is dropped.
    pub fn open_unpinned(
        palace: &Path,
        executable: &Path,
        deadline: std::time::Instant,
    ) -> Result<Self, Error> {
        let mut client = client::Client::spawn_until(
            executable,
            &["--palace".as_ref(), palace.as_os_str()],
            palace,
            deadline,
        )?;
        client.initialize_unpinned("mempalace_search")?;
        Ok(Self {
            client: RefCell::new(client),
        })
    }
}
impl MemPalace {
    /// `Some(reason)` once the MemPalace transport itself has failed (not a content error).
    pub fn unavailable(&self) -> Option<String> {
        self.client.borrow().failure().map(str::to_owned)
    }
    /// The MCP server's `initialize.serverInfo.version`.
    pub fn server_version(&self) -> Option<String> {
        self.client.borrow().server_version().map(str::to_owned)
    }
    /// `mempalace_search` with an optional `wing` filter (the regression harness compares like
    /// with like: the frozen answers were captured with `wing=openclaw`).
    pub fn search_in(
        &self,
        query: &str,
        limit: usize,
        max_distance: f64,
        wing: Option<&str>,
    ) -> Result<Vec<SearchHit>, Error> {
        if query.chars().count() > 250 || !max_distance.is_finite() || limit == 0 || limit > 100 {
            return Err("invalid MemPalace search arguments".into());
        }
        let mut arguments = json!({"query":query,"limit":limit,"max_distance":max_distance});
        if let Some(wing) = wing {
            palace_core::filters::optional_name(Some(wing))?;
            arguments["wing"] = json!(wing);
        }
        let result = self.client.borrow_mut().request(
            "tools/call",
            json!({"name":"mempalace_search","arguments":arguments}),
        )?;
        if result["isError"] == true {
            return Err(format!("MemPalace search failed: {result}").into());
        }
        let text = result["content"][0]["text"]
            .as_str()
            .ok_or("missing search text envelope")?;
        search_rows(serde_json::from_str(text)?)
    }
}
impl Searcher for MemPalace {
    type Error = Error;
    fn search(
        &self,
        query: &str,
        limit: usize,
        max_distance: f64,
    ) -> Result<Vec<SearchHit>, Error> {
        self.search_in(query, limit, max_distance, None)
    }
}
/// Creates `<palace>/knowledge_graph.sqlite3` through MemPalace itself (`mempalace_kg_stats`
/// opens `KnowledgeGraph`, which applies its own schema): the compiler never runs KG DDL.
pub fn create_kg(mcp: &Path, palace: &Path) -> Result<std::path::PathBuf, Error> {
    let mut client =
        client::Client::spawn(mcp, &["--palace".as_ref(), palace.as_os_str()], palace)?;
    client.initialize("mempalace_kg_stats", false)?;
    let result = client.request(
        "tools/call",
        json!({"name":"mempalace_kg_stats","arguments":{}}),
    )?;
    if result["isError"] == true {
        return Err(format!("mempalace_kg_stats failed: {result}").into());
    }
    let kg = palace.join("knowledge_graph.sqlite3");
    if !kg.is_file() {
        return Err("MemPalace did not create knowledge_graph.sqlite3".into());
    }
    Ok(kg)
}
pub struct Evidence<S = MemPalace> {
    pub searcher: S,
    pub diary: diary::Diary,
}
impl<S: Searcher<Error = Error>> EvidenceSearch for Evidence<S> {
    type Error = Error;
    fn search_headers(&self, query: &str, limit: usize) -> Result<Vec<String>, Error> {
        Ok(self
            .searcher
            .search(query, limit, 1.5)?
            .into_iter()
            .map(|row| {
                // Unavailable/out-of-root provenance is never opened; use the returned hit text,
                // matching the existing gate's missing-header fallback.
                self.diary.header(&row.source_path).unwrap_or(row.text)
            })
            .collect())
    }
}

pub fn search_rows(result: serde_json::Value) -> Result<Vec<SearchHit>, Error> {
    if let Some(error) = result.get("error") {
        return Err(format!("MemPalace search error: {error}").into());
    }
    if result["vector_disabled"] == true {
        return Err("MemPalace vector_disabled: refusing lexical-only evidence".into());
    }
    let rows: Vec<SearchHit> = serde_json::from_value(result["results"].clone())?;
    Ok(rows
        .into_iter()
        .filter(|row| {
            palace_core::filters::optional_source(Some(&row.source_file)).is_ok()
                && palace_core::filters::optional_name(Some(&row.wing)).is_ok()
                && palace_core::filters::optional_name(Some(&row.room)).is_ok()
        })
        .collect())
}
#[cfg(test)]
mod search_tests {
    #[test]
    fn error_and_lexical_fallback_fail_closed() {
        assert!(
            super::search_rows(serde_json::json!({"error":"bad filter"}))
                .unwrap_err()
                .to_string()
                .contains("bad filter")
        );
        assert!(
            super::search_rows(serde_json::json!({"vector_disabled":true,"results":[]})).is_err()
        );
        assert!(
            super::search_rows(serde_json::json!({"results":[]}))
                .unwrap()
                .is_empty()
        );
    }
}
