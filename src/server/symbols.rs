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

use either::Either;
use tower_lsp::lsp_types::{Location, SymbolInformation, SymbolKind, Url};

use crate::analyzer::{AnalyzedFile, Analyzer, Template, Variable, WorkspaceAnalyzer};

impl Variable<'_> {
    #[allow(deprecated)]
    fn as_symbol_information(&self) -> SymbolInformation {
        let first_assignment = self.assignments.first().unwrap();
        SymbolInformation {
            name: first_assignment.primary_variable.as_str().to_string(),
            kind: if self.is_args {
                SymbolKind::CONSTANT
            } else {
                SymbolKind::VARIABLE
            },
            tags: None,
            deprecated: None,
            location: Location {
                uri: Url::from_file_path(&first_assignment.document.path).unwrap(),
                range: first_assignment.document.line_index.range(
                    match first_assignment.assignment_or_call {
                        Either::Left(assignment) => assignment.span,
                        Either::Right(call) => call.span,
                    },
                ),
            },
            container_name: None,
        }
    }
}

impl Template<'_> {
    #[allow(deprecated)]
    fn as_symbol_information(&self) -> SymbolInformation {
        SymbolInformation {
            name: self.name.to_string(),
            kind: SymbolKind::FUNCTION,
            tags: None,
            deprecated: None,
            location: Location {
                uri: Url::from_file_path(&self.document.path).unwrap(),
                range: self.document.line_index.range(self.call.span),
            },
            container_name: None,
        }
    }
}

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

    pub fn symbol_informations(&self) -> impl Iterator<Item = SymbolInformation> + '_ {
        self.files.iter().flat_map(|file| {
            let variables = file
                .exports
                .get()
                .variables
                .values()
                .map(|variable| variable.as_symbol_information());
            let templates = file
                .exports
                .get()
                .templates
                .values()
                .map(|template| template.as_symbol_information());
            variables.chain(templates)
        })
    }
}
