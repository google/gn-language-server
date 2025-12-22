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

import com.intellij.openapi.application.PathManager
import java.nio.file.Files
import java.nio.file.Path
import org.jetbrains.plugins.textmate.api.TextMateBundleProvider

class GnTextMateBundleProvider : TextMateBundleProvider {
    override fun getBundles(): List<TextMateBundleProvider.PluginBundle> {
        val tempDir = Files.createTempDirectory(Path.of(PathManager.getTempPath()), "gn-textmate")

        for (name in listOf("package.json", "syntaxes/gn.tmLanguage.json")) {
            val resource = javaClass.classLoader.getResource("textmate/$name")
            resource?.openStream()?.use { stream ->
                val destination = tempDir.resolve(name)
                Files.createDirectories(destination.parent)
                Files.copy(stream, destination)
            }
        }

        return listOf(TextMateBundleProvider.PluginBundle("gn", tempDir))
    }
}
