use std::path::Path;

use anyhow::Result;
use http_client::HttpClient;
use sqlez::connection::Connection;

use crate::chunker;
use crate::embedding::{Embedder, EmbeddingProvider};
use crate::store;

pub fn get_project_files(project: &project::Project, cx: &gpui::App) -> Result<Vec<(String, String)>> {
    let worktrees = project.worktrees(cx);
    let mut files = Vec::new();

    for worktree in worktrees {
        let worktree = worktree.read(cx);
        let root_path = worktree.abs_path();

        let entries = walkdir::WalkDir::new(&root_path)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file());

        for entry in entries {
            let path = entry.path();
            let relative = path.strip_prefix(&root_path).unwrap_or(path);

            let path_str = relative.to_string_lossy();
            if should_skip_path(&path_str) {
                continue;
            }

            let language = chunker::language_from_extension(path);
            if language == "text" {
                continue;
            }

            files.push((relative.to_string_lossy().to_string(), path.to_string_lossy().to_string()));
        }
    }

    Ok(files)
}

fn should_skip_path(path: &str) -> bool {
    path.starts_with('.')
        || path.contains("/.")
        || path.contains("node_modules/")
        || path.contains("target/")
        || path.contains("__pycache__/")
        || path.contains(".git/")
        || path.contains("vendor/")
        || path.contains(".venv/")
        || path.contains("venv/")
        || path.contains(".cargo/")
        || path.ends_with(".pyc")
        || path.ends_with(".pyo")
        || path.ends_with(".class")
        || path.ends_with(".exe")
        || path.ends_with(".dll")
        || path.ends_with(".so")
        || path.ends_with(".dylib")
        || path.ends_with(".wasm")
        || path.ends_with(".png")
        || path.ends_with(".jpg")
        || path.ends_with(".jpeg")
        || path.ends_with(".gif")
        || path.ends_with(".svg")
        || path.ends_with(".ico")
        || path.ends_with(".woff")
        || path.ends_with(".woff2")
        || path.ends_with(".ttf")
        || path.ends_with(".eot")
        || path.ends_with(".pdf")
        || path.ends_with(".zip")
        || path.ends_with(".tar.gz")
        || path.ends_with(".bin")
}

pub async fn index_files(
    db: &Connection,
    files: Vec<(String, String)>,
    http_client: &dyn HttpClient,
) -> Result<usize> {
    let embedder = Embedder::new(EmbeddingProvider::default());
    let mut indexed_count = 0;

    for (relative_path, abs_path) in &files {
        let path = Path::new(abs_path);
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let file_hash = chunker::compute_file_hash(&content);

        let existing_hash = store::get_file_hash(db, relative_path).ok().flatten();
        if existing_hash.as_deref() == Some(&file_hash) {
            continue;
        }

        let _ = store::delete_chunks_for_file(db, relative_path);

        let chunks = chunker::chunk_file(path, &content);
        if chunks.is_empty() {
            continue;
        }

        let chunk_texts: Vec<&str> = chunks.iter().map(|c| c.text.as_str()).collect();
        let embeddings = match embedder.embed(&chunk_texts, http_client).await {
            Ok(embeddings) => embeddings.into_iter().map(Some).collect(),
            Err(e) => {
                log::error!("Embedding failed for {}: {}", relative_path, e);
                chunk_texts.iter().map(|_| None).collect::<Vec<Option<Vec<f32>>>>()
            }
        };

        let file_id = match store::upsert_file(db, relative_path, &file_hash, &chunks[0].language) {
            Ok(id) => id,
            Err(e) => {
                log::error!("Failed to upsert file {}: {}", relative_path, e);
                continue;
            }
        };

        for (chunk, embedding) in chunks.iter().zip(embeddings.iter()) {
            let _ = store::insert_chunk(
                db,
                file_id,
                relative_path,
                chunk.start_line,
                chunk.end_line,
                &chunk.text,
                embedding.as_deref(),
            );
        }

        indexed_count += 1;
    }

    Ok(indexed_count)
}
