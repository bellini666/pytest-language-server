package com.github.bellini666.pytestlsp

import com.intellij.notification.NotificationGroupManager
import com.intellij.notification.NotificationType
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
            // Check if user explicitly configured custom path or system PATH
            val customPath = System.getProperty("pytest.lsp.executable")
            val useSystemPath = System.getProperty("pytest.lsp.useSystemPath")?.toBoolean() ?: false

            if (customPath != null || useSystemPath) {
                // User explicitly configured a custom location, show helpful message
                NotificationGroupManager.getInstance()
                    .getNotificationGroup("pytest Language Server")
                    .createNotification(
                        "pytest Language Server: Binary not found",
                        if (customPath != null) {
                            "Custom pytest-language-server binary not found at: <code>$customPath</code><br>" +
                                    "Please check the path or install it via: <code>pip install pytest-language-server</code>"
                        } else {
                            "pytest-language-server not found in system PATH.<br>" +
                                    "Please install it via: <code>pip install pytest-language-server</code><br>" +
                                    "Or install via cargo: <code>cargo install pytest-language-server</code><br>" +
                                    "Or install via homebrew: <code>brew install pytest-language-server</code>"
                        },
                        NotificationType.ERROR
                    )
                    .notify(project)
            } else {
                // Bundled binary not found - this is a packaging error
                NotificationGroupManager.getInstance()
                    .getNotificationGroup("pytest Language Server")
                    .createNotification(
                        "pytest Language Server: Packaging error",
                        "The bundled pytest-language-server binary was not found. " +
                                "This is a plugin packaging error.<br>" +
                                "Please report this issue at: " +
                                "<a href='https://github.com/bellini666/pytest-language-server/issues'>GitHub Issues</a><br><br>" +
                                "Workaround: Install pytest-language-server manually and configure the plugin to use it:<br>" +
                                "1. Install: <code>pip install pytest-language-server</code><br>" +
                                "2. Add VM option: <code>-Dpytest.lsp.useSystemPath=true</code>",
                        NotificationType.ERROR
                    )
                    .notify(project)
            }
        } else {
            LOG.info("pytest Language Server ready at: $executablePath")
        }
    }
}
