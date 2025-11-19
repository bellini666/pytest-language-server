plugins {
    id("org.jetbrains.kotlin.jvm") version "1.9.22"
    id("org.jetbrains.intellij") version "1.17.2"
}

group = "com.github.bellini666"
version = "0.7.2"

repositories {
    mavenCentral()
}

intellij {
    version.set("2023.3")
    type.set("PC") // PyCharm Community
    plugins.set(listOf("PythonCore"))

    // Download sources and javadocs for development
    downloadSources.set(true)
}

kotlin {
    jvmToolchain(17)
}

tasks {
    // Set the JVM compatibility versions
    withType<JavaCompile> {
        sourceCompatibility = "17"
        targetCompatibility = "17"
    }
    withType<org.jetbrains.kotlin.gradle.tasks.KotlinCompile> {
        kotlinOptions {
            jvmTarget = "17"
            apiVersion = "1.8"
            languageVersion = "1.8"
        }
    }

    patchPluginXml {
        // Support from PyCharm 2023.3 (build 233) onwards
        sinceBuild.set("233")
        // Leave empty for forward compatibility with all future versions
        untilBuild.set("")
    }

    // Ensure binaries are included in the plugin distribution
    // Place them in lib/bin relative to plugin root
    prepareSandbox {
        from("src/main/resources/bin") {
            into("${pluginName.get()}/lib/bin")
            fileMode = 0b111101101 // 0755 in octal - executable
        }
    }

    // Also ensure binaries are in the distribution ZIP
    buildPlugin {
        from("src/main/resources/bin") {
            into("lib/bin")
            fileMode = 0b111101101 // 0755 in octal - executable
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
}
