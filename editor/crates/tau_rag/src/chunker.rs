use std::path::Path;
use std::sync::Arc;

/// A single code chunk extracted from a file.
#[derive(Clone, Debug)]
pub struct Chunk {
    pub file_path: Arc<str>,
    pub start_line: usize,
    pub end_line: usize,
    pub text: String,
    pub language: String,
}

/// Detect language from file extension
pub fn language_from_extension(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("rs") => "rust",
        Some("py") => "python",
        Some("js") | Some("ts") | Some("jsx") | Some("tsx") | Some("mjs") => "typescript",
        Some("go") => "go",
        Some("java") => "java",
        Some("c") | Some("h") => "c",
        Some("cpp") | Some("hpp") | Some("cc") | Some("cxx") => "cpp",
        Some("rb") => "ruby",
        Some("php") => "php",
        Some("swift") => "swift",
        Some("kt") | Some("kts") => "kotlin",
        Some("cs") => "csharp",
        Some("sh") | Some("bash") | Some("zsh") => "bash",
        Some("sql") => "sql",
        Some("html") | Some("htm") => "html",
        Some("css") | Some("scss") | Some("less") => "css",
        Some("json") => "json",
        Some("yaml") | Some("yml") => "yaml",
        Some("toml") => "toml",
        Some("md") | Some("mdx") => "markdown",
        Some("rsx") => "rust",
        Some("vue") => "typescript",
        Some("r") | Some("R") => "r",
        Some("dart") => "dart",
        Some("lua") => "lua",
        Some("ex") | Some("exs") => "elixir",
        Some("hs") => "haskell",
        Some("scala") | Some("sc") => "scala",
        _ => "text",
    }
}

const DEFAULT_CHUNK_SIZE: usize = 100;
const MIN_CHUNK_SIZE: usize = 20;
const MAX_CHUNK_SIZE: usize = 300;

/// Simple line-based chunking with smart boundaries.
/// Prefers splitting at blank lines or section boundaries.
pub fn chunk_code(content: &str, language: &str) -> Vec<(usize, usize, String)> {
    let lines: Vec<&str> = content.lines().collect();
    let total_lines = lines.len();
    if total_lines == 0 {
        return vec![];
    }

    if total_lines <= MAX_CHUNK_SIZE {
        return vec![(
            0,
            total_lines,
            content.to_string(),
        )];
    }

    let mut chunks = Vec::new();
    let mut start = 0;

    while start < total_lines {
        let end = if start + MAX_CHUNK_SIZE >= total_lines {
            total_lines
        } else {
            // Try to find a good split point
            let search_end = (start + MAX_CHUNK_SIZE).min(total_lines);
            let search_start = start + MIN_CHUNK_SIZE;

            // Look for blank lines near the chunk boundary (prefer splitting at blank lines)
            let ideal_end = start + DEFAULT_CHUNK_SIZE;

            let mut best_split = ideal_end;
            // Look backwards from ideal_end for blank lines
            let mut found_split = false;
            for i in (search_start..search_end).rev() {
                if lines[i].trim().is_empty() {
                    best_split = i + 1; // Split AFTER the blank line
                    found_split = true;
                    break;
                }
            }

            if !found_split {
                // Look for function/class definitions backwards
                let def_keywords = match language {
                    "rust" => &["fn ", "pub fn", "struct ", "enum ", "impl ", "trait ", "mod ", "pub struct", "pub enum", "pub trait", "pub mod", "unsafe fn", "pub unsafe fn"][..],
                    "python" => &["def ", "class ", "async def ", "@"][..],
                    "typescript" | "javascript" => &["function ", "class ", "const ", "async function", "export function", "export class", "export const", "interface ", "type "][..],
                    "go" => &["func ", "type ", "struct ", "interface "][..],
                    "java" => &["class ", "interface ", "enum ", "public class", "public interface"][..],
                    "cpp" | "c" => &["class ", "struct ", "enum ", "void ", "int ", "bool ", "auto "][..],
                    _ => &["fn ", "def ", "class ", "function "][..],
                };

                'outer: for i in (search_start..search_end).rev() {
                    let line = lines[i].trim();
                    for kw in def_keywords {
                        if line.starts_with(kw) {
                            best_split = i;
                            found_split = true;
                            break 'outer;
                        }
                    }
                }
            }

            if !found_split {
                best_split = ideal_end;
            }

            best_split.min(total_lines)
        };

        if end <= start {
            break;
        }

        let chunk_text = lines[start..end].join("\n");
        chunks.push((start, end, chunk_text));
        start = end;
    }

    chunks
}

pub fn chunk_file(
    file_path: &Path,
    content: &str,
) -> Vec<Chunk> {
    let language = language_from_extension(file_path);
    let chunks = chunk_code(content, language);

    let file_path: Arc<str> = file_path.to_string_lossy().into();
    chunks
        .into_iter()
        .map(|(start, end, text)| Chunk {
            file_path: file_path.clone(),
            start_line: start,
            end_line: end,
            text,
            language: language.to_string(),
        })
        .collect()
}

pub fn compute_file_hash(content: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    content.hash(&mut hasher);
    format!("{:x}", hasher.finish())
}
