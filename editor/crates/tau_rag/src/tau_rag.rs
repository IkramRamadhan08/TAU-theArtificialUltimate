pub mod chunker;
pub mod embedding;
pub mod indexer;
pub mod search;
pub mod store;

use http_client::HttpClient;
use paths::embeddings_dir;
use sqlez::connection::Connection;
use sqlez::domain::Domain;

const RAG_DOMAIN: &str = "rag_store";
const RAG_MIGRATIONS: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS files (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        file_path TEXT NOT NULL UNIQUE,
        file_hash TEXT NOT NULL,
        language TEXT,
        indexed_at TEXT NOT NULL DEFAULT (datetime('now'))
    ) STRICT",
    "CREATE TABLE IF NOT EXISTS chunks (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        file_id INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
        file_path TEXT NOT NULL,
        start_line INTEGER NOT NULL,
        end_line INTEGER NOT NULL,
        chunk_text TEXT NOT NULL,
        embedding BLOB
    ) STRICT",
    "CREATE INDEX IF NOT EXISTS idx_chunks_file_id ON chunks(file_id)",
    "CREATE INDEX IF NOT EXISTS idx_chunks_file_path ON chunks(file_path)",
    "CREATE VIRTUAL TABLE IF NOT EXISTS chunks_fts USING fts5(
        chunk_text,
        content='chunks',
        content_rowid='id',
        tokenize='unicode61'
    )",
    "CREATE TRIGGER IF NOT EXISTS chunks_ai AFTER INSERT ON chunks BEGIN
        INSERT INTO chunks_fts(rowid, chunk_text) VALUES (new.id, new.chunk_text);
    END",
    "CREATE TRIGGER IF NOT EXISTS chunks_ad AFTER DELETE ON chunks BEGIN
        INSERT INTO chunks_fts(chunks_fts, rowid, chunk_text) VALUES('delete', old.id, old.chunk_text);
    END",
    "CREATE TRIGGER IF NOT EXISTS chunks_au AFTER UPDATE ON chunks BEGIN
        INSERT INTO chunks_fts(chunks_fts, rowid, chunk_text) VALUES('delete', old.id, old.chunk_text);
        INSERT INTO chunks_fts(rowid, chunk_text) VALUES (new.id, new.chunk_text);
    END",
];

pub struct RagDomain;

impl Domain for RagDomain {
    const NAME: &str = RAG_DOMAIN;
    const MIGRATIONS: &[&str] = RAG_MIGRATIONS;
}

pub fn open_or_create_db() -> anyhow::Result<Connection> {
    let db_dir = embeddings_dir();
    std::fs::create_dir_all(db_dir)?;
    let db_path = db_dir.join("rag.db");
    let connection = Connection::open_file(&db_path.to_string_lossy());
    store::ensure_schema(&connection)?;
    Ok(connection)
}

pub fn search(
    query: &str,
    limit: usize,
    file_filter: Option<&str>,
) -> anyhow::Result<Vec<search::SearchResult>> {
    let db = open_or_create_db()?;
    search::search(&db, query, limit, file_filter)
}

pub async fn ensure_indexed(
    files: Vec<(String, String)>,
    http_client: &dyn HttpClient,
) -> anyhow::Result<usize> {
    let db = open_or_create_db()?;
    indexer::index_files(&db, files, http_client).await
}

pub fn get_index_stats() -> anyhow::Result<(usize, usize)> {
    let db = open_or_create_db()?;
    let chunks = store::chunk_count(&db)?;
    let files = store::file_count(&db)?;
    Ok((files, chunks))
}
