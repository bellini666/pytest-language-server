package com.github.bellini666.pytestlsp

import com.intellij.openapi.project.Project
import com.redhat.devtools.lsp4ij.LanguageServerFactory
import com.redhat.devtools.lsp4ij.client.LanguageClientImpl
import com.redhat.devtools.lsp4ij.server.StreamConnectionProvider

/**
 * Factory for creating pytest Language Server instances.
 */
class PytestLanguageServerFactory : LanguageServerFactory {

    override fun createConnectionProvider(project: Project): StreamConnectionProvider {
        return PytestLanguageServerConnectionProvider(project)
    }

    override fun createLanguageClient(project: Project): LanguageClientImpl {
        return PytestLanguageClient(project)
    }
}

/**
 * Language client for pytest Language Server.
 */
class PytestLanguageClient(project: Project) : LanguageClientImpl(project)
