package com.github.bellini666.pytestlsp

import com.intellij.openapi.diagnostic.Logger
import com.intellij.openapi.project.Project
import com.intellij.openapi.project.ProjectManagerListener

class PytestLanguageServerListener : ProjectManagerListener {

    private val LOG = Logger.getInstance(PytestLanguageServerListener::class.java)

    override fun projectOpened(project: Project) {
        LOG.info("pytest Language Server plugin activated for project: ${project.name}")

        val service = PytestLanguageServerService.getInstance(project)
        val executablePath = service.getExecutablePath()

        if (executablePath == null) {
            LOG.warn("pytest-language-server binary not found. Please install it or configure the path.")
        } else {
            LOG.info("pytest Language Server ready at: $executablePath")
        }
    }
}
