use std::fs;
use tau::lsp::CompletionKind;
use tau::{CodeLabel, CodeLabelSpan, LanguageServerId};
use tau_extension_api::process::Command;
use tau_extension_api::{self as tau, Result};

struct TestExtension {
    cached_binary_path: Option<String>,
}

impl TestExtension {
    fn language_server_binary_path(
        &mut self,
        language_server_id: &LanguageServerId,
        _worktree: &tau::Worktree,
    ) -> Result<String> {
        let (platform, arch) = tau::current_platform();

        let current_dir = std::env::current_dir().unwrap();
        println!("current_dir: {}", current_dir.display());
        assert_eq!(
            current_dir.file_name().unwrap().to_str().unwrap(),
            "test-extension"
        );

        fs::create_dir_all(current_dir.join("dir-created-with-abs-path")).unwrap();
        fs::create_dir_all("./dir-created-with-rel-path").unwrap();
        fs::write("file-created-with-rel-path", b"contents 1").unwrap();
        fs::write(
            current_dir.join("file-created-with-abs-path"),
            b"contents 2",
        )
        .unwrap();
        assert_eq!(
            fs::read("file-created-with-rel-path").unwrap(),
            b"contents 1"
        );
        assert_eq!(
            fs::read("file-created-with-abs-path").unwrap(),
            b"contents 2"
        );

        let command = match platform {
            tau::Os::Linux | tau::Os::Mac => Command::new("echo"),
            tau::Os::Windows => Command::new("cmd").args(["/C", "echo"]),
        };
        let output = command.arg("hello from a child process!").output()?;
        println!(
            "command output: {}",
            String::from_utf8_lossy(&output.stdout).trim()
        );

        if let Some(path) = &self.cached_binary_path
            && fs::metadata(path).is_ok_and(|stat| stat.is_file())
        {
            return Ok(path.clone());
        }

        tau::set_language_server_installation_status(
            language_server_id,
            &tau::LanguageServerInstallationStatus::CheckingForUpdate,
        );
        let release = tau::latest_github_release(
            "gleam-lang/gleam",
            tau::GithubReleaseOptions {
                require_assets: true,
                pre_release: false,
            },
        )?;

        let ext = "tar.gz";
        let download_type = tau::DownloadedFileType::GzipTar;

        // Do this if you want to actually run this extension -
        // the actual asset is a .zip. But the integration test is simpler
        // if every platform uses .tar.gz.
        //
        // ext = "zip";
        // download_type = tau::DownloadedFileType::Zip;

        let asset_name = format!(
            "gleam-{version}-{arch}-{os}.{ext}",
            version = release.version,
            arch = match arch {
                tau::Architecture::Aarch64 => "aarch64",
                tau::Architecture::X8664 => "x86_64",
            },
            os = match platform {
                tau::Os::Mac => "apple-darwin",
                tau::Os::Linux => "unknown-linux-musl",
                tau::Os::Windows => "pc-windows-msvc",
            },
        );

        let asset = release
            .assets
            .iter()
            .find(|asset| asset.name == asset_name)
            .ok_or_else(|| format!("no asset found matching {:?}", asset_name))?;

        let version_dir = format!("gleam-{}", release.version);
        let binary_path = format!("{version_dir}/gleam");

        if !fs::metadata(&binary_path).is_ok_and(|stat| stat.is_file()) {
            tau::set_language_server_installation_status(
                language_server_id,
                &tau::LanguageServerInstallationStatus::Downloading,
            );

            tau::download_file(&asset.download_url, &version_dir, download_type)
                .map_err(|e| format!("failed to download file: {e}"))?;

            tau::set_language_server_installation_status(
                language_server_id,
                &tau::LanguageServerInstallationStatus::None,
            );

            let entries =
                fs::read_dir(".").map_err(|e| format!("failed to list working directory {e}"))?;
            for entry in entries {
                let entry = entry.map_err(|e| format!("failed to load directory entry {e}"))?;
                let filename = entry.file_name();
                let filename = filename.to_str().unwrap();
                if filename.starts_with("gleam-") && filename != version_dir {
                    fs::remove_dir_all(entry.path()).ok();
                }
            }
        }

        self.cached_binary_path = Some(binary_path.clone());
        Ok(binary_path)
    }
}

impl tau::Extension for TestExtension {
    fn new() -> Self {
        Self {
            cached_binary_path: None,
        }
    }

    fn language_server_command(
        &mut self,
        language_server_id: &LanguageServerId,
        worktree: &tau::Worktree,
    ) -> Result<tau::Command> {
        Ok(tau::Command {
            command: self.language_server_binary_path(language_server_id, worktree)?,
            args: vec!["lsp".to_string()],
            env: Default::default(),
        })
    }

    fn label_for_completion(
        &self,
        _language_server_id: &LanguageServerId,
        completion: tau::lsp::Completion,
    ) -> Option<tau::CodeLabel> {
        let name = &completion.label;
        let ty = strip_newlines_from_detail(&completion.detail?);
        let let_binding = "let a";
        let colon = ": ";
        let assignment = " = ";
        let call = match completion.kind? {
            CompletionKind::Function | CompletionKind::Constructor => "()",
            _ => "",
        };
        let code = format!("{let_binding}{colon}{ty}{assignment}{name}{call}");

        Some(CodeLabel {
            spans: vec![
                CodeLabelSpan::code_range({
                    let start = let_binding.len() + colon.len() + ty.len() + assignment.len();
                    start..start + name.len()
                }),
                CodeLabelSpan::code_range({
                    let start = let_binding.len();
                    start..start + colon.len()
                }),
                CodeLabelSpan::code_range({
                    let start = let_binding.len() + colon.len();
                    start..start + ty.len()
                }),
            ],
            filter_range: (0..name.len()).into(),
            code,
        })
    }
}

tau::register_extension!(TestExtension);

/// Removes newlines from the completion detail.
///
/// The Gleam LSP can return types containing newlines, which causes formatting
/// issues within the Tau completions menu.
fn strip_newlines_from_detail(detail: &str) -> String {
    let without_newlines = detail
        .replace("->\n  ", "-> ")
        .replace("\n  ", "")
        .replace(",\n", "");

    let comma_delimited_parts = without_newlines.split(',');
    comma_delimited_parts
        .map(|part| part.trim())
        .collect::<Vec<_>>()
        .join(", ")
}
