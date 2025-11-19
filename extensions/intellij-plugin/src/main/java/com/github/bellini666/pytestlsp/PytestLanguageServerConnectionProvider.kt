package com.github.bellini666.pytestlsp

import com.intellij.execution.configurations.GeneralCommandLine
import com.intellij.openapi.diagnostic.Logger
import com.intellij.openapi.project.Project
import com.redhat.devtools.lsp4ij.server.ProcessStreamConnectionProvider

/**
 * Connection provider for pytest Language Server.
 * Handles starting the language server process and managing the stdio connection.
 */
class PytestLanguageServerConnectionProvider(
    private val project: Project
) : ProcessStreamConnectionProvider() {

    private val LOG = Logger.getInstance(PytestLanguageServerConnectionProvider::class.java)

    override fun start() {
        val service = PytestLanguageServerService.getInstance(project)
        val executablePath = service.getExecutablePath()

        if (executablePath == null) {
            LOG.error("pytest-language-server executable not found")
            throw IllegalStateException(
                "pytest-language-server executable not found. " +
                "Please ensure the bundled binary is present or configure a custom path."
            )
        }

        LOG.info("Starting pytest-language-server from: $executablePath")

        // Create command line
        val commands = listOf(executablePath)
        setCommands(commands)

        // Set working directory to project root
        project.basePath?.let { basePath ->
            setWorkingDirectory(basePath)
        }

        // Start the language server process
        super.start()

        LOG.info("pytest-language-server started successfully")
    }

    override fun toString(): String = "pytest Language Server"
}
