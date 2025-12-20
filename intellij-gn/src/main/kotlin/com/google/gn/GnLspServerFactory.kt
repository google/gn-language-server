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

class GnLspServerFactory : LanguageServerFactory {
  override fun createConnectionProvider(project: Project): StreamConnectionProvider {
    val plugin = PluginManagerCore.getPlugin(PluginId.getId("com.google.gn"))
    val binaryName = if (SystemInfo.isWindows) "gn-language-server.exe" else "gn-language-server"
    val bundledBinary = plugin?.pluginPath?.resolve("bin")?.resolve(binaryName)
    val command = if (bundledBinary != null && Files.exists(bundledBinary)) {
      listOf(bundledBinary.toAbsolutePath().toString())
    } else {
      listOf("gn-language-server")
    }

    return object : ProcessStreamConnectionProvider(
      command,
      project.basePath
    ) {}
  }
}
