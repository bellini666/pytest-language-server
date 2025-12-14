plugins {
    id("org.jetbrains.kotlin.jvm") version "2.2.21"
    id("org.jetbrains.intellij.platform") version "2.10.5"
}

group = "com.github.bellini666"
version = "0.16.0"

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
    buildSearchableOptions = false
    instrumentCode = false

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
            apiVersion.set(org.jetbrains.kotlin.gradle.dsl.KotlinVersion.KOTLIN_2_0)
            languageVersion.set(org.jetbrains.kotlin.gradle.dsl.KotlinVersion.KOTLIN_2_0)
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
}
