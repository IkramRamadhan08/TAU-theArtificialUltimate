use std::fmt;

use anyhow::{Context, Result};
use futures::io::AsyncReadExt;
use http_client::{AsyncBody, HttpClient, Method, Request as HttpRequest};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub enum EmbeddingProvider {
    Ollama {
        base_url: String,
        model: String,
    },
    OpenAI {
        base_url: String,
        api_key: String,
        model: String,
    },
}

impl Default for EmbeddingProvider {
    fn default() -> Self {
        Self::Ollama {
            base_url: "http://localhost:11434".into(),
            model: "nomic-embed-text".into(),
        }
    }
}

impl fmt::Display for EmbeddingProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ollama { model, .. } => write!(f, "Ollama({})", model),
            Self::OpenAI { model, .. } => write!(f, "OpenAI({})", model),
        }
    }
}

pub struct Embedder {
    provider: EmbeddingProvider,
}

impl Embedder {
    pub fn new(provider: EmbeddingProvider) -> Self {
        Self { provider }
    }

    pub async fn embed(&self, texts: &[&str], http_client: &dyn HttpClient) -> Result<Vec<Vec<f32>>> {
        match &self.provider {
            EmbeddingProvider::Ollama { base_url, model } => {
                embed_ollama(http_client, base_url, model, texts).await
            }
            EmbeddingProvider::OpenAI {
                base_url,
                api_key,
                model,
            } => embed_openai(http_client, base_url, api_key, model, texts).await,
        }
    }

    pub fn embedding_dimension(&self) -> usize {
        match &self.provider {
            EmbeddingProvider::Ollama { model, .. } => match model.as_str() {
                "nomic-embed-text" => 768,
                "all-minilm" => 384,
                "mxbai-embed-large" => 1024,
                "snowflake-arctic-embed" => 1024,
                "bge-m3" => 1024,
                _ => 768,
            },
            EmbeddingProvider::OpenAI { model, .. } => match model.as_str() {
                "text-embedding-3-small" => 1536,
                "text-embedding-3-large" => 3072,
                "text-embedding-ada-002" => 1536,
                _ => 1536,
            },
        }
    }
}

#[derive(Serialize)]
struct OllamaEmbedRequest<'a> {
    model: &'a str,
    input: Vec<&'a str>,
}

#[derive(Deserialize)]
struct OllamaEmbedResponse {
    embeddings: Vec<Vec<f32>>,
}

async fn embed_ollama(
    client: &dyn HttpClient,
    base_url: &str,
    model: &str,
    texts: &[&str],
) -> Result<Vec<Vec<f32>>> {
    let uri = format!("{}/api/embed", base_url.trim_end_matches('/'));
    let request = OllamaEmbedRequest {
        model,
        input: texts.to_vec(),
    };
    let body = AsyncBody::from(serde_json::to_string(&request)?);

    let request = HttpRequest::builder()
        .method(Method::POST)
        .uri(&uri)
        .header("Content-Type", "application/json")
        .body(body)?;

    let mut response = client.send(request).await?;
    let status = response.status();
    let mut response_body = String::new();
    response.body_mut().read_to_string(&mut response_body).await?;

    anyhow::ensure!(
        status.is_success(),
        "Ollama embedding failed: status={}, body={}",
        status,
        response_body,
    );

    let parsed: OllamaEmbedResponse =
        serde_json::from_str(&response_body).context("failed to parse Ollama embedding response")?;
    Ok(parsed.embeddings)
}

#[derive(Serialize)]
struct OpenAIEmbedRequest<'a> {
    model: &'a str,
    input: Vec<&'a str>,
}

#[derive(Deserialize)]
struct OpenAIEmbedResponse {
    data: Vec<OpenAIEmbedding>,
}

#[derive(Deserialize)]
struct OpenAIEmbedding {
    embedding: Vec<f32>,
}

async fn embed_openai(
    client: &dyn HttpClient,
    api_url: &str,
    api_key: &str,
    model: &str,
    texts: &[&str],
) -> Result<Vec<Vec<f32>>> {
    let uri = format!("{}/embeddings", api_url.trim_end_matches('/'));
    let request = OpenAIEmbedRequest {
        model,
        input: texts.to_vec(),
    };
    let body = AsyncBody::from(serde_json::to_string(&request)?);

    let request = HttpRequest::builder()
        .method(Method::POST)
        .uri(&uri)
        .header("Content-Type", "application/json")
        .header("Authorization", format!("Bearer {}", api_key.trim()))
        .body(body)?;

    let mut response = client.send(request).await?;
    let status = response.status();
    let mut response_body = String::new();
    response.body_mut().read_to_string(&mut response_body).await?;

    anyhow::ensure!(
        status.is_success(),
        "OpenAI embedding failed: status={}, body={}",
        status,
        response_body,
    );

    let parsed: OpenAIEmbedResponse =
        serde_json::from_str(&response_body).context("failed to parse OpenAI embedding response")?;
    Ok(parsed.data.into_iter().map(|d| d.embedding).collect())
}

pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }
    dot / (norm_a * norm_b)
}
