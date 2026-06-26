use anyhow::Result;
use sqlez::connection::Connection;
use sqlez::domain::Migrator;

use crate::RagDomain;
use crate::chunker::Chunk;

fn serialize_embedding(embedding: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(embedding.len() * 4);
    for val in embedding {
        bytes.extend_from_slice(&val.to_le_bytes());
    }
    bytes
}

fn deserialize_embedding(bytes: &[u8]) -> Option<Vec<f32>> {
    if bytes.len() % 4 != 0 {
        return None;
    }
    let mut embedding = Vec::with_capacity(bytes.len() / 4);
    for chunk in bytes.chunks_exact(4) {
        let val = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        embedding.push(val);
    }
    Some(embedding)
}

pub fn ensure_schema(conn: &Connection) -> Result<()> {
    conn.exec("PRAGMA journal_mode=WAL")?()?;
    conn.exec("PRAGMA foreign_keys=ON")?()?;
    RagDomain::migrate(conn)
}

pub fn get_file_hash(conn: &Connection, file_path: &str) -> Result<Option<String>> {
    let mut select = conn.select_row_bound::<&str, String>(
        "SELECT file_hash FROM files WHERE file_path = ?",
    )?;
    select(file_path)
}

pub fn upsert_file(conn: &Connection, file_path: &str, file_hash: &str, language: &str) -> Result<i64> {
    let mut insert = conn.exec_bound::<(&str, &str, &str)>(
        "INSERT INTO files (file_path, file_hash, language) VALUES (?, ?, ?)
         ON CONFLICT(file_path) DO UPDATE SET file_hash = excluded.file_hash, language = excluded.language",
    )?;
    insert((file_path, file_hash, language))?;

    let file_id = conn.select_row::<i64>("SELECT last_insert_rowid()")?()?.unwrap_or(-1);
    Ok(file_id)
}

pub fn delete_chunks_for_file(conn: &Connection, file_path: &str) -> Result<()> {
    let mut delete = conn.exec_bound::<&str>("DELETE FROM chunks WHERE file_path = ?")?;
    delete(file_path)
}

pub fn insert_chunk(
    conn: &Connection,
    file_id: i64,
    file_path: &str,
    start_line: usize,
    end_line: usize,
    chunk_text: &str,
    embedding: Option<&[f32]>,
) -> Result<()> {
    let embedding_blob = embedding.map(serialize_embedding);

    let mut insert = conn.exec_bound::<(i64, &str, usize, usize, &str, Option<Vec<u8>>)>(
        "INSERT INTO chunks (file_id, file_path, start_line, end_line, chunk_text, embedding)
         VALUES (?, ?, ?, ?, ?, ?)",
    )?;
    insert((file_id, file_path, start_line, end_line, chunk_text, embedding_blob))
}

pub fn insert_chunks(conn: &Connection, chunks: &[Chunk], embeddings: &[Option<Vec<f32>>]) -> Result<()> {
    anyhow::ensure!(
        chunks.len() == embeddings.len(),
        "chunks and embeddings length mismatch"
    );

    for (chunk, embedding) in chunks.iter().zip(embeddings.iter()) {
        let file_hash = crate::chunker::compute_file_hash("");
        let file_id = upsert_file(conn, &chunk.file_path, &file_hash, &chunk.language)?;

        let embedding_blob = embedding.as_ref().map(|e| serialize_embedding(e));

        let mut insert = conn.exec_bound::<(i64, &str, usize, usize, &str, Option<Vec<u8>>)>(
            "INSERT INTO chunks (file_id, file_path, start_line, end_line, chunk_text, embedding)
             VALUES (?, ?, ?, ?, ?, ?)",
        )?;
        insert((file_id, &chunk.file_path, chunk.start_line, chunk.end_line, &chunk.text, embedding_blob))?;
    }

    Ok(())
}

pub struct StoredChunk {
    pub id: i64,
    pub file_path: String,
    pub start_line: usize,
    pub end_line: usize,
    pub chunk_text: String,
    pub embedding: Option<Vec<f32>>,
}

pub fn load_all_chunks(conn: &Connection) -> Result<Vec<StoredChunk>> {
    let chunks = conn.select::<(i64, String, usize, usize, String, Option<Vec<u8>>)>(
        "SELECT id, file_path, start_line, end_line, chunk_text, embedding FROM chunks WHERE embedding IS NOT NULL",
    )?()?;

    Ok(chunks
        .into_iter()
        .map(|(id, file_path, start_line, end_line, chunk_text, embedding_blob)| StoredChunk {
            id,
            file_path,
            start_line,
            end_line,
            chunk_text,
            embedding: embedding_blob.and_then(|b| deserialize_embedding(&b)),
        })
        .collect())
}

pub fn chunk_count(conn: &Connection) -> Result<usize> {
    let count = conn.select_row::<i64>("SELECT COUNT(*) FROM chunks")?()?;
    Ok(count.unwrap_or(0) as usize)
}

pub fn file_count(conn: &Connection) -> Result<usize> {
    let count = conn.select_row::<i64>("SELECT COUNT(*) FROM files")?()?;
    Ok(count.unwrap_or(0) as usize)
}
