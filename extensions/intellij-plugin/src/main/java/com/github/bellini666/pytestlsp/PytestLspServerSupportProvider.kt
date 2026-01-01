package com.github.bellini666.pytestlsp

import com.intellij.execution.configurations.GeneralCommandLine
import com.intellij.openapi.diagnostic.Logger
import com.intellij.openapi.extensions.ExtensionNotApplicableException
import com.intellij.openapi.extensions.PluginId
import com.intellij.openapi.project.Project
import com.intellij.openapi.vfs.VirtualFile
import com.intellij.platform.lsp.api.LspServerSupportProvider
import com.intellij.platform.lsp.api.LspServerSupportProvider.LspServerStarter
import com.intellij.platform.lsp.api.ProjectWideLspServerDescriptor
import com.intellij.ide.plugins.PluginManagerCore
import java.io.File

/**
 * LSP Server Support Provider for pytest Language Server.
 * Uses the native IntelliJ LSP API (available since 2023.2).
 *
 * This is the primary provider, registered via lsp-module.xml for IDEs with
 * com.intellij.modules.lsp (2025.2+ and unified PyCharm 2025.1+).
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
 * Fallback LSP Server Support Provider for older commercial IDEs (2023.2 - 2025.1).
 * Registered via ultimate-module.xml for IDEs with com.intellij.modules.ultimate.
 *
 * This provider throws ExtensionNotApplicableException if com.intellij.modules.lsp
 * is available, since PytestLspServerSupportProvider will handle it instead.
 * This prevents duplicate server entries in the "Language Services" UI when
 * both modules are present (PyCharm Professional 2025.2+).
 *
 * Using ExtensionNotApplicableException in the constructor is the documented
 * JetBrains way to conditionally opt-out of extension registration at runtime.
 * See: https://plugins.jetbrains.com/docs/intellij/plugin-extensions.html
 */
@Suppress("UnstableApiUsage")
class PytestLspServerSupportProviderFallback : LspServerSupportProvider {

    init {
        // Check if the primary LSP module is available
        // If so, throw ExtensionNotApplicableException to prevent this fallback
        // provider from being registered (avoiding duplicate entries in Language Services UI)
        if (isLspModuleAvailable()) {
            throw ExtensionNotApplicableException.create()
        }
    }

    override fun fileOpened(
        project: Project,
        file: VirtualFile,
        serverStarter: LspServerStarter
    ) {
        if (file.isPytestFile()) {
            serverStarter.ensureServerStarted(PytestLspServerDescriptor(project))
        }
    }

    companion object {
        private val LOG = Logger.getInstance(PytestLspServerSupportProviderFallback::class.java)

        /**
         * Check if com.intellij.modules.lsp is available.
         * This module is present in:
         * - Unified PyCharm 2025.1+ (free tier)
         * - All commercial IDEs 2025.2+
         */
        private fun isLspModuleAvailable(): Boolean {
            return try {
                val lspModuleId = PluginId.getId("com.intellij.modules.lsp")
                val isAvailable = PluginManagerCore.getPlugin(lspModuleId)?.isEnabled == true
                if (isAvailable) {
                    LOG.info("com.intellij.modules.lsp is available, fallback provider will not be registered")
                }
                isAvailable
            } catch (e: Exception) {
                LOG.debug("Could not check for LSP module: ${e.message}")
                false
            }
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
