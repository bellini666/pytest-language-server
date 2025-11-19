package com.github.bellini666.pytestlsp

import com.intellij.openapi.diagnostic.Logger
import com.intellij.openapi.project.Project
import com.intellij.openapi.startup.ProjectActivity

/**
 * Startup activity for project lifecycle events.
 *
 * Note: LSP4IJ handles the actual language server lifecycle automatically.
 * This activity is only for logging and diagnostics.
 */
class PytestLanguageServerListener : ProjectActivity {

    private val LOG = Logger.getInstance(PytestLanguageServerListener::class.java)

    override suspend fun execute(project: Project) {
        LOG.info("pytest Language Server plugin activated for project: ${project.name}")

        // Verify that the executable can be found
        // LSP4IJ will handle starting the server automatically when needed
        val service = PytestLanguageServerService.getInstance(project)
        val executablePath = service.getExecutablePath()

        if (executablePath == null) {
            LOG.warn("pytest-language-server executable not found. The language server will not start.")
            LOG.warn("To configure a custom binary path, add VM option: -Dpytest.lsp.executable=/path/to/binary")
            LOG.warn("To use system PATH, add VM option: -Dpytest.lsp.useSystemPath=true")
        } else {
            LOG.info("pytest-language-server executable located at: $executablePath")
            LOG.info("Language server will start automatically when Python test files are opened")
        }
    }
}
