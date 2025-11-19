package com.github.bellini666.pytestlsp

import com.intellij.ide.plugins.PluginManagerCore
import com.intellij.openapi.components.Service
import com.intellij.openapi.diagnostic.Logger
import com.intellij.openapi.extensions.PluginId
import com.intellij.openapi.project.Project
import com.intellij.openapi.util.SystemInfo
import com.intellij.util.system.CpuArch
import java.io.File
import java.nio.file.Files
import java.nio.file.StandardCopyOption

@Service(Service.Level.PROJECT)
class PytestLanguageServerService(private val project: Project) {

    private val LOG = Logger.getInstance(PytestLanguageServerService::class.java)

    /**
     * Gets the path to the pytest-language-server executable.
     *
     * Priority order:
     * 1. Custom path via system property: -Dpytest.lsp.executable=/path/to/binary
     * 2. System PATH via system property: -Dpytest.lsp.useSystemPath=true
     * 3. Bundled binary (default)
     *
     * @return The path to the executable, or null if not found
     */
    fun getExecutablePath(): String? {
        // Check if user explicitly configured a custom path or wants to use PATH
        val customPath = System.getProperty("pytest.lsp.executable")
        val useSystemPath = System.getProperty("pytest.lsp.useSystemPath")?.toBoolean() ?: false

        if (customPath != null) {
            // User specified a custom path
            val file = File(customPath)
            if (file.exists()) {
                LOG.info("Using custom pytest-language-server from: $customPath")
                return customPath
            } else {
                LOG.error("Custom pytest-language-server path does not exist: $customPath")
                return null
            }
        }

        if (useSystemPath) {
            // User wants to use system PATH
            val pathExecutable = findInPath()
            if (pathExecutable != null) {
                LOG.info("Using pytest-language-server from PATH: $pathExecutable")
                return pathExecutable
            } else {
                LOG.error("pytest-language-server not found in PATH. Install via: pip install pytest-language-server")
                return null
            }
        }

        // Default: use bundled binary
        val bundledPath = getBundledBinaryPath()
        if (bundledPath != null) {
            LOG.info("Using bundled pytest-language-server: $bundledPath")
            return bundledPath
        }

        // This is an error - bundled binary should always be present in releases
        LOG.error("Bundled pytest-language-server binary not found. This is a packaging error. Please report at: https://github.com/bellini666/pytest-language-server/issues")
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
                if (CpuArch.isArm64()) {
                    "pytest-language-server-aarch64-apple-darwin"
                } else {
                    "pytest-language-server-x86_64-apple-darwin"
                }
            }
            SystemInfo.isLinux -> {
                if (CpuArch.isArm64()) {
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

        // Get plugin directory using IntelliJ's plugin API
        val pluginId = PluginId.getId("com.github.bellini666.pytest-language-server")
        val pluginDescriptor = PluginManagerCore.getPlugin(pluginId)
        if (pluginDescriptor == null) {
            LOG.error("Failed to find plugin descriptor")
            return null
        }

        val pluginPath = pluginDescriptor.pluginPath
        LOG.info("Plugin path: $pluginPath")

        // Try multiple possible locations for the binary
        val possibleLocations = listOf(
            pluginPath.resolve("lib/bin/$binaryName"),      // Inside plugin lib
            pluginPath.resolve("bin/$binaryName"),           // Direct bin directory
            pluginPath.resolve("pytest-language-server/lib/bin/$binaryName")  // Nested structure
        )

        for (location in possibleLocations) {
            val bundledBinary = location.toFile()
            LOG.info("Checking for binary at: ${bundledBinary.absolutePath}")
            if (bundledBinary.exists()) {
                // Ensure executable permissions on Unix-like systems
                if (!SystemInfo.isWindows) {
                    bundledBinary.setExecutable(true)
                }
                LOG.info("Found bundled binary at: ${bundledBinary.absolutePath}")
                return bundledBinary.absolutePath
            }
        }

        LOG.error("Bundled binary '$binaryName' not found in any of the expected locations: ${possibleLocations.map { it.toFile().absolutePath }}")
        return null
    }

    companion object {
        fun getInstance(project: Project): PytestLanguageServerService {
            return project.getService(PytestLanguageServerService::class.java)
        }
    }
}
