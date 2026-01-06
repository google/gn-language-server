// Copyright 2025 Google LLC
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

plugins {
    id("java")
    id("org.jetbrains.kotlin.jvm") version "2.1.20"
    id("org.jetbrains.intellij.platform") version "2.10.2"
    id("com.diffplug.spotless") version "8.1.0"
}

group = "com.google.gn"
val version: String by project

repositories {
    mavenCentral()
    intellijPlatform {
        defaultRepositories()
    }
}

dependencies {
    intellijPlatform {
        intellijIdea("2024.2.4")
        zipSigner()
        testFramework(org.jetbrains.intellij.platform.gradle.TestFrameworkType.Platform)

        bundledPlugin("org.jetbrains.plugins.textmate")
        plugin("com.redhat.devtools.lsp4ij:0.19.1")
    }
}

intellijPlatform {
    pluginConfiguration {
        ideaVersion {
            sinceBuild = "242"
        }

        changeNotes = """
        Initial version
    """.trimIndent()
    }

    signing {
        certificateChain = providers.environmentVariable("JETBRAINS_CERT")
        privateKey = providers.environmentVariable("JETBRAINS_PRIVATE_KEY")
    }

    publishing {
        token = providers.environmentVariable("JETBRAINS_TOKEN")
        channels = providers.gradleProperty("channel").map { listOf(it) }.orElse(emptyList())
    }

    buildSearchableOptions = false
}

tasks {
    publishPlugin {
        providers.gradleProperty("pluginFile").orNull?.let {
            archiveFile = file(it)
        }
    }

    // Set the JVM compatibility versions
    withType<JavaCompile> {
        sourceCompatibility = "21"
        targetCompatibility = "21"
    }

    wrapper {
        gradleVersion = "8.14.3"
    }

    val buildLanguageServer = register<Exec>("buildLanguageServer") {
        commandLine("sh", "-c", "cargo build --release")
        workingDir = file("..")
    }

    prepareSandbox {
        val pluginName = project.name
        val prebuiltsPath = System.getenv("GN_LSP_PREBUILTS")

        if (prebuiltsPath != null) {
            into("$pluginName/bin") {
                from(prebuiltsPath)
            }
        } else {
            dependsOn(buildLanguageServer)
            val os = System.getProperty("os.name").lowercase()
            val arch = System.getProperty("os.arch").lowercase()
            val (platform, ext) = when {
                os.contains("linux") -> "x86_64-unknown-linux-musl" to ""
                os.contains("mac") && arch == "aarch64" -> "aarch64-apple-darwin" to ""
                os.contains("win") -> "x86_64-pc-windows-msvc" to ".exe"
                else -> return@prepareSandbox
            }

            into("$pluginName/bin/$platform") {
                from(file("../target/release/gn-language-server$ext"))
            }
        }
    }
}

kotlin {
    compilerOptions {
        jvmTarget.set(org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_21)
    }
}

spotless {
    kotlin {
        ktfmt("0.60").kotlinlangStyle()
    }
}
