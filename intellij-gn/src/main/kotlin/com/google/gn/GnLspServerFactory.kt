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

package com.google.gn

import com.intellij.ide.plugins.PluginManagerCore
import com.intellij.openapi.extensions.PluginId
import com.intellij.openapi.project.Project
import com.intellij.openapi.util.SystemInfo
import com.redhat.devtools.lsp4ij.LanguageServerFactory
import com.redhat.devtools.lsp4ij.server.ProcessStreamConnectionProvider
import com.redhat.devtools.lsp4ij.server.StreamConnectionProvider
import java.nio.file.Files
import java.nio.file.Path
import java.nio.file.attribute.PosixFilePermission

class GnLspServerFactory : LanguageServerFactory {
    override fun createConnectionProvider(project: Project): StreamConnectionProvider {
        val binary = getBundledBinaryPath()
        return object :
            ProcessStreamConnectionProvider(listOf(binary.toString()), project.basePath) {}
    }

    private fun getBundledBinaryPath(): Path {
        val plugin =
            PluginManagerCore.getPlugin(PluginId.getId("com.google.gn"))
                ?: throw RuntimeException("Plugin descriptor not found")

        val target =
            when {
                SystemInfo.isLinux && SystemInfo.is64Bit -> "linux-x64"
                SystemInfo.isMac && SystemInfo.isAarch64 -> "darwin-arm64"
                SystemInfo.isWindows && SystemInfo.is64Bit -> "win32-x64"
                else ->
                    throw RuntimeException(
                        "Unsupported platform: ${SystemInfo.OS_NAME} (${SystemInfo.OS_ARCH})"
                    )
            }

        val binaryName =
            if (SystemInfo.isWindows) "gn-language-server.exe" else "gn-language-server"
        val binaryPath = plugin.pluginPath.resolve("bin/$target/$binaryName")

        if (!Files.exists(binaryPath)) {
            throw RuntimeException("Bundled gn-language-server binary not found at: $binaryPath")
        }

        if (!SystemInfo.isWindows) {
            ensureExecutable(binaryPath)
        }
        return binaryPath
    }

    private fun ensureExecutable(path: Path) {
        if (SystemInfo.isWindows) return
        val permissions = Files.getPosixFilePermissions(path).toMutableSet()
        if (permissions.add(PosixFilePermission.OWNER_EXECUTE)) {
            Files.setPosixFilePermissions(path, permissions)
        }
    }
}
