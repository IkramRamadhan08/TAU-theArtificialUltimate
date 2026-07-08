use anyhow::{Result, anyhow};
use fs::Fs;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use text::Rope;

#[derive(Serialize, Deserialize)]
struct CredentialEntry {
    values: HashMap<String, String>,
}

/// Persistent credential store backed by an obfuscated JSON file in the TAU
/// config directory.
///
/// Credentials are grouped by service name (e.g. "google_oauth"). Each service
/// stores key-value pairs for its fields.
pub struct CredentialStore {
    fs: Arc<dyn Fs>,
    store_dir: PathBuf,
}

impl CredentialStore {
    pub fn new(fs: Arc<dyn Fs>, config_dir: PathBuf) -> Self {
        let store_dir = config_dir.join("credentials");
        Self { fs, store_dir }
    }

    pub fn store_path(&self, service: &str) -> PathBuf {
        self.store_dir.join(format!("{}.json.enc", service))
    }

    /// Retrieve stored credentials for a service.
    pub async fn get(&self, service: &str) -> Result<Option<HashMap<String, String>>> {
        let path = self.store_path(service);
        if !self.fs.is_file(&path).await {
            return Ok(None);
        }
        let data = self.fs.load(&path).await?;
        let decoded = Self::decode(&data)?;
        let entry: CredentialEntry = serde_json::from_str(&decoded)?;
        Ok(Some(entry.values))
    }

    /// Store credentials for a service.
    pub async fn set(&self, service: &str, values: HashMap<String, String>) -> Result<()> {
        if !self.fs.is_dir(&self.store_dir).await {
            self.fs.create_dir(&self.store_dir).await?;
        }
        let entry = CredentialEntry { values };
        let data = serde_json::to_string(&entry)?;
        let encoded = Self::encode(&data);
        let path = self.store_path(service);
        self.fs.save(&path, &Rope::from(encoded.as_str()), Default::default()).await?;
        Ok(())
    }

    /// Remove stored credentials for a service.
    pub async fn remove(&self, service: &str) -> Result<()> {
        let path = self.store_path(service);
        if self.fs.is_file(&path).await {
            self.fs.remove_file(&path, Default::default()).await?;
        }
        Ok(())
    }

    /// Simple obfuscation to avoid storing credentials in plaintext.
    fn encode(data: &str) -> String {
        use base64::Engine as _;
        let bytes = data.as_bytes();
        let key = b"tau-credential-store-v1";
        let obfuscated: Vec<u8> = bytes
            .iter()
            .enumerate()
            .map(|(i, b)| b ^ key[i % key.len()])
            .collect();
        base64::engine::general_purpose::STANDARD.encode(&obfuscated)
    }

    fn decode(data: &str) -> Result<String> {
        use base64::Engine as _;
        let obfuscated = base64::engine::general_purpose::STANDARD
            .decode(data)
            .map_err(|e| anyhow!("Failed to decode credential: {}", e))?;
        let key = b"tau-credential-store-v1";
        let bytes: Vec<u8> = obfuscated
            .iter()
            .enumerate()
            .map(|(i, b)| b ^ key[i % key.len()])
            .collect();
        Ok(String::from_utf8(bytes).map_err(|e| anyhow!("Invalid UTF-8 in credential: {}", e))?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;
    use project::FakeFs;
    use std::sync::Arc;
    use util::path;

    #[gpui::test]
    async fn test_store_and_retrieve(cx: &mut TestAppContext) {
        let fs = Arc::new(FakeFs::new(cx.executor()));
        let dir = PathBuf::from("/config");
        fs.insert_tree(path!("/config"), Default::default()).await;
        let store = CredentialStore::new(fs, dir);

        let mut values = HashMap::new();
        values.insert("CLIENT_ID".to_string(), "abc123".to_string());
        values.insert("CLIENT_SECRET".to_string(), "secret456".to_string());

        store.set("google_oauth", values.clone()).await.unwrap();

        let retrieved = store.get("google_oauth").await.unwrap().unwrap();
        assert_eq!(retrieved.get("CLIENT_ID").unwrap(), "abc123");
        assert_eq!(retrieved.get("CLIENT_SECRET").unwrap(), "secret456");
    }
}
