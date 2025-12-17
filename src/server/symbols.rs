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

use std::sync::Arc;

use crate::analyzer::{AnalyzedFile, Analyzer, Template, Variable, WorkspaceAnalyzer};

pub struct SymbolSet {
    files: Vec<Arc<AnalyzedFile>>,
}

impl SymbolSet {
    pub async fn global(analyzer: &Analyzer) -> Self {
        let workspaces = analyzer.workspaces();

        let mut files: Vec<_> = Vec::new();
        for workspace in workspaces.into_values() {
            files.extend(
                workspace
                    .scan_files()
                    .await
                    .into_iter()
                    .filter(|file| !file.external),
            );
        }

        Self { files }
    }

    pub async fn workspace(workspace: &WorkspaceAnalyzer) -> Self {
        let files: Vec<_> = workspace
            .scan_files()
            .await
            .into_iter()
            .filter(|file| !file.external)
            .collect();
        Self { files }
    }

    pub fn variables(&self) -> impl Iterator<Item = &Variable<'_>> + '_ {
        self.files
            .iter()
            .flat_map(|file| file.exports.get().variables.values())
    }

    pub fn templates(&self) -> impl Iterator<Item = &Template<'_>> + '_ {
        self.files
            .iter()
            .flat_map(|file| file.exports.get().templates.values())
    }
}
