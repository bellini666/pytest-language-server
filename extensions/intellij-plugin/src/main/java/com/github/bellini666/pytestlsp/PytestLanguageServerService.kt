package com.github.bellini666.pytestlsp

import com.intellij.openapi.components.Service
import com.intellij.openapi.diagnostic.Logger
import com.intellij.openapi.project.Project
import com.intellij.openapi.util.SystemInfo
import java.io.File
import java.nio.file.Files
import java.nio.file.StandardCopyOption

@Service(Service.Level.PROJECT)
class PytestLanguageServerService(private val project: Project) {

    private val LOG = Logger.getInstance(PytestLanguageServerService::class.java)

    fun getExecutablePath(): String? {
        // First, check if user configured a custom path
        val customPath = System.getProperty("pytest.lsp.executable")
        if (customPath != null && File(customPath).exists()) {
            LOG.info("Using custom pytest-language-server from: $customPath")
            return customPath
        }

        // Try to find in PATH
        val pathExecutable = findInPath()
        if (pathExecutable != null) {
            LOG.info("Using pytest-language-server from PATH: $pathExecutable")
            return pathExecutable
        }

        // Use bundled binary
        val bundledPath = getBundledBinaryPath()
        if (bundledPath != null && File(bundledPath).exists()) {
            LOG.info("Using bundled pytest-language-server: $bundledPath")
            return bundledPath
        }

        LOG.error("pytest-language-server binary not found")
        return null
    }

    private fun findInPath(): String? {
        val pathEnv = System.getenv("PATH") ?: return null
        val pathSeparator = if (SystemInfo.isWindows) ";" else ":"
        val executable = if (SystemInfo.isWindows) "pytest-language-server.exe" else "pytest-language-server"

        pathEnv.split(pathSeparator).forEach { dir ->
            val file = File(dir, executable)
            if (file.exists() && file.canExecute()) {
                return file.absolutePath
            }
        }

        return null
    }

    private fun getBundledBinaryPath(): String? {
        val binaryName = when {
            SystemInfo.isWindows -> "pytest-language-server.exe"
            SystemInfo.isMac -> {
                if (SystemInfo.isAarch64) {
                    "pytest-language-server-aarch64-apple-darwin"
                } else {
                    "pytest-language-server-x86_64-apple-darwin"
                }
            }
            SystemInfo.isLinux -> {
                if (SystemInfo.isAarch64) {
                    "pytest-language-server-aarch64-unknown-linux-gnu"
                } else {
                    "pytest-language-server-x86_64-unknown-linux-gnu"
                }
            }
            else -> {
                LOG.error("Unsupported platform: ${SystemInfo.OS_NAME}")
                return null
            }
        }

        // Extract bundled binary to temp location
        val pluginDir = this::class.java.protectionDomain.codeSource.location.toURI().path
        val binDir = File(pluginDir, "bin")
        val bundledBinary = File(binDir, binaryName)

        if (bundledBinary.exists()) {
            // Ensure executable permissions on Unix-like systems
            if (!SystemInfo.isWindows) {
                bundledBinary.setExecutable(true)
            }
            return bundledBinary.absolutePath
        }

        return null
    }

    companion object {
        fun getInstance(project: Project): PytestLanguageServerService {
            return project.getService(PytestLanguageServerService::class.java)
        }
    }
}
