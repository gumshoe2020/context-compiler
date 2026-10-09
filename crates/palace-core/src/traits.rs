pub trait GraphStore {
    type Error;
    fn query_entity(
        &self,
        name: &str,
        direction: crate::triple::Direction,
        as_of: Option<&str>,
    ) -> Result<Vec<crate::triple::Triple>, Self::Error>;
    fn list_entities(&self) -> Result<Vec<crate::entity::Entity>, Self::Error>;
    fn entity(&self, name: &str) -> Result<Option<crate::entity::Entity>, Self::Error>;

    fn add_triple(
        &mut self,
        arguments: &serde_json::Value,
    ) -> Result<serde_json::Value, Self::Error>;
}
pub trait EvidenceSearch {
    type Error;
    fn search_headers(&self, query: &str, limit: usize) -> Result<Vec<String>, Self::Error>;
}
pub trait Judge {
    type Error;
    fn judge(&self, question: &crate::canary::Question) -> Result<serde_json::Value, Self::Error>;
}
/// Plain text in, plain text out (rollup thread selection and body writing).
pub trait Model {
    type Error;
    fn complete(&self, prompt: &str) -> Result<String, Self::Error>;
}
pub trait Clock {
    fn now(&self) -> String;
}

pub trait GraphHistory: GraphStore {
    fn stats(&self) -> Result<serde_json::Value, Self::Error>;
    fn timeline(&self, entity: Option<&str>) -> Result<Vec<crate::triple::Triple>, Self::Error>;
}
pub trait GraphMutation: GraphStore {
    fn invalidate(
        &mut self,
        subject: &str,
        predicate: &str,
        object: &str,
    ) -> Result<serde_json::Value, Self::Error>;
    fn retire_entity(
        &mut self,
        name: &str,
        reason: &str,
        by: &str,
    ) -> Result<serde_json::Value, Self::Error>;
}

pub trait Searcher {
    type Error;
    fn search(
        &self,
        query: &str,
        limit: usize,
        max_distance: f64,
    ) -> Result<Vec<crate::search_hit::SearchHit>, Self::Error>;
}
pub trait DiaryReader {
    type Error;
    fn header(&self, source_path: &str) -> Result<String, Self::Error>;
    fn names(&self) -> Result<Vec<String>, Self::Error>;
    fn read(&self, name: &str) -> Result<Option<String>, Self::Error>;
}
