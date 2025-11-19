use std::fs;
use zed::settings::LspSettings;
use zed_extension_api::{
    self as zed, DownloadedFileType, GithubReleaseOptions, LanguageServerId,
    LanguageServerInstallationStatus, Result,
};

struct PytestLspExtension {
    cached_binary_path: Option<String>,
}

const OWNER: &str = "bellini666";
const REPO: &str = "pytest-language-server";

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
        let (platform, arch) = zed::current_platform();

        // Get shell environment for proper PATH resolution
        let environment = match platform {
            zed::Os::Mac | zed::Os::Linux => worktree.shell_env(),
            zed::Os::Windows => vec![],
        };

        // Try to get binary path (cached or fresh)
        let binary_path = if let Some(ref cached) = self.cached_binary_path {
            cached.clone()
        } else {
            self.get_binary_path(platform, arch, worktree, language_server_id)?
        };

        Ok(zed::Command {
            command: binary_path,
            args: vec![],
            env: environment,
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

impl PytestLspExtension {
    fn get_binary_path(
        &mut self,
        platform: zed::Os,
        arch: zed::Architecture,
        worktree: &zed::Worktree,
        language_server_id: &LanguageServerId,
    ) -> Result<String> {
        // Priority 1: Try PATH first (user may have installed via pip/cargo/brew)
        if let Some(path) = worktree.which("pytest-language-server") {
            self.cached_binary_path = Some(path.clone());
            return Ok(path);
        }

        // Priority 2: Download binary from GitHub releases
        let binary_name = self.get_binary_name(platform, arch)?;
        let binary_path = format!("bin/{}", binary_name);

        // Check if already downloaded
        if fs::metadata(&binary_path).is_ok() {
            zed::make_file_executable(&binary_path)?;
            self.cached_binary_path = Some(binary_path.clone());
            return Ok(binary_path);
        }

        // Download from GitHub release
        zed::set_language_server_installation_status(
            language_server_id,
            &LanguageServerInstallationStatus::Downloading,
        );

        let repo = format!("{}/{}", OWNER, REPO);
        let release = zed::latest_github_release(
            &repo,
            GithubReleaseOptions {
                require_assets: true,
                pre_release: false,
            },
        )?;

        let asset_name = binary_name;
        let asset = release
            .assets
            .iter()
            .find(|asset| asset.name == asset_name)
            .ok_or_else(|| {
                format!(
                    "no asset found matching {:?} in release {}",
                    asset_name, release.version
                )
            })?;

        // Create bin directory if it doesn't exist
        fs::create_dir_all("bin").map_err(|e| format!("failed to create bin directory: {}", e))?;

        zed::download_file(
            &asset.download_url,
            &binary_path,
            DownloadedFileType::Uncompressed,
        )
        .map_err(|e| format!("failed to download file: {}", e))?;

        zed::make_file_executable(&binary_path)?;

        // Clear installation status (download complete)
        zed::set_language_server_installation_status(
            language_server_id,
            &LanguageServerInstallationStatus::None,
        );

        self.cached_binary_path = Some(binary_path.clone());
        Ok(binary_path)
    }

    fn get_binary_name(&self, platform: zed::Os, arch: zed::Architecture) -> Result<String> {
        Ok(match platform {
            zed::Os::Mac => match arch {
                zed::Architecture::Aarch64 => "pytest-language-server-aarch64-apple-darwin",
                zed::Architecture::X8664 => "pytest-language-server-x86_64-apple-darwin",
                _ => return Err("Unsupported macOS architecture".to_string()),
            },
            zed::Os::Linux => match arch {
                zed::Architecture::Aarch64 => "pytest-language-server-aarch64-unknown-linux-gnu",
                zed::Architecture::X8664 => "pytest-language-server-x86_64-unknown-linux-gnu",
                _ => return Err("Unsupported Linux architecture".to_string()),
            },
            zed::Os::Windows => "pytest-language-server.exe",
        }
        .to_string())
    }
}

zed::register_extension!(PytestLspExtension);
