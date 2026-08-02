use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, LazyLock};

use anyhow::Result;
use credentials_provider::CredentialsProvider;
use futures::FutureExt as _;
use gpui::{App, AsyncApp, Global};
use release_channel::ReleaseChannel;

/// An environment variable whose presence indicates that the system keychain
/// should be used in development.
///
/// By default, running Tau in development uses the development credentials
/// provider. Setting this environment variable allows you to interact with the
/// system keychain (for instance, if you need to test something).
///
/// Only works in development. Setting this environment variable in other
/// release channels is a no-op.
static TAU_DEVELOPMENT_USE_KEYCHAIN: LazyLock<bool> = LazyLock::new(|| {
    std::env::var("TAU_DEVELOPMENT_USE_KEYCHAIN").is_ok_and(|value| !value.is_empty())
});

pub struct ZedCredentialsProvider(pub Arc<dyn CredentialsProvider>);

impl Global for ZedCredentialsProvider {}

/// Returns the global [`CredentialsProvider`].
pub fn init_global(cx: &mut App) {
    // The `CredentialsProvider` trait has `Send + Sync` bounds on it, so it
    // seems like this is a false positive from Clippy.
    #[allow(clippy::arc_with_non_send_sync)]
    let provider = new(cx);
    cx.set_global(ZedCredentialsProvider(provider));
}

pub fn global(cx: &App) -> Arc<dyn CredentialsProvider> {
    cx.try_global::<ZedCredentialsProvider>()
        .map(|provider| provider.0.clone())
        .unwrap_or_else(|| new(cx))
}

fn new(cx: &App) -> Arc<dyn CredentialsProvider> {
    let use_development_provider = match ReleaseChannel::try_global(cx) {
        Some(ReleaseChannel::Dev) => {
            // In development we default to using the development
            // credentials provider to avoid getting spammed by relentless
            // keychain access prompts.
            //
            // However, if the `TAU_DEVELOPMENT_USE_KEYCHAIN` environment
            // variable is set, we will use the actual keychain.
            !*TAU_DEVELOPMENT_USE_KEYCHAIN
        }
        Some(ReleaseChannel::Nightly | ReleaseChannel::Preview | ReleaseChannel::Stable) | None => {
            false
        }
    };

    if use_development_provider {
        Arc::new(FileCredentialsProvider::new(
            paths::config_dir().join("development_credentials"),
        ))
    } else {
        #[cfg(target_os = "linux")]
        {
            // The system keychain on Linux relies on the D-Bus Secret Service,
            // which is not always running. Fall back to a file so that
            // credentials are not lost when it is unavailable.
            Arc::new(KeychainWithFileFallbackCredentialsProvider {
                keychain: Arc::new(KeychainCredentialsProvider),
                fallback: FileCredentialsProvider::new(
                    paths::config_dir().join("keychain_fallback_credentials"),
                ),
            })
        }
        #[cfg(not(target_os = "linux"))]
        {
            Arc::new(KeychainCredentialsProvider)
        }
    }
}

/// A credentials provider that stores credentials in the system keychain.
struct KeychainCredentialsProvider;

impl CredentialsProvider for KeychainCredentialsProvider {
    fn read_credentials<'a>(
        &'a self,
        url: &'a str,
        cx: &'a AsyncApp,
    ) -> Pin<Box<dyn Future<Output = Result<Option<(String, Vec<u8>)>>> + 'a>> {
        async move { cx.update(|cx| cx.read_credentials(url)).await }.boxed_local()
    }

    fn write_credentials<'a>(
        &'a self,
        url: &'a str,
        username: &'a str,
        password: &'a [u8],
        cx: &'a AsyncApp,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + 'a>> {
        async move {
            cx.update(move |cx| cx.write_credentials(url, username, password))
                .await
        }
        .boxed_local()
    }

    fn delete_credentials<'a>(
        &'a self,
        url: &'a str,
        cx: &'a AsyncApp,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + 'a>> {
        async move { cx.update(move |cx| cx.delete_credentials(url)).await }.boxed_local()
    }
}

/// A credentials provider that stores credentials in a local file.
///
/// This MUST only be used in development or as a fallback when the system
/// keychain is unavailable, as this is not a secure way of storing credentials
/// on user machines.
///
/// Its existence is purely to work around the annoyance of having to constantly
/// re-allow access to the system keychain when developing Tau, and to keep
/// credentials from being lost when the Linux Secret Service is not running.
struct FileCredentialsProvider {
    path: PathBuf,
}

impl FileCredentialsProvider {
    fn new(path: PathBuf) -> Self {
        Self { path }
    }

    fn load_credentials(&self) -> Result<HashMap<String, (String, Vec<u8>)>> {
        let json = std::fs::read(&self.path)?;
        let credentials: HashMap<String, (String, Vec<u8>)> = serde_json::from_slice(&json)?;

        Ok(credentials)
    }

    fn save_credentials(&self, credentials: &HashMap<String, (String, Vec<u8>)>) -> Result<()> {
        let json = serde_json::to_string(credentials)?;
        std::fs::write(&self.path, json)?;
        restrict_permissions(&self.path)?;

        Ok(())
    }
}

fn restrict_permissions(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }

    Ok(())
}

impl CredentialsProvider for FileCredentialsProvider {
    fn read_credentials<'a>(
        &'a self,
        url: &'a str,
        _cx: &'a AsyncApp,
    ) -> Pin<Box<dyn Future<Output = Result<Option<(String, Vec<u8>)>>> + 'a>> {
        async move {
            Ok(self
                .load_credentials()
                .unwrap_or_default()
                .get(url)
                .cloned())
        }
        .boxed_local()
    }

    fn write_credentials<'a>(
        &'a self,
        url: &'a str,
        username: &'a str,
        password: &'a [u8],
        _cx: &'a AsyncApp,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + 'a>> {
        async move {
            let mut credentials = self.load_credentials().unwrap_or_default();
            credentials.insert(url.to_string(), (username.to_string(), password.to_vec()));

            self.save_credentials(&credentials)
        }
        .boxed_local()
    }

    fn delete_credentials<'a>(
        &'a self,
        url: &'a str,
        _cx: &'a AsyncApp,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + 'a>> {
        async move {
            if !self.path.exists() {
                return Ok(());
            }
            let mut credentials = self.load_credentials()?;
            credentials.remove(url);

            self.save_credentials(&credentials)
        }
        .boxed_local()
    }
}

/// A credentials provider that falls back to storing credentials in a local
/// file when the system keychain is unavailable.
///
/// On Linux, the system keychain relies on the D-Bus Secret Service, which is
/// not always running. When it is unavailable, keychain operations fail and
/// credentials would otherwise be lost when the app restarts. This provider
/// transparently falls back to a file so that credentials still persist.
///
/// A fallback entry is removed whenever the keychain succeeds, so once the
/// keychain becomes available again it takes over and the file does not keep a
/// stale copy.
#[cfg(target_os = "linux")]
struct KeychainWithFileFallbackCredentialsProvider {
    keychain: Arc<dyn CredentialsProvider>,
    fallback: FileCredentialsProvider,
}

#[cfg(target_os = "linux")]
impl KeychainWithFileFallbackCredentialsProvider {
    async fn remove_fallback_entry(&self, url: &str, cx: &AsyncApp) {
        if let Err(err) = self.fallback.delete_credentials(url, cx).await {
            log::warn!("failed to remove stale fallback credential entry: {err:#}");
        }
    }
}

#[cfg(target_os = "linux")]
impl CredentialsProvider for KeychainWithFileFallbackCredentialsProvider {
    fn read_credentials<'a>(
        &'a self,
        url: &'a str,
        cx: &'a AsyncApp,
    ) -> Pin<Box<dyn Future<Output = Result<Option<(String, Vec<u8>)>>> + 'a>> {
        let keychain = self.keychain.clone();
        let fallback = &self.fallback;
        async move {
            match keychain.read_credentials(url, cx).await {
                Ok(Some(credentials)) => Ok(Some(credentials)),
                // A key may have been stored in the fallback file while the
                // keychain was unavailable, so check the file even when the
                // keychain reports no credentials.
                Ok(None) => Ok(fallback.read_credentials(url, cx).await?),
                Err(keychain_error) => {
                    log::warn!(
                        "system keychain unavailable ({keychain_error:#}), falling back to file credentials"
                    );
                    Ok(fallback.read_credentials(url, cx).await?)
                }
            }
        }
        .boxed_local()
    }

    fn write_credentials<'a>(
        &'a self,
        url: &'a str,
        username: &'a str,
        password: &'a [u8],
        cx: &'a AsyncApp,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + 'a>> {
        let keychain = self.keychain.clone();
        let fallback = &self.fallback;
        async move {
            match keychain.write_credentials(url, username, password, cx).await {
                Ok(()) => {
                    self.remove_fallback_entry(url, cx).await;
                    Ok(())
                }
                Err(keychain_error) => {
                    log::warn!(
                        "system keychain unavailable ({keychain_error:#}), storing credentials in file"
                    );
                    fallback.write_credentials(url, username, password, cx).await
                }
            }
        }
        .boxed_local()
    }

    fn delete_credentials<'a>(
        &'a self,
        url: &'a str,
        cx: &'a AsyncApp,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + 'a>> {
        let keychain = self.keychain.clone();
        let fallback = &self.fallback;
        async move {
            match keychain.delete_credentials(url, cx).await {
                Ok(()) => {
                    self.remove_fallback_entry(url, cx).await;
                    Ok(())
                }
                Err(keychain_error) => {
                    log::warn!(
                        "system keychain unavailable ({keychain_error:#}), deleting credential from file"
                    );
                    fallback.delete_credentials(url, cx).await
                }
            }
        }
        .boxed_local()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use gpui::{AsyncApp, TestAppContext};
    use tempfile::tempdir;

    use super::*;

    /// A keychain whose behavior can be scripted per test.
    #[derive(Clone)]
    enum ReadOutcome {
        None,
        Unavailable,
    }

    #[derive(Clone)]
    enum Outcome {
        Success,
        Unavailable,
    }

    struct ScriptedKeychain {
        read: Mutex<ReadOutcome>,
        write: Mutex<Outcome>,
        delete: Mutex<Outcome>,
    }

    impl ScriptedKeychain {
        fn new(
            read: ReadOutcome,
            write: Outcome,
            delete: Outcome,
        ) -> Self {
            Self {
                read: Mutex::new(read),
                write: Mutex::new(write),
                delete: Mutex::new(delete),
            }
        }

        fn unavailable() -> Self {
            Self::new(
                ReadOutcome::Unavailable,
                Outcome::Unavailable,
                Outcome::Unavailable,
            )
        }
    }

    impl CredentialsProvider for ScriptedKeychain {
        fn read_credentials<'a>(
            &'a self,
            _url: &'a str,
            _cx: &'a AsyncApp,
        ) -> Pin<Box<dyn Future<Output = Result<Option<(String, Vec<u8>)>>> + 'a>> {
            let outcome = self.read.lock().unwrap().clone();
            Box::pin(async move {
                match outcome {
                    ReadOutcome::None => Ok(None),
                    ReadOutcome::Unavailable => {
                        Err(anyhow::anyhow!("secret service unavailable"))
                    }
                }
            })
        }

        fn write_credentials<'a>(
            &'a self,
            _url: &'a str,
            _username: &'a str,
            _password: &'a [u8],
            _cx: &'a AsyncApp,
        ) -> Pin<Box<dyn Future<Output = Result<()>> + 'a>> {
            let outcome = self.write.lock().unwrap().clone();
            Box::pin(async move {
                match outcome {
                    Outcome::Success => Ok(()),
                    Outcome::Unavailable => {
                        Err(anyhow::anyhow!("secret service unavailable"))
                    }
                }
            })
        }

        fn delete_credentials<'a>(
            &'a self,
            _url: &'a str,
            _cx: &'a AsyncApp,
        ) -> Pin<Box<dyn Future<Output = Result<()>> + 'a>> {
            let outcome = self.delete.lock().unwrap().clone();
            Box::pin(async move {
                match outcome {
                    Outcome::Success => Ok(()),
                    Outcome::Unavailable => {
                        Err(anyhow::anyhow!("secret service unavailable"))
                    }
                }
            })
        }
    }

    #[gpui::test]
    async fn test_file_credentials_provider_round_trip(cx: &mut TestAppContext) {
        let cx = cx.to_async();
        let temp_dir = tempdir().unwrap();
        let path = temp_dir.path().join("credentials");

        let provider = FileCredentialsProvider::new(path.clone());

        provider
            .write_credentials("https://example.com", "Bearer", b"secret", &cx)
            .await
            .unwrap();
        let credentials = provider
            .read_credentials("https://example.com", &cx)
            .await
            .unwrap();
        assert_eq!(
            credentials,
            Some(("Bearer".to_string(), b"secret".to_vec()))
        );

        provider
            .delete_credentials("https://example.com", &cx)
            .await
            .unwrap();
        let credentials = provider
            .read_credentials("https://example.com", &cx)
            .await
            .unwrap();
        assert_eq!(credentials, None);
    }

    #[cfg(target_os = "linux")]
    #[gpui::test]
    async fn test_file_fallback_used_when_keychain_unavailable(cx: &mut TestAppContext) {
        let cx = cx.to_async();
        let temp_dir = tempdir().unwrap();
        let path = temp_dir.path().join("credentials");

        let provider = KeychainWithFileFallbackCredentialsProvider {
            keychain: Arc::new(ScriptedKeychain::unavailable()),
            fallback: FileCredentialsProvider::new(path),
        };

        provider
            .write_credentials("https://example.com", "Bearer", b"secret", &cx)
            .await
            .unwrap();
        let credentials = provider
            .read_credentials("https://example.com", &cx)
            .await
            .unwrap();
        assert_eq!(
            credentials,
            Some(("Bearer".to_string(), b"secret".to_vec()))
        );

        provider
            .delete_credentials("https://example.com", &cx)
            .await
            .unwrap();
        let credentials = provider
            .read_credentials("https://example.com", &cx)
            .await
            .unwrap();
        assert_eq!(credentials, None);
    }

    #[cfg(target_os = "linux")]
    #[gpui::test]
    async fn test_file_fallback_read_when_keychain_has_no_entry(cx: &mut TestAppContext) {
        let cx = cx.to_async();
        let temp_dir = tempdir().unwrap();
        let path = temp_dir.path().join("credentials");

        // A keychain that is available (read succeeds) but was down when the
        // credential was stored, so the entry only exists in the fallback file.
        let keychain = ScriptedKeychain::new(
            ReadOutcome::None,
            Outcome::Unavailable,
            Outcome::Unavailable,
        );
        let provider = KeychainWithFileFallbackCredentialsProvider {
            keychain: Arc::new(keychain),
            fallback: FileCredentialsProvider::new(path),
        };

        provider
            .write_credentials("https://example.com", "Bearer", b"secret", &cx)
            .await
            .unwrap();
        let credentials = provider
            .read_credentials("https://example.com", &cx)
            .await
            .unwrap();
        assert_eq!(
            credentials,
            Some(("Bearer".to_string(), b"secret".to_vec()))
        );
    }

    #[cfg(target_os = "linux")]
    #[gpui::test]
    async fn test_successful_keychain_write_removes_fallback_entry(cx: &mut TestAppContext) {
        let cx = cx.to_async();
        let temp_dir = tempdir().unwrap();
        let path = temp_dir.path().join("credentials");

        let provider = KeychainWithFileFallbackCredentialsProvider {
            keychain: Arc::new(ScriptedKeychain::new(
                ReadOutcome::None,
                Outcome::Success,
                Outcome::Success,
            )),
            fallback: FileCredentialsProvider::new(path.clone()),
        };

        // Seed a stale fallback entry from when the keychain was unavailable.
        provider
            .fallback
            .write_credentials("https://example.com", "Bearer", b"old", &cx)
            .await
            .unwrap();

        provider
            .write_credentials("https://example.com", "Bearer", b"new", &cx)
            .await
            .unwrap();
        let credentials = provider
            .read_credentials("https://example.com", &cx)
            .await
            .unwrap();
        assert_eq!(credentials, None);
    }
}
