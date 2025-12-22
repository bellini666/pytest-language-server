package com.github.bellini666.pytestlsp

import com.intellij.execution.configurations.GeneralCommandLine
import com.intellij.openapi.diagnostic.Logger
import com.intellij.openapi.project.Project
import com.intellij.openapi.vfs.VirtualFile
import com.intellij.platform.lsp.api.LspServerSupportProvider
import com.intellij.platform.lsp.api.LspServerSupportProvider.LspServerStarter
import com.intellij.platform.lsp.api.ProjectWideLspServerDescriptor
import java.io.File

/**
 * LSP Server Support Provider for pytest Language Server.
 * Uses the native IntelliJ LSP API (available since 2023.2).
 */
@Suppress("UnstableApiUsage")
class PytestLspServerSupportProvider : LspServerSupportProvider {

    override fun fileOpened(
        project: Project,
        file: VirtualFile,
        serverStarter: LspServerStarter
    ) {
        if (file.isPytestFile()) {
            serverStarter.ensureServerStarted(PytestLspServerDescriptor(project))
        }
    }
}

/**
 * Check if a file is a pytest-related file.
 */
private fun VirtualFile.isPytestFile(): Boolean {
    if (extension != "py") return false
    val name = this.name
    return name.startsWith("test_") || name.endsWith("_test.py") || name == "conftest.py"
}

/**
 * LSP Server Descriptor for pytest Language Server.
 * Configures how the language server is started and which files it handles.
 */
@Suppress("UnstableApiUsage")
class PytestLspServerDescriptor(project: Project) :
    ProjectWideLspServerDescriptor(project, "pytest Language Server") {

    private val LOG = Logger.getInstance(PytestLspServerDescriptor::class.java)

    override fun isSupportedFile(file: VirtualFile): Boolean = file.isPytestFile()

    override fun createCommandLine(): GeneralCommandLine {
        val service = PytestLanguageServerService.getInstance(project)
        val executablePath = service.getExecutablePath()

        if (executablePath == null) {
            LOG.error("pytest-language-server executable not found")
            return GeneralCommandLine()
        }

        LOG.info("Starting pytest-language-server from: $executablePath")

        return GeneralCommandLine(executablePath).apply {
            // Set working directory to project root
            project.basePath?.let { workDirectory = File(it) }
        }
    }
}
