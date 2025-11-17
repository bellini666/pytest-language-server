plugins {
    id 'java'
    id 'org.jetbrains.kotlin.jvm' version '1.9.0'
    id 'org.jetbrains.intellij' version '1.15.0'
}

group = 'com.github.bellini666'
version = '0.5.1'

repositories {
    mavenCentral()
}

dependencies {
    implementation 'org.jetbrains.kotlin:kotlin-stdlib'
}

intellij {
    version = '2023.1'
    type = 'IC' // IntelliJ IDEA Community Edition
    plugins = ['com.intellij.java', 'PythonCore']
}

tasks {
    patchPluginXml {
        sinceBuild = '231'
        untilBuild = '241.*'
    }

    signPlugin {
        certificateChain = System.getenv("CERTIFICATE_CHAIN")
        privateKey = System.getenv("PRIVATE_KEY")
        password = System.getenv("PRIVATE_KEY_PASSWORD")
    }

    publishPlugin {
        token = System.getenv("PUBLISH_TOKEN")
    }

    buildPlugin {
        doLast {
            // Copy binaries to build output
            copy {
                from '../target/release'
                into 'build/idea-sandbox/plugins/pytest-language-server/bin'
                include 'pytest-language-server*'
            }
        }
    }
}

kotlin {
    jvmToolchain(17)
}
