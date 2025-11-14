use std::fs;
use zed::settings::LspSettings;
use zed_extension_api::{self as zed, Result};

struct PytestLspBinary {
    path: String,
    args: Option<Vec<String>>,
    environment: Option<Vec<(String, String)>>,
}

struct PytestLspExtension {
    cached_binary_path: Option<String>,
}

impl PytestLspExtension {
    fn language_server_binary(
        &mut self,
        language_server_id: &zed::LanguageServerId,
        worktree: &zed::Worktree,
    ) -> Result<PytestLspBinary> {
        let mut args: Option<Vec<String>> = None;

        let (platform, arch) = zed::current_platform();

        let environment = match platform {
            zed::Os::Mac | zed::Os::Linux => Some(worktree.shell_env()),
            zed::Os::Windows => None,
        };

        // Check for user-configured binary in settings
        if let Ok(lsp_settings) = LspSettings::for_worktree("pytest-lsp", worktree) {
            if let Some(binary) = lsp_settings.binary {
                args = binary.arguments;
                if let Some(path) = binary.path {
                    return Ok(PytestLspBinary {
                        path: path.clone(),
                        args,
                        environment,
                    });
                }
            }
        }

        // Check if pytest-language-server is in PATH
        if let Some(path) = worktree.which("pytest-language-server") {
            return Ok(PytestLspBinary {
                path,
                args,
                environment,
            });
        }

        // Check cached binary
        if let Some(path) = &self.cached_binary_path {
            if fs::metadata(path).is_ok_and(|stat| stat.is_file()) {
                return Ok(PytestLspBinary {
                    path: path.clone(),
                    args,
                    environment,
                });
            }
        }

        zed::set_language_server_installation_status(
            language_server_id,
            &zed::LanguageServerInstallationStatus::CheckingForUpdate,
        );

        let release = zed::latest_github_release(
            "bellini666/pytest-language-server",
            zed::GithubReleaseOptions {
                require_assets: true,
                pre_release: false,
            },
        )?;

        let arch: &str = match arch {
            zed::Architecture::Aarch64 => "aarch64",
            zed::Architecture::X8664 => "x86_64",
            zed::Architecture::X86 => return Err("unsupported platform x86".into()),
        };

        let os: &str = match platform {
            zed::Os::Mac => "apple-darwin",
            zed::Os::Linux => "unknown-linux-musl",
            zed::Os::Windows => "pc-windows-msvc",
        };

        let extension: &str = match platform {
            zed::Os::Mac | zed::Os::Linux => "tar.gz",
            zed::Os::Windows => "zip",
        };

        // Asset name format from GitHub releases: pytest-language-server-v0.3.0-x86_64-apple-darwin.tar.gz
        let asset_name: String = format!(
            "pytest-language-server-{}-{}-{}.{}",
            release.version, arch, os, extension
        );
        let download_url = format!(
            "https://github.com/bellini666/pytest-language-server/releases/download/{}/{}",
            release.version, asset_name
        );

        let version_dir = format!("pytest-language-server-{}", release.version);
        let binary_path = match platform {
            zed::Os::Mac | zed::Os::Linux => format!("{version_dir}/pytest-language-server"),
            zed::Os::Windows => format!("{version_dir}/pytest-language-server.exe"),
        };

        if !fs::metadata(&binary_path).is_ok_and(|stat| stat.is_file()) {
            zed::set_language_server_installation_status(
                language_server_id,
                &zed::LanguageServerInstallationStatus::Downloading,
            );

            zed::download_file(
                &download_url,
                &version_dir,
                match platform {
                    zed::Os::Mac | zed::Os::Linux => zed::DownloadedFileType::GzipTar,
                    zed::Os::Windows => zed::DownloadedFileType::Zip,
                },
            )
            .map_err(|e| format!("failed to download file: {e}"))?;

            zed::make_file_executable(&binary_path)?;

            // Clean up old versions
            let entries =
                fs::read_dir(".").map_err(|e| format!("failed to list working directory {e}"))?;
            for entry in entries {
                let entry = entry.map_err(|e| format!("failed to load directory entry {e}"))?;
                if entry.file_name().to_str() != Some(&version_dir) {
                    fs::remove_dir_all(entry.path()).ok();
                }
            }
        }

        self.cached_binary_path = Some(binary_path.clone());
        Ok(PytestLspBinary {
            path: binary_path,
            args,
            environment,
        })
    }
}

impl zed::Extension for PytestLspExtension {
    fn new() -> Self {
        Self {
            cached_binary_path: None,
        }
    }

    fn language_server_command(
        &mut self,
        language_server_id: &zed::LanguageServerId,
        worktree: &zed::Worktree,
    ) -> Result<zed::Command> {
        let binary = self.language_server_binary(language_server_id, worktree)?;
        Ok(zed::Command {
            command: binary.path,
            args: binary.args.unwrap_or_default(),
            env: binary.environment.unwrap_or_default(),
        })
    }

    fn language_server_workspace_configuration(
        &mut self,
        language_server_id: &zed::LanguageServerId,
        worktree: &zed::Worktree,
    ) -> Result<Option<zed::serde_json::Value>> {
        let settings = LspSettings::for_worktree(language_server_id.as_ref(), worktree)
            .ok()
            .and_then(|lsp_settings| lsp_settings.settings.clone())
            .unwrap_or_default();
        Ok(Some(settings))
    }
}

zed::register_extension!(PytestLspExtension);
