use tau_extension_api::{
    self as tau, Architecture, DownloadedFileType, GithubReleaseOptions, Os, Result,
    settings::LspSettings,
};

use crate::language_servers::util;

pub(crate) struct ProtoLs {
    cached_binary_path: Option<String>,
}

impl ProtoLs {
    pub(crate) const SERVER_NAME: &str = "protols";

    pub(crate) fn new() -> Self {
        ProtoLs {
            cached_binary_path: None,
        }
    }

    pub(crate) fn language_server_binary(
        &mut self,
        worktree: &tau::Worktree,
    ) -> Result<tau::Command> {
        let binary_settings = LspSettings::for_worktree(Self::SERVER_NAME, worktree)
            .ok()
            .and_then(|lsp_settings| lsp_settings.binary);

        let args = binary_settings
            .as_ref()
            .and_then(|binary_settings| binary_settings.arguments.clone())
            .unwrap_or_default();

        let env = worktree.shell_env();

        if let Some(path) = binary_settings.and_then(|binary_settings| binary_settings.path) {
            return Ok(tau::Command {
                command: path,
                args,
                env,
            });
        } else if let Some(path) = self.cached_binary_path.clone() {
            return Ok(tau::Command {
                command: path,
                args,
                env,
            });
        } else if let Some(path) = worktree.which(Self::SERVER_NAME) {
            self.cached_binary_path = Some(path.clone());
            return Ok(tau::Command {
                command: path,
                args,
                env,
            });
        }

        let latest_release = tau::latest_github_release(
            "coder3101/protols",
            GithubReleaseOptions {
                require_assets: true,
                pre_release: false,
            },
        )?;

        let (os, arch) = tau::current_platform();

        let release_suffix = match (os, arch) {
            (Os::Mac, Architecture::Aarch64) => "aarch64-apple-darwin.tar.gz",
            (Os::Mac, Architecture::X8664) => "x86_64-apple-darwin.tar.gz",
            (Os::Linux, Architecture::Aarch64) => "aarch64-unknown-linux-gnu.tar.gz",
            (Os::Linux, Architecture::X8664) => "x86_64-unknown-linux-gnu.tar.gz",
            (Os::Windows, Architecture::X8664) => "x86_64-pc-windows-msvc.zip",
            _ => {
                return Err("Platform and architecture not supported by Protols".to_string());
            }
        };

        let release_name = format!("protols-{release_suffix}");

        let file_type = if os == Os::Windows {
            DownloadedFileType::Zip
        } else {
            DownloadedFileType::GzipTar
        };

        let version_dir = format!("{}-{}", Self::SERVER_NAME, latest_release.version);
        let binary_path = format!("{version_dir}/protols");

        let download_target = latest_release
            .assets
            .into_iter()
            .find(|asset| asset.name == release_name)
            .ok_or_else(|| {
                format!(
                    "Could not find asset with name {} in Protols release",
                    &release_name
                )
            })?;

        tau::download_file(&download_target.download_url, &version_dir, file_type)?;
        tau::make_file_executable(&binary_path)?;

        util::remove_outdated_versions(Self::SERVER_NAME, &version_dir)?;

        self.cached_binary_path = Some(binary_path.clone());

        Ok(tau::Command {
            command: binary_path,
            args,
            env,
        })
    }
}
