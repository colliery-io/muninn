//! Graph storage via graphqlite

/// Code graph storing symbols and relationships
pub struct CodeGraph {
    // graphqlite connection
}

impl CodeGraph {
    /// Open or create a code graph at the given path
    pub fn open(_path: &std::path::Path) -> anyhow::Result<Self> {
        todo!("Implement graphqlite storage")
    }
}
