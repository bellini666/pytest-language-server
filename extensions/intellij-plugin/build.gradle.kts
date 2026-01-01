plugins {
    id("org.jetbrains.kotlin.jvm") version "2.3.0"
    id("org.jetbrains.intellij.platform") version "2.10.5"
}

group = "com.github.bellini666"
version = "0.17.2"

repositories {
    mavenCentral()
    intellijPlatform {
        defaultRepositories()
    }
}

dependencies {
    intellijPlatform {
        // Target PyCharm Professional for native LSP support (available since 2023.2)
        // PY = PyCharm Professional (includes Python plugin and LSP support)
        create("PY", "2023.2")

        // No LSP4IJ dependency - using native IntelliJ LSP API

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
            sinceBuild = "232"  // IntelliJ/PyCharm 2023.2+
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
