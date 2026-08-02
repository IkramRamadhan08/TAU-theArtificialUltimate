use anyhow::{Context as _, Result, anyhow};
use credentials_provider::CredentialsProvider;
use env_var::EnvVar;
use futures::{FutureExt, future};
use gpui::{AsyncApp, Context, SharedString, Task};
use std::{
    fmt::{Display, Formatter},
    sync::Arc,
};
use crate::AuthenticateError;
/// Manages a single API key for a language model provider. API keys either come from environment
/// variables or the system keychain.
///
/// Keys from the system keychain are associated with a provider URL, and this ensures that they are
/// only used with that URL.
pub struct ApiKeyState {
    pub url: SharedString,
    env_var: EnvVar,
    load_status: LoadStatus,
    load_task: Option<future::Shared<Task<()>>>,
    last_store_error: Option<SharedString>,
}

#[derive(Debug, Clone)]
pub enum LoadStatus {
    NotPresent,
    Error(String),
    Loaded(ApiKey),
}

#[derive(Debug, Clone)]
pub struct ApiKey {
    source: ApiKeySource,
    key: Arc<str>,
}

impl ApiKeyState {
    pub fn new(url: SharedString, env_var: EnvVar) -> Self {
        Self {
            url,
            env_var,
            load_status: LoadStatus::NotPresent,
            load_task: None,
            last_store_error: None,
        }
    }

    pub fn last_store_error(&self) -> Option<SharedString> {
        self.last_store_error.clone()
    }

    pub fn has_key(&self) -> bool {
        matches!(self.load_status, LoadStatus::Loaded { .. })
    }

    pub fn env_var_name(&self) -> &SharedString {
        &self.env_var.name
    }

    pub fn is_from_env_var(&self) -> bool {
        match &self.load_status {
            LoadStatus::Loaded(ApiKey {
                source: ApiKeySource::EnvVar { .. },
                ..
            }) => true,
            _ => false,
        }
    }

    /// Get the stored API key, verifying that it is associated with the URL. Returns `None` if
    /// there is no key or for URL mismatches, and the mismatch case is logged.
    ///
    /// To avoid URL mismatches, expects that `load_if_needed` or `handle_url_change` has been
    /// called with this URL.
    pub fn key(&self, url: &str) -> Option<Arc<str>> {
        let api_key = match &self.load_status {
            LoadStatus::Loaded(api_key) => api_key,
            _ => return None,
        };
        if url == self.url.as_str() {
            Some(api_key.key.clone())
        } else if let ApiKeySource::EnvVar(var_name) = &api_key.source {
            log::warn!(
                "{} is now being used with URL {}, when initially it was used with URL {}",
                var_name,
                url,
                self.url
            );
            Some(api_key.key.clone())
        } else {
            // bug case because load_if_needed should be called whenever the url may have changed
            log::error!(
                "bug: Attempted to use API key associated with URL {} instead with URL {}",
                self.url,
                url
            );
            None
        }
    }

    /// Set or delete the API key in the system keychain.
    pub fn store<Ent: 'static>(
        &mut self,
        url: SharedString,
        key: Option<String>,
        get_this: impl Fn(&mut Ent) -> &mut Self + 'static,
        provider: Arc<dyn CredentialsProvider>,
        cx: &Context<Ent>,
    ) -> Task<Result<()>> {
        if self.is_from_env_var() {
            return Task::ready(Err(anyhow!(
                "bug: attempted to store API key in system keychain when API key is from env var",
            )));
        }
        cx.spawn(async move |ent, cx| {
            let store_result = match &key {
                Some(key) => provider
                    .write_credentials(&url, "Bearer", key.as_bytes(), cx)
                    .await
                    .context("failed to store API key in system keychain"),
                None => provider
                    .delete_credentials(&url, cx)
                    .await
                    .context("failed to delete API key from system keychain"),
            };

            ent.update(cx, |ent, cx| {
                let this = get_this(ent);
                this.url = url;
                match store_result.as_ref() {
                    Ok(()) => {
                        this.last_store_error = None;
                        this.load_status = match &key {
                            Some(key) => LoadStatus::Loaded(ApiKey {
                                source: ApiKeySource::SystemKeychain,
                                key: key.as_str().into(),
                            }),
                            None => LoadStatus::NotPresent,
                        };
                    }
                    Err(err) => {
                        this.last_store_error = Some(err.to_string().into());
                    }
                }
                cx.notify();
            })?;

            store_result
        })
    }

    /// Reloads the API key if the current API key is associated with a different URL.
    ///
    /// Note that it is not efficient to use this or `load_if_needed` with multiple URLs
    /// interchangeably - URL change should correspond to some user initiated change.
    pub fn handle_url_change<Ent: 'static>(
        &mut self,
        url: SharedString,
        get_this: impl Fn(&mut Ent) -> &mut Self + Clone + 'static,
        provider: Arc<dyn CredentialsProvider>,
        cx: &mut Context<Ent>,
    ) {
        if url != self.url {
            if !self.is_from_env_var() {
                // loading will continue even though this result task is dropped
                let _task = self.load_if_needed(url, get_this, provider, cx);
            }
        }
    }

    /// If needed, loads the API key associated with the given URL from the system keychain. When a
    /// non-empty environment variable is provided, it will be used instead. If called when an API
    /// key was already loaded for a different URL, that key will be cleared before loading.
    ///
    /// Dropping the returned Task does not cancel key loading.
    pub fn load_if_needed<Ent: 'static>(
        &mut self,
        url: SharedString,
        get_this: impl Fn(&mut Ent) -> &mut Self + Clone + 'static,
        provider: Arc<dyn CredentialsProvider>,
        cx: &mut Context<Ent>,
    ) -> Task<Result<(), AuthenticateError>> {
        if let LoadStatus::Loaded { .. } = &self.load_status
            && self.url == url
        {
            return Task::ready(Ok(()));
        }

        if let Some(key) = &self.env_var.value
            && !key.is_empty()
        {
            let api_key = ApiKey::from_env(self.env_var.name.clone(), key);
            self.url = url;
            self.load_status = LoadStatus::Loaded(api_key);
            self.load_task = None;
            cx.notify();
            return Task::ready(Ok(()));
        }

        let task = if let Some(load_task) = &self.load_task {
            load_task.clone()
        } else {
            let load_task = Self::load(url.clone(), get_this.clone(), provider, cx).shared();
            self.url = url;
            self.load_status = LoadStatus::NotPresent;
            self.load_task = Some(load_task.clone());
            cx.notify();
            load_task
        };

        cx.spawn(async move |ent, cx| {
            task.await;
            ent.update(cx, |ent, _cx| {
                get_this(ent).load_status.clone().into_authenticate_result()
            })
            .ok();
            Ok(())
        })
    }

    fn load<Ent: 'static>(
        url: SharedString,
        get_this: impl Fn(&mut Ent) -> &mut Self + 'static,
        provider: Arc<dyn CredentialsProvider>,
        cx: &Context<Ent>,
    ) -> Task<()> {
        cx.spawn({
            async move |ent, cx| {
                let load_status =
                    ApiKey::load_from_system_keychain_impl(&url, provider.as_ref(), cx).await;
                ent.update(cx, |ent, cx| {
                    let this = get_this(ent);
                    this.url = url;
                    this.load_status = load_status;
                    this.load_task = None;
                    cx.notify();
                })
                .ok();
            }
        })
    }
}

impl ApiKey {
    pub fn key(&self) -> &str {
        &self.key
    }

    pub fn from_env(env_var_name: SharedString, key: &str) -> Self {
        Self {
            source: ApiKeySource::EnvVar(env_var_name),
            key: key.into(),
        }
    }

    pub async fn load_from_system_keychain(
        url: &str,
        credentials_provider: &dyn CredentialsProvider,
        cx: &AsyncApp,
    ) -> Result<Self, AuthenticateError> {
        Self::load_from_system_keychain_impl(url, credentials_provider, cx)
            .await
            .into_authenticate_result()
    }

    async fn load_from_system_keychain_impl(
        url: &str,
        credentials_provider: &dyn CredentialsProvider,
        cx: &AsyncApp,
    ) -> LoadStatus {
        if url.is_empty() {
            return LoadStatus::NotPresent;
        }
        let read_result = credentials_provider.read_credentials(&url, cx).await;
        let api_key = match read_result {
            Ok(Some((_, api_key))) => api_key,
            Ok(None) => return LoadStatus::NotPresent,
            Err(err) => return LoadStatus::Error(err.to_string()),
        };
        let key = match str::from_utf8(&api_key) {
            Ok(key) => key,
            Err(_) => return LoadStatus::Error(format!("API key for URL {url} is not utf8")),
        };
        LoadStatus::Loaded(Self {
            source: ApiKeySource::SystemKeychain,
            key: key.into(),
        })
    }
}

impl LoadStatus {
    fn into_authenticate_result(self) -> Result<ApiKey, AuthenticateError> {
        match self {
            LoadStatus::Loaded(api_key) => Ok(api_key),
            LoadStatus::NotPresent => Err(AuthenticateError::CredentialsNotFound),
            LoadStatus::Error(err) => Err(AuthenticateError::Other(anyhow!(err))),
        }
    }
}

#[derive(Debug, Clone)]
enum ApiKeySource {
    EnvVar(SharedString),
    SystemKeychain,
}

impl Display for ApiKeySource {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            ApiKeySource::EnvVar(var) => write!(f, "environment variable {}", var),
            ApiKeySource::SystemKeychain => write!(f, "system keychain"),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::Arc;

    use anyhow::Result;
    use credentials_provider::CredentialsProvider;
    use env_var::EnvVar;
    use gpui::{App, AppContext as _, AsyncApp, SharedString, TestAppContext};

    use crate::api_key::ApiKeyState;

    struct FailingWriteProvider;

    impl CredentialsProvider for FailingWriteProvider {
        fn read_credentials<'a>(
            &'a self,
            _url: &'a str,
            _cx: &'a AsyncApp,
        ) -> Pin<Box<dyn Future<Output = Result<Option<(String, Vec<u8>)>>> + 'a>> {
            Box::pin(async { Ok(None) })
        }

        fn write_credentials<'a>(
            &'a self,
            _url: &'a str,
            _username: &'a str,
            _password: &'a [u8],
            _cx: &'a AsyncApp,
        ) -> Pin<Box<dyn Future<Output = Result<()>> + 'a>> {
            Box::pin(async { Err(anyhow::anyhow!("keychain unavailable")) })
        }

        fn delete_credentials<'a>(
            &'a self,
            _url: &'a str,
            _cx: &'a AsyncApp,
        ) -> Pin<Box<dyn Future<Output = Result<()>> + 'a>> {
            Box::pin(async { Ok(()) })
        }
    }

    struct SucceedingWriteProvider;

    impl CredentialsProvider for SucceedingWriteProvider {
        fn read_credentials<'a>(
            &'a self,
            _url: &'a str,
            _cx: &'a AsyncApp,
        ) -> Pin<Box<dyn Future<Output = Result<Option<(String, Vec<u8>)>>> + 'a>> {
            Box::pin(async { Ok(None) })
        }

        fn write_credentials<'a>(
            &'a self,
            _url: &'a str,
            _username: &'a str,
            _password: &'a [u8],
            _cx: &'a AsyncApp,
        ) -> Pin<Box<dyn Future<Output = Result<()>> + 'a>> {
            Box::pin(async { Ok(()) })
        }

        fn delete_credentials<'a>(
            &'a self,
            _url: &'a str,
            _cx: &'a AsyncApp,
        ) -> Pin<Box<dyn Future<Output = Result<()>> + 'a>> {
            Box::pin(async { Ok(()) })
        }
    }

    struct ApiKeyHolder {
        api_key_state: ApiKeyState,
    }

    fn new_holder(cx: &mut App) -> gpui::Entity<ApiKeyHolder> {
        cx.new(|_cx| ApiKeyHolder {
            api_key_state: ApiKeyState::new(
                SharedString::new("https://example.com"),
                EnvVar::new(SharedString::new("TEST_API_KEY")),
            ),
        })
    }

    #[gpui::test]
    async fn test_store_reports_keychain_failure(cx: &mut TestAppContext) {
        let entity = cx.update(|cx| new_holder(cx));

        let store_task = entity.update(cx, |this, cx| {
            this.api_key_state.store(
                SharedString::new("https://example.com"),
                Some("secret".to_string()),
                |this| &mut this.api_key_state,
                Arc::new(FailingWriteProvider),
                cx,
            )
        });

        assert!(store_task.await.is_err());

        let (has_key, last_store_error) = cx.update(|cx| {
            let holder = entity.read(cx);
            (
                holder.api_key_state.has_key(),
                holder.api_key_state.last_store_error(),
            )
        });
        assert!(!has_key);
        assert!(last_store_error.is_some());
    }

    #[gpui::test]
    async fn test_successful_store_loads_key_and_clears_error(cx: &mut TestAppContext) {
        let entity = cx.update(|cx| new_holder(cx));

        let store_task = entity.update(cx, |this, cx| {
            this.api_key_state.store(
                SharedString::new("https://example.com"),
                Some("secret".to_string()),
                |this| &mut this.api_key_state,
                Arc::new(SucceedingWriteProvider),
                cx,
            )
        });

        assert!(store_task.await.is_ok());

        let (has_key, last_store_error) = cx.update(|cx| {
            let holder = entity.read(cx);
            (
                holder.api_key_state.has_key(),
                holder.api_key_state.last_store_error(),
            )
        });
        assert!(has_key);
        assert!(last_store_error.is_none());
    }
}
