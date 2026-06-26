use anyhow::Result;
use collections::HashMap;
use language::{
    LanguageServerName, LspAdapter, LspAdapterDelegate, LspInstaller, Toolchain,
};
use lsp::LanguageServerBinary;
use node_runtime::{NodeRuntime, VersionStrategy};
use std::{future::Future, path::PathBuf, sync::Arc};
use util::{ResultExt, maybe};

pub struct PhpLspAdapter {
    node: NodeRuntime,
}

impl PhpLspAdapter {
    const SERVER_NAME: LanguageServerName =
        LanguageServerName::new_static("intelephense");
    const SERVER_PATH: &str = "node_modules/intelephense/lib/intelephense.js";

    pub fn new(node: NodeRuntime) -> Self {
        Self { node }
    }

    async fn get_cached_server_binary(
        container_dir: &PathBuf,
        env: HashMap<String, String>,
        node: &NodeRuntime,
    ) -> Option<lsp::LanguageServerBinary> {
        maybe!(async {
            let server_path = container_dir.join(Self::SERVER_PATH);
            anyhow::ensure!(
                server_path.exists(),
                "missing executable in directory {server_path:?}"
            );
            Ok(LanguageServerBinary {
                path: node.binary_path().await?,
                env: Some(env),
                arguments: vec![server_path.into(), "--stdio".into()],
            })
        })
        .await
        .log_err()
    }
}

impl LspInstaller for PhpLspAdapter {
    type BinaryVersion = semver::Version;

    async fn cached_server_binary(
        &self,
        container_dir: std::path::PathBuf,
        delegate: &dyn LspAdapterDelegate,
    ) -> Option<lsp::LanguageServerBinary> {
        let env = delegate.shell_env().await;
        Self::get_cached_server_binary(&container_dir, env, &self.node).await
    }

    async fn check_if_user_installed(
        &self,
        delegate: &Arc<dyn LspAdapterDelegate>,
        _: Option<Toolchain>,
        _: &gpui::AsyncApp,
    ) -> Option<lsp::LanguageServerBinary> {
        let path = delegate.which("intelephense".as_ref()).await?;
        let env = delegate.shell_env().await;

        Some(LanguageServerBinary {
            path,
            env: Some(env),
            arguments: vec!["--stdio".into()],
        })
    }

    fn check_if_version_installed(
        &self,
        version: &Self::BinaryVersion,
        container_dir: &PathBuf,
        delegate: &Arc<dyn LspAdapterDelegate>,
    ) -> impl Send + Future<Output = Option<lsp::LanguageServerBinary>> + use<> {
        let node = self.node.clone();
        let version = version.clone();
        let container_dir = container_dir.clone();
        let delegate = delegate.clone();
        async move {
            let server_path = container_dir.join(Self::SERVER_PATH);
            let should_install = node
                .should_install_npm_package(
                    Self::SERVER_NAME.as_ref(),
                    &server_path,
                    &container_dir,
                    VersionStrategy::Latest(&version),
                )
                .await;
            if should_install {
                None
            } else {
                let env = delegate.shell_env().await;
                Some(LanguageServerBinary {
                    path: node.binary_path().await.ok()?,
                    env: Some(env),
                    arguments: vec![server_path.into(), "--stdio".into()],
                })
            }
        }
    }

    async fn fetch_latest_server_version(
        &self,
        _: &Arc<dyn LspAdapterDelegate>,
        _: bool,
        _: &mut gpui::AsyncApp,
    ) -> Result<Self::BinaryVersion> {
        self.node.npm_package_latest_version("intelephense").await
    }

    fn fetch_server_binary(
        &self,
        _latest_version: Self::BinaryVersion,
        container_dir: PathBuf,
        delegate: &Arc<dyn LspAdapterDelegate>,
    ) -> impl Send + Future<Output = Result<lsp::LanguageServerBinary>> + use<> {
        let node = self.node.clone();
        let delegate = delegate.clone();
        async move {
            let server_path = container_dir.join(Self::SERVER_PATH);
            node.npm_install_latest_packages(&container_dir, &["intelephense"])
                .await?;
            anyhow::ensure!(
                server_path.exists(),
                "intelephense was not installed at {server_path:?}"
            );
            let env = delegate.shell_env().await;
            Ok(LanguageServerBinary {
                path: node.binary_path().await?,
                env: Some(env),
                arguments: vec![server_path.into(), "--stdio".into()],
            })
        }
    }
}

impl LspAdapter for PhpLspAdapter {
    fn name(&self) -> LanguageServerName {
        Self::SERVER_NAME
    }
}
