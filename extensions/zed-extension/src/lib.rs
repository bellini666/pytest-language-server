use zed::settings::LspSettings;
use zed_extension_api::{self as zed, Result};

struct PytestLspExtension {
    cached_binary_path: Option<String>,
}

impl zed::Extension for PytestLspExtension {
    fn new() -> Self {
        Self {
            cached_binary_path: None,
        }
    }

    fn language_server_command(
        &mut self,
        _language_server_id: &zed::LanguageServerId,
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
            self.get_binary_path(platform, arch, worktree)?
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
    ) -> Result<String> {
        // Priority 1: Try PATH first (user may have installed via pip)
        if let Some(path) = worktree.which("pytest-language-server") {
            self.cached_binary_path = Some(path.clone());
            return Ok(path);
        }

        // Priority 2: Use bundled binary
        let binary_name = match platform {
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
        };

        let bundled_path = format!("bin/{}", binary_name);

        // Ensure the bundled binary is executable (no-op on Windows)
        zed::make_file_executable(&bundled_path)?;

        self.cached_binary_path = Some(bundled_path.clone());
        Ok(bundled_path)
    }
}

zed::register_extension!(PytestLspExtension);
