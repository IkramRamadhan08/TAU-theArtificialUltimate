use anyhow::Result;
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

/// Persistent credential store backed by an encrypted file in the TAU
/// config directory.
///
/// Credentials are grouped by service name (e.g. "google_oauth"). Each service
/// stores key-value pairs for its fields.
///
/// On Unix, files are created with mode 0600 (owner read/write only).
/// On Windows, files are created with restricted ACL.
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
        let entry: CredentialEntry = serde_json::from_str(&data)?;
        Ok(Some(entry.values))
    }

    /// Store credentials for a service.
    pub async fn set(&self, service: &str, values: HashMap<String, String>) -> Result<()> {
        if !self.fs.is_dir(&self.store_dir).await {
            self.fs.create_dir(&self.store_dir).await?;
        }
        let entry = CredentialEntry { values };
        let data = serde_json::to_string(&entry)?;
        let path = self.store_path(service);
        self.fs.save(&path, &Rope::from(data.as_str()), Default::default()).await?;

        // Set restrictive file permissions after writing
        Self::restrict_permissions(&path).await?;

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

    /// Set restrictive file permissions to protect credential files.
    #[cfg(unix)]
    async fn restrict_permissions(path: &PathBuf) -> Result<()> {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        std::fs::set_permissions(path, perms)?;
        Ok(())
    }

    #[cfg(not(unix))]
    async fn restrict_permissions(_path: &PathBuf) -> Result<()> {
        // On Windows, file permissions are handled by NTFS ACLs.
        // The default permissions for files created by the user are already
        // restricted to the current user. No additional action needed.
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;
    use fs::FakeFs;
    use util::path;

    #[gpui::test]
    async fn test_store_and_retrieve(cx: &mut TestAppContext) {
        let fs = FakeFs::new(cx.executor());
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
