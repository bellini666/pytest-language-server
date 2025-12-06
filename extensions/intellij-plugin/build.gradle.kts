plugins {
    id("org.jetbrains.kotlin.jvm") version "2.1.0"
    id("org.jetbrains.intellij.platform") version "2.10.5"
}

group = "com.github.bellini666"
version = "0.13.0"

repositories {
    mavenCentral()
    intellijPlatform {
        defaultRepositories()
    }
}

dependencies {
    intellijPlatform {
        create("PC", "2024.2") // PyCharm Community
        bundledPlugins("PythonCore")

        // Add LSP4IJ dependency from JetBrains Marketplace
        // Use version 0.18.0 which is compatible with PyCharm 2024.2+
        plugin("com.redhat.devtools.lsp4ij", "0.18.0")

        pluginVerifier()
    }
}

kotlin {
    jvmToolchain(21)
}

intellijPlatform {
    pluginConfiguration {
        ideaVersion {
            sinceBuild = "242"
            untilBuild = provider { null } // Support all future versions
        }
    }

    pluginVerification {
        ides {
            recommended()
        }
    }
}

tasks {
    // Kotlin API/language version compatibility
    // Note: jvmTarget is automatically set to 21 by jvmToolchain(21) above
    withType<org.jetbrains.kotlin.gradle.tasks.KotlinCompile> {
        compilerOptions {
            apiVersion.set(org.jetbrains.kotlin.gradle.dsl.KotlinVersion.KOTLIN_1_9)
            languageVersion.set(org.jetbrains.kotlin.gradle.dsl.KotlinVersion.KOTLIN_1_9)
        }
    }

    // Ensure binaries are included in the plugin distribution
    // Place them in lib/bin relative to plugin root
    prepareSandbox {
        from("src/main/resources/bin") {
            into("pytest Language Server/lib/bin")
            filePermissions {
                unix("rwxr-xr-x")
            }
        }
    }

    // Also ensure binaries are in the distribution ZIP
    buildPlugin {
        from("src/main/resources/bin") {
            into("lib/bin")
            filePermissions {
                unix("rwxr-xr-x")
            }
        }
    }

    signPlugin {
        certificateChain.set(System.getenv("CERTIFICATE_CHAIN"))
        privateKey.set(System.getenv("PRIVATE_KEY"))
        password.set(System.getenv("PRIVATE_KEY_PASSWORD"))
    }

    publishPlugin {
        token.set(System.getenv("PUBLISH_TOKEN"))
    }

    buildSearchableOptions {
        enabled = false
    }

    // Disable instrumentCode to avoid dependency resolution issues with JetBrains Maven
    // This task performs bytecode instrumentation which is not critical for our simple LSP extension
    instrumentCode {
        enabled = false
    }
}
