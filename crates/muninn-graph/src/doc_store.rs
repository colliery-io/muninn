//! Documentation storage for dependency libraries.
//!
//! Stores extracted documentation from external libraries (Rust crates, Python packages)
//! with support for full-text search (FTS5) and embeddings for semantic search.

use std::path::Path;

use graphqlite::Graph;

/// Error type for doc store operations.
#[derive(Debug, thiserror::Error)]
pub enum DocStoreError {
    #[error("Database error: {0}")]
    Database(#[from] graphqlite::Error),
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("Library not found: {0}")]
    LibraryNotFound(String),
    #[error("Invalid data: {0}")]
    InvalidData(String),
}

pub type Result<T> = std::result::Result<T, DocStoreError>;

/// Ecosystem for a library.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ecosystem {
    Rust,
    Python,
    /// Web documentation from llms.txt or similar sources.
    Web,
}

impl Ecosystem {
    pub fn as_str(&self) -> &'static str {
        match self {
            Ecosystem::Rust => "rust",
            Ecosystem::Python => "python",
            Ecosystem::Web => "web",
        }
    }

    // Intentionally returns Option<Self> rather than impl-ing FromStr,
    // because callers want "no, not one of ours" to be a non-error signal.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "rust" => Some(Ecosystem::Rust),
            "python" => Some(Ecosystem::Python),
            "web" => Some(Ecosystem::Web),
            _ => None,
        }
    }
}

/// Type of documented item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemType {
    Module,
    Class,
    Struct,
    Enum,
    Trait,
    Function,
    Method,
    Constant,
    Type,
    /// A documentation page (from llms.txt or similar).
    Page,
    /// A guide or tutorial.
    Guide,
}

impl ItemType {
    pub fn as_str(&self) -> &'static str {
        match self {
            ItemType::Module => "module",
            ItemType::Class => "class",
            ItemType::Struct => "struct",
            ItemType::Enum => "enum",
            ItemType::Trait => "trait",
            ItemType::Function => "function",
            ItemType::Method => "method",
            ItemType::Constant => "constant",
            ItemType::Type => "type",
            ItemType::Page => "page",
            ItemType::Guide => "guide",
        }
    }

    // Intentionally returns Option<Self> rather than impl-ing FromStr,
    // because callers want "no, not one of ours" to be a non-error signal.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "module" => Some(ItemType::Module),
            "class" => Some(ItemType::Class),
            "struct" => Some(ItemType::Struct),
            "enum" => Some(ItemType::Enum),
            "trait" => Some(ItemType::Trait),
            "function" => Some(ItemType::Function),
            "method" => Some(ItemType::Method),
            "constant" => Some(ItemType::Constant),
            "type" => Some(ItemType::Type),
            "page" => Some(ItemType::Page),
            "guide" => Some(ItemType::Guide),
            _ => None,
        }
    }
}

/// Metadata for an indexed library.
#[derive(Debug, Clone)]
pub struct DocLibrary {
    pub id: i64,
    pub library: String,
    pub ecosystem: Ecosystem,
    pub version: String,
    pub source_url: Option<String>,
    pub indexed_at: String,
}

/// A documentation chunk for a library item.
#[derive(Debug, Clone)]
pub struct DocChunk {
    pub id: i64,
    pub library_id: i64,
    pub item_path: String,
    pub item_type: ItemType,
    pub doc_text: String,
    pub signature: Option<String>,
    // Note: embedding stored separately as BLOB
}

/// A documentation chunk with a relevance score.
#[derive(Debug, Clone)]
pub struct ScoredChunk {
    pub chunk: DocChunk,
    pub score: f64,
}

/// Search mode for hybrid search.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SearchMode {
    /// Full-text search only (FTS5).
    Fts,
    /// Semantic search only (embeddings).
    Semantic,
    /// Hybrid search combining FTS and semantic with RRF fusion.
    #[default]
    Hybrid,
}

/// RRF (Reciprocal Rank Fusion) constant. Standard value is 60.
const RRF_K: f64 = 60.0;

/// Convert a list of chunks to scored chunks with rank-based scores.
fn chunks_to_scored(chunks: Vec<DocChunk>) -> Vec<ScoredChunk> {
    chunks
        .into_iter()
        .enumerate()
        .map(|(rank, chunk)| ScoredChunk {
            chunk,
            // RRF score based on single list rank
            score: 1.0 / (RRF_K + rank as f64 + 1.0),
        })
        .collect()
}

/// Apply Reciprocal Rank Fusion (RRF) to combine two ranked lists.
///
/// RRF is a simple but effective method for combining search results from
/// different ranking systems. It's particularly good because:
/// - It doesn't require score normalization between systems
/// - It's robust to outliers
/// - It tends to promote items that rank well in both lists
///
/// Formula: RRF_score(d) = Σ 1 / (k + rank(d))
/// where k is a constant (typically 60) and rank is 1-indexed.
fn rrf_fusion(
    fts_results: &[DocChunk],
    semantic_results: &[DocChunk],
    limit: usize,
) -> Vec<ScoredChunk> {
    use std::collections::HashMap;

    // Map chunk ID to (chunk, accumulated RRF score)
    let mut scores: HashMap<i64, (DocChunk, f64)> = HashMap::new();

    // Add FTS scores (rank is 0-indexed, convert to 1-indexed for RRF)
    for (rank, chunk) in fts_results.iter().enumerate() {
        let rrf_score = 1.0 / (RRF_K + rank as f64 + 1.0);
        scores
            .entry(chunk.id)
            .and_modify(|(_, score)| *score += rrf_score)
            .or_insert_with(|| (chunk.clone(), rrf_score));
    }

    // Add semantic scores
    for (rank, chunk) in semantic_results.iter().enumerate() {
        let rrf_score = 1.0 / (RRF_K + rank as f64 + 1.0);
        scores
            .entry(chunk.id)
            .and_modify(|(_, score)| *score += rrf_score)
            .or_insert_with(|| (chunk.clone(), rrf_score));
    }

    // Convert to scored chunks and sort by score descending
    let mut results: Vec<ScoredChunk> = scores
        .into_values()
        .map(|(chunk, score)| ScoredChunk { chunk, score })
        .collect();

    results.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    results.truncate(limit);
    results
}

/// Input for inserting a new doc chunk.
#[derive(Debug, Clone)]
pub struct DocChunkInput {
    pub item_path: String,
    pub item_type: ItemType,
    pub doc_text: String,
    pub signature: Option<String>,
    pub embedding: Option<Vec<f32>>,
}

/// Documentation storage for external libraries.
///
/// Uses the same SQLite database as the code graph but with separate tables
/// for documentation chunks and library metadata.
pub struct DocStore {
    graph: Graph,
}

impl DocStore {
    /// Open or create a doc store at the specified path.
    ///
    /// This opens the same database file used by GraphStore, adding doc tables if needed.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let graph = Graph::open(path)?;
        let store = Self { graph };
        store.init_schema()?;
        Ok(store)
    }

    /// Create an in-memory doc store (for testing).
    pub fn open_in_memory() -> Result<Self> {
        let graph = Graph::open_in_memory()?;
        let store = Self { graph };
        store.init_schema()?;
        Ok(store)
    }

    /// Get the underlying rusqlite connection for parameterized queries.
    fn sqlite(&self) -> &rusqlite::Connection {
        self.graph.connection().sqlite_connection()
    }

    /// Initialize the doc store schema (creates tables if they don't exist).
    fn init_schema(&self) -> Result<()> {
        let conn = self.sqlite();

        // Create doc_libraries table
        conn.execute(
            "CREATE TABLE IF NOT EXISTS doc_libraries (
                id INTEGER PRIMARY KEY,
                library TEXT NOT NULL UNIQUE,
                ecosystem TEXT NOT NULL,
                version TEXT NOT NULL,
                source_url TEXT,
                indexed_at TEXT NOT NULL DEFAULT (datetime('now'))
            )",
            [],
        )?;

        // Create doc_chunks table
        conn.execute(
            "CREATE TABLE IF NOT EXISTS doc_chunks (
                id INTEGER PRIMARY KEY,
                library_id INTEGER NOT NULL REFERENCES doc_libraries(id) ON DELETE CASCADE,
                item_path TEXT NOT NULL,
                item_type TEXT NOT NULL,
                doc_text TEXT NOT NULL,
                signature TEXT,
                embedding BLOB,
                UNIQUE(library_id, item_path)
            )",
            [],
        )?;

        // Create index for faster library lookups
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_doc_chunks_library
             ON doc_chunks(library_id)",
            [],
        )?;

        // Create FTS5 virtual table for full-text search
        // Note: FTS5 tables can't use IF NOT EXISTS, so we check first
        let fts_exists: i64 = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master
             WHERE type='table' AND name='doc_chunks_fts'",
            [],
            |row| row.get(0),
        )?;

        if fts_exists == 0 {
            conn.execute(
                "CREATE VIRTUAL TABLE doc_chunks_fts USING fts5(
                    item_path,
                    doc_text,
                    content='doc_chunks',
                    content_rowid='id'
                )",
                [],
            )?;

            // Create triggers to keep FTS in sync with doc_chunks
            conn.execute(
                "CREATE TRIGGER IF NOT EXISTS doc_chunks_ai AFTER INSERT ON doc_chunks BEGIN
                    INSERT INTO doc_chunks_fts(rowid, item_path, doc_text)
                    VALUES (new.id, new.item_path, new.doc_text);
                END",
                [],
            )?;

            conn.execute(
                "CREATE TRIGGER IF NOT EXISTS doc_chunks_ad AFTER DELETE ON doc_chunks BEGIN
                    INSERT INTO doc_chunks_fts(doc_chunks_fts, rowid, item_path, doc_text)
                    VALUES ('delete', old.id, old.item_path, old.doc_text);
                END",
                [],
            )?;

            conn.execute(
                "CREATE TRIGGER IF NOT EXISTS doc_chunks_au AFTER UPDATE ON doc_chunks BEGIN
                    INSERT INTO doc_chunks_fts(doc_chunks_fts, rowid, item_path, doc_text)
                    VALUES ('delete', old.id, old.item_path, old.doc_text);
                    INSERT INTO doc_chunks_fts(rowid, item_path, doc_text)
                    VALUES (new.id, new.item_path, new.doc_text);
                END",
                [],
            )?;
        }

        Ok(())
    }

    /// Insert or update a library's metadata.
    ///
    /// Returns the library ID.
    pub fn upsert_library(
        &self,
        library: &str,
        ecosystem: Ecosystem,
        version: &str,
        source_url: Option<&str>,
    ) -> Result<i64> {
        let conn = self.sqlite();

        conn.execute(
            "INSERT INTO doc_libraries (library, ecosystem, version, source_url, indexed_at)
             VALUES (?1, ?2, ?3, ?4, datetime('now'))
             ON CONFLICT(library) DO UPDATE SET
                ecosystem = excluded.ecosystem,
                version = excluded.version,
                source_url = excluded.source_url,
                indexed_at = datetime('now')",
            rusqlite::params![
                library,
                ecosystem.as_str(),
                version,
                source_url.unwrap_or("")
            ],
        )?;

        let id: i64 = conn.query_row(
            "SELECT id FROM doc_libraries WHERE library = ?1",
            [library],
            |row| row.get(0),
        )?;

        Ok(id)
    }

    /// Get a library by name.
    pub fn get_library(&self, library: &str) -> Result<Option<DocLibrary>> {
        let conn = self.sqlite();

        let result = conn.query_row(
            "SELECT id, library, ecosystem, version, source_url, indexed_at
             FROM doc_libraries WHERE library = ?1",
            [library],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, String>(5)?,
                ))
            },
        );

        match result {
            Ok((id, lib, eco_str, version, source_url, indexed_at)) => {
                let ecosystem = Ecosystem::from_str(&eco_str).ok_or_else(|| {
                    DocStoreError::InvalidData(format!("Invalid ecosystem: {}", eco_str))
                })?;

                Ok(Some(DocLibrary {
                    id,
                    library: lib,
                    ecosystem,
                    version,
                    source_url: source_url.filter(|s| !s.is_empty()),
                    indexed_at,
                }))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// List all indexed libraries.
    pub fn list_libraries(&self) -> Result<Vec<DocLibrary>> {
        let conn = self.sqlite();

        let mut stmt = conn.prepare(
            "SELECT id, library, ecosystem, version, source_url, indexed_at
             FROM doc_libraries ORDER BY library",
        )?;

        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, String>(5)?,
            ))
        })?;

        let mut libraries = Vec::new();
        for row in rows {
            let (id, lib, eco_str, version, source_url, indexed_at) = row?;
            let ecosystem = Ecosystem::from_str(&eco_str).ok_or_else(|| {
                DocStoreError::InvalidData(format!("Invalid ecosystem: {}", eco_str))
            })?;

            libraries.push(DocLibrary {
                id,
                library: lib,
                ecosystem,
                version,
                source_url: source_url.filter(|s| !s.is_empty()),
                indexed_at,
            });
        }

        Ok(libraries)
    }

    /// Delete a library and all its chunks.
    pub fn delete_library(&self, library: &str) -> Result<bool> {
        let conn = self.sqlite();

        let rows_affected =
            conn.execute("DELETE FROM doc_libraries WHERE library = ?1", [library])?;

        Ok(rows_affected > 0)
    }

    /// Insert a documentation chunk.
    pub fn insert_chunk(&self, library_id: i64, chunk: &DocChunkInput) -> Result<i64> {
        let conn = self.sqlite();

        // For now, store embedding as NULL - will be populated by embedding pipeline
        conn.execute(
            "INSERT INTO doc_chunks (library_id, item_path, item_type, doc_text, signature, embedding)
             VALUES (?1, ?2, ?3, ?4, ?5, NULL)
             ON CONFLICT(library_id, item_path) DO UPDATE SET
                item_type = excluded.item_type,
                doc_text = excluded.doc_text,
                signature = excluded.signature",
            rusqlite::params![
                library_id,
                &chunk.item_path,
                chunk.item_type.as_str(),
                &chunk.doc_text,
                chunk.signature.as_deref().unwrap_or(""),
            ],
        )?;

        let id: i64 = conn.query_row(
            "SELECT id FROM doc_chunks WHERE library_id = ?1 AND item_path = ?2",
            rusqlite::params![library_id, &chunk.item_path],
            |row| row.get(0),
        )?;

        Ok(id)
    }

    /// Insert multiple documentation chunks in a batch.
    pub fn insert_chunks_batch(&self, library_id: i64, chunks: &[DocChunkInput]) -> Result<usize> {
        if chunks.is_empty() {
            return Ok(0);
        }

        let conn = self.sqlite();
        conn.execute("BEGIN", [])?;

        let mut inserted = 0;
        for chunk in chunks {
            let result = conn.execute(
                "INSERT INTO doc_chunks (library_id, item_path, item_type, doc_text, signature, embedding)
                 VALUES (?1, ?2, ?3, ?4, ?5, NULL)
                 ON CONFLICT(library_id, item_path) DO UPDATE SET
                    item_type = excluded.item_type,
                    doc_text = excluded.doc_text,
                    signature = excluded.signature",
                rusqlite::params![
                    library_id,
                    &chunk.item_path,
                    chunk.item_type.as_str(),
                    &chunk.doc_text,
                    chunk.signature.as_deref().unwrap_or(""),
                ],
            );

            if let Err(e) = result {
                let _ = conn.execute("ROLLBACK", []);
                return Err(e.into());
            }
            inserted += 1;
        }

        conn.execute("COMMIT", [])?;
        Ok(inserted)
    }

    /// Search documentation using full-text search.
    pub fn search_fts(&self, library: &str, query: &str, limit: usize) -> Result<Vec<DocChunk>> {
        let lib = self
            .get_library(library)?
            .ok_or_else(|| DocStoreError::LibraryNotFound(library.to_string()))?;

        let conn = self.sqlite();

        // Use FTS5 MATCH syntax
        let mut stmt = conn.prepare(
            "SELECT dc.id, dc.library_id, dc.item_path, dc.item_type, dc.doc_text, dc.signature
             FROM doc_chunks dc
             JOIN doc_chunks_fts fts ON dc.id = fts.rowid
             WHERE dc.library_id = ?1 AND doc_chunks_fts MATCH ?2
             ORDER BY rank
             LIMIT ?3",
        )?;

        let rows = stmt.query_map(rusqlite::params![lib.id, query, limit as i64], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, Option<String>>(5)?,
            ))
        })?;

        self.rows_to_chunks(rows)
    }

    /// Search documentation using semantic (embedding) search.
    ///
    /// **Note**: This is a stub that returns empty results until sqlite-vec
    /// is integrated and embeddings are populated. The embedding column exists
    /// but is not yet used.
    ///
    /// When implemented, this will:
    /// 1. Accept a query embedding vector
    /// 2. Use sqlite-vec to find nearest neighbors
    /// 3. Return chunks ordered by cosine similarity
    #[allow(unused_variables)]
    pub fn search_semantic(
        &self,
        library: &str,
        query_embedding: &[f32],
        limit: usize,
    ) -> Result<Vec<DocChunk>> {
        let _lib = self
            .get_library(library)?
            .ok_or_else(|| DocStoreError::LibraryNotFound(library.to_string()))?;

        // TODO: Implement semantic search when sqlite-vec is integrated
        // For now, return empty results - hybrid search will fall back to FTS only
        Ok(Vec::new())
    }

    /// Perform hybrid search combining FTS and semantic search with RRF fusion.
    ///
    /// This method:
    /// 1. Runs FTS5 full-text search
    /// 2. Runs semantic (embedding) search (if query_embedding provided)
    /// 3. Combines results using Reciprocal Rank Fusion (RRF)
    ///
    /// RRF formula: score(d) = Σ 1 / (k + rank(d))
    /// where k is a constant (60) and rank is the position in each result list.
    ///
    /// If semantic search is not available (no embeddings), falls back to FTS only.
    pub fn search_hybrid(
        &self,
        library: &str,
        query: &str,
        query_embedding: Option<&[f32]>,
        limit: usize,
        mode: SearchMode,
    ) -> Result<Vec<ScoredChunk>> {
        match mode {
            SearchMode::Fts => {
                let chunks = self.search_fts(library, query, limit)?;
                Ok(chunks_to_scored(chunks))
            }
            SearchMode::Semantic => {
                if let Some(embedding) = query_embedding {
                    let chunks = self.search_semantic(library, embedding, limit)?;
                    Ok(chunks_to_scored(chunks))
                } else {
                    // No embedding provided, return empty
                    Ok(Vec::new())
                }
            }
            SearchMode::Hybrid => {
                // Fetch more results than needed for better fusion
                let fetch_limit = limit * 3;

                // Get FTS results
                let fts_results = self.search_fts(library, query, fetch_limit)?;

                // Get semantic results (if embedding provided)
                let semantic_results = if let Some(embedding) = query_embedding {
                    self.search_semantic(library, embedding, fetch_limit)?
                } else {
                    Vec::new()
                };

                // If semantic search returned nothing, fall back to FTS-only
                if semantic_results.is_empty() {
                    let chunks = self.search_fts(library, query, limit)?;
                    return Ok(chunks_to_scored(chunks));
                }

                // Apply RRF fusion
                let fused = rrf_fusion(&fts_results, &semantic_results, limit);
                Ok(fused)
            }
        }
    }

    /// Search with default hybrid mode and no semantic embedding.
    ///
    /// This is a convenience method that uses FTS-only search (since no embedding
    /// is provided). Use `search_hybrid` for full control.
    pub fn search(&self, library: &str, query: &str, limit: usize) -> Result<Vec<ScoredChunk>> {
        self.search_hybrid(library, query, None, limit, SearchMode::default())
    }

    /// Get all chunks for a library.
    pub fn get_chunks(&self, library: &str) -> Result<Vec<DocChunk>> {
        let lib = self
            .get_library(library)?
            .ok_or_else(|| DocStoreError::LibraryNotFound(library.to_string()))?;

        let conn = self.sqlite();

        let mut stmt = conn.prepare(
            "SELECT id, library_id, item_path, item_type, doc_text, signature
             FROM doc_chunks WHERE library_id = ?1
             ORDER BY item_path",
        )?;

        let rows = stmt.query_map([lib.id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, Option<String>>(5)?,
            ))
        })?;

        self.rows_to_chunks(rows)
    }

    /// Get chunk count for a library.
    pub fn chunk_count(&self, library: &str) -> Result<usize> {
        let lib = self
            .get_library(library)?
            .ok_or_else(|| DocStoreError::LibraryNotFound(library.to_string()))?;

        let conn = self.sqlite();

        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM doc_chunks WHERE library_id = ?1",
            [lib.id],
            |row| row.get(0),
        )?;

        Ok(count as usize)
    }

    /// Convert query result rows to DocChunk structs.
    fn rows_to_chunks(
        &self,
        rows: rusqlite::MappedRows<
            '_,
            impl FnMut(
                &rusqlite::Row<'_>,
            )
                -> rusqlite::Result<(i64, i64, String, String, String, Option<String>)>,
        >,
    ) -> Result<Vec<DocChunk>> {
        let mut chunks = Vec::new();

        for row in rows {
            let (id, library_id, item_path, item_type_str, doc_text, signature) = row?;
            let item_type = ItemType::from_str(&item_type_str).ok_or_else(|| {
                DocStoreError::InvalidData(format!("Invalid item type: {}", item_type_str))
            })?;

            chunks.push(DocChunk {
                id,
                library_id,
                item_path,
                item_type,
                doc_text,
                signature: signature.filter(|s| !s.is_empty()),
            });
        }

        Ok(chunks)
    }

    /// Get the underlying graph for advanced operations.
    pub fn inner(&self) -> &Graph {
        &self.graph
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    #[test]
    #[serial]
    fn test_open_in_memory() {
        let store = DocStore::open_in_memory().expect("Should open in-memory store");
        let libs = store.list_libraries().expect("Should list libraries");
        assert!(libs.is_empty());
    }

    #[test]
    #[serial]
    fn test_upsert_library() {
        let store = DocStore::open_in_memory().unwrap();

        let id = store
            .upsert_library("tokio", Ecosystem::Rust, "1.35.0", Some("https://tokio.rs"))
            .expect("Should insert library");

        assert!(id > 0);

        let lib = store
            .get_library("tokio")
            .unwrap()
            .expect("Should find library");
        assert_eq!(lib.library, "tokio");
        assert_eq!(lib.ecosystem, Ecosystem::Rust);
        assert_eq!(lib.version, "1.35.0");
        assert_eq!(lib.source_url, Some("https://tokio.rs".to_string()));
    }

    #[test]
    #[serial]
    fn test_insert_chunk() {
        let store = DocStore::open_in_memory().unwrap();

        let lib_id = store
            .upsert_library("tokio", Ecosystem::Rust, "1.35.0", None)
            .unwrap();

        let chunk = DocChunkInput {
            item_path: "tokio::spawn".to_string(),
            item_type: ItemType::Function,
            doc_text: "Spawns a new asynchronous task.".to_string(),
            signature: Some("pub fn spawn<F>(future: F) -> JoinHandle<F::Output>".to_string()),
            embedding: None,
        };

        let chunk_id = store
            .insert_chunk(lib_id, &chunk)
            .expect("Should insert chunk");
        assert!(chunk_id > 0);

        let count = store.chunk_count("tokio").unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    #[serial]
    fn test_insert_chunks_batch() {
        let store = DocStore::open_in_memory().unwrap();

        let lib_id = store
            .upsert_library("requests", Ecosystem::Python, "2.31.0", None)
            .unwrap();

        let chunks = vec![
            DocChunkInput {
                item_path: "requests.get".to_string(),
                item_type: ItemType::Function,
                doc_text: "Sends a GET request.".to_string(),
                signature: Some("def get(url, **kwargs)".to_string()),
                embedding: None,
            },
            DocChunkInput {
                item_path: "requests.post".to_string(),
                item_type: ItemType::Function,
                doc_text: "Sends a POST request.".to_string(),
                signature: Some("def post(url, data=None, **kwargs)".to_string()),
                embedding: None,
            },
        ];

        let inserted = store
            .insert_chunks_batch(lib_id, &chunks)
            .expect("Should batch insert");
        assert_eq!(inserted, 2);
        assert_eq!(store.chunk_count("requests").unwrap(), 2);
    }

    #[test]
    #[serial]
    fn test_search_fts() {
        let store = DocStore::open_in_memory().unwrap();

        let lib_id = store
            .upsert_library("tokio", Ecosystem::Rust, "1.35.0", None)
            .unwrap();

        let chunks = vec![
            DocChunkInput {
                item_path: "tokio::spawn".to_string(),
                item_type: ItemType::Function,
                doc_text: "Spawns a new asynchronous task and returns a JoinHandle.".to_string(),
                signature: None,
                embedding: None,
            },
            DocChunkInput {
                item_path: "tokio::runtime::Runtime".to_string(),
                item_type: ItemType::Struct,
                doc_text: "The Tokio runtime for executing async code.".to_string(),
                signature: None,
                embedding: None,
            },
        ];

        store.insert_chunks_batch(lib_id, &chunks).unwrap();

        let results = store.search_fts("tokio", "asynchronous task", 10).unwrap();
        assert!(!results.is_empty());
        assert!(results.iter().any(|c| c.item_path == "tokio::spawn"));
    }

    #[test]
    #[serial]
    fn test_delete_library() {
        let store = DocStore::open_in_memory().unwrap();

        let lib_id = store
            .upsert_library("test-lib", Ecosystem::Rust, "1.0.0", None)
            .unwrap();

        store
            .insert_chunk(
                lib_id,
                &DocChunkInput {
                    item_path: "test::func".to_string(),
                    item_type: ItemType::Function,
                    doc_text: "A test function.".to_string(),
                    signature: None,
                    embedding: None,
                },
            )
            .unwrap();

        assert!(store.delete_library("test-lib").unwrap());
        assert!(store.get_library("test-lib").unwrap().is_none());
    }

    #[test]
    #[serial]
    fn test_list_libraries() {
        let store = DocStore::open_in_memory().unwrap();

        store
            .upsert_library("tokio", Ecosystem::Rust, "1.35.0", None)
            .unwrap();
        store
            .upsert_library("requests", Ecosystem::Python, "2.31.0", None)
            .unwrap();

        let libs = store.list_libraries().unwrap();
        assert_eq!(libs.len(), 2);

        let names: Vec<_> = libs.iter().map(|l| l.library.as_str()).collect();
        assert!(names.contains(&"tokio"));
        assert!(names.contains(&"requests"));
    }

    #[test]
    #[serial]
    fn test_search_hybrid_fts_mode() {
        let store = DocStore::open_in_memory().unwrap();

        let lib_id = store
            .upsert_library("tokio", Ecosystem::Rust, "1.35.0", None)
            .unwrap();

        let chunks = vec![
            DocChunkInput {
                item_path: "tokio::spawn".to_string(),
                item_type: ItemType::Function,
                doc_text: "Spawns a new asynchronous task and returns a JoinHandle.".to_string(),
                signature: None,
                embedding: None,
            },
            DocChunkInput {
                item_path: "tokio::runtime::Runtime".to_string(),
                item_type: ItemType::Struct,
                doc_text: "The Tokio runtime for executing async code.".to_string(),
                signature: None,
                embedding: None,
            },
        ];

        store.insert_chunks_batch(lib_id, &chunks).unwrap();

        // Test FTS-only mode
        let results = store
            .search_hybrid("tokio", "asynchronous task", None, 10, SearchMode::Fts)
            .unwrap();

        assert!(!results.is_empty());
        assert!(results.iter().any(|r| r.chunk.item_path == "tokio::spawn"));
        // Scores should be positive (RRF-based)
        assert!(results.iter().all(|r| r.score > 0.0));
    }

    #[test]
    #[serial]
    fn test_search_hybrid_default_mode() {
        let store = DocStore::open_in_memory().unwrap();

        let lib_id = store
            .upsert_library("tokio", Ecosystem::Rust, "1.35.0", None)
            .unwrap();

        let chunks = vec![DocChunkInput {
            item_path: "tokio::spawn".to_string(),
            item_type: ItemType::Function,
            doc_text: "Spawns a new asynchronous task and returns a JoinHandle.".to_string(),
            signature: None,
            embedding: None,
        }];

        store.insert_chunks_batch(lib_id, &chunks).unwrap();

        // Test hybrid mode without embedding (should fall back to FTS)
        let results = store
            .search_hybrid("tokio", "asynchronous task", None, 10, SearchMode::Hybrid)
            .unwrap();

        assert!(!results.is_empty());
        assert!(results.iter().any(|r| r.chunk.item_path == "tokio::spawn"));
    }

    #[test]
    #[serial]
    fn test_search_convenience_method() {
        let store = DocStore::open_in_memory().unwrap();

        let lib_id = store
            .upsert_library("tokio", Ecosystem::Rust, "1.35.0", None)
            .unwrap();

        store
            .insert_chunk(
                lib_id,
                &DocChunkInput {
                    item_path: "tokio::spawn".to_string(),
                    item_type: ItemType::Function,
                    doc_text: "Spawns a new asynchronous task.".to_string(),
                    signature: None,
                    embedding: None,
                },
            )
            .unwrap();

        // Test convenience search method
        let results = store.search("tokio", "asynchronous", 10).unwrap();
        assert!(!results.is_empty());
    }

    #[test]
    #[serial]
    fn test_rrf_fusion_algorithm() {
        // Test the RRF fusion algorithm directly
        let chunk1 = DocChunk {
            id: 1,
            library_id: 1,
            item_path: "foo::bar".to_string(),
            item_type: ItemType::Function,
            doc_text: "A function".to_string(),
            signature: None,
        };
        let chunk2 = DocChunk {
            id: 2,
            library_id: 1,
            item_path: "foo::baz".to_string(),
            item_type: ItemType::Function,
            doc_text: "Another function".to_string(),
            signature: None,
        };
        let chunk3 = DocChunk {
            id: 3,
            library_id: 1,
            item_path: "foo::qux".to_string(),
            item_type: ItemType::Function,
            doc_text: "Third function".to_string(),
            signature: None,
        };

        // FTS ranks: chunk1 (rank 0), chunk2 (rank 1)
        let fts_results = vec![chunk1.clone(), chunk2.clone()];

        // Semantic ranks: chunk2 (rank 0), chunk3 (rank 1)
        let semantic_results = vec![chunk2.clone(), chunk3.clone()];

        let fused = rrf_fusion(&fts_results, &semantic_results, 10);

        // chunk2 should be first (appears in both lists)
        assert_eq!(fused[0].chunk.id, 2);
        assert_eq!(fused.len(), 3);

        // Verify chunk2 has higher score than others (appears in both lists)
        assert!(fused[0].score > fused[1].score);
    }

    #[test]
    #[serial]
    fn test_search_semantic_stub() {
        let store = DocStore::open_in_memory().unwrap();

        store
            .upsert_library("tokio", Ecosystem::Rust, "1.35.0", None)
            .unwrap();

        // Semantic search should return empty (stub implementation)
        let results = store
            .search_semantic("tokio", &[0.1, 0.2, 0.3], 10)
            .unwrap();

        assert!(results.is_empty());
    }
}
