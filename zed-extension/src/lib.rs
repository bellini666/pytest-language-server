use zed::settings::LspSettings;
use zed_extension_api::{self as zed, Result};

struct PytestLspExtension;

impl zed::Extension for PytestLspExtension {
    fn new() -> Self {
        Self
    }

    fn language_server_command(
        &mut self,
        _language_server_id: &zed::LanguageServerId,
        worktree: &zed::Worktree,
    ) -> Result<zed::Command> {
        let (platform, _) = zed::current_platform();

        // Get shell environment for proper PATH resolution
        let environment = match platform {
            zed::Os::Mac | zed::Os::Linux => worktree.shell_env(),
            zed::Os::Windows => vec![],
        };

        // Try to find pytest-language-server in PATH
        let binary_path = worktree
            .which("pytest-language-server")
            .ok_or_else(|| {
                "pytest-language-server not found in PATH. Please install it with: pip install pytest-language-server".to_string()
            })?;

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

zed::register_extension!(PytestLspExtension);
