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

use either::Either;
use tower_lsp::lsp_types::{Location, SymbolInformation, SymbolKind, Url};

use crate::{
    analyzer::{AnalyzedFile, Analyzer, WorkspaceAnalyzer},
    common::utils::is_good_for_scan,
};

pub async fn collect_global_symbols(analyzer: &Analyzer) -> Vec<SymbolInformation> {
    let workspaces = analyzer.workspaces();

    let mut symbols: Vec<SymbolInformation> = Vec::new();
    for workspace in workspaces.into_values() {
        symbols.extend(collect_workspace_symbols(&workspace).await);
    }
    symbols
}

pub async fn collect_workspace_symbols(workspace: &WorkspaceAnalyzer) -> Vec<SymbolInformation> {
    workspace.indexed().wait().await;
    let files = workspace.cached_files();
    files
        .into_iter()
        .filter(|file| !file.external && is_good_for_scan(&file.document.path))
        .flat_map(|file| extract_symbols(&file))
        .collect()
}

fn extract_symbols(file: &AnalyzedFile) -> Vec<SymbolInformation> {
    let mut symbols = Vec::new();
    let uri = Url::from_file_path(&file.document.path).unwrap();

    for (name, variable) in &file.exports.get().variables {
        if let Some(assignment) = variable.assignments.first() {
            #[allow(deprecated)]
            symbols.push(SymbolInformation {
                name: name.to_string(),
                kind: if variable.is_args {
                    SymbolKind::CONSTANT
                } else {
                    SymbolKind::VARIABLE
                },
                tags: None,
                deprecated: None,
                location: Location {
                    uri: uri.clone(),
                    range: assignment.document.line_index.range(
                        match assignment.assignment_or_call {
                            Either::Left(assignment) => assignment.span,
                            Either::Right(call) => call.span,
                        },
                    ),
                },
                container_name: None,
            });
        }
    }

    for (name, template) in &file.exports.get().templates {
        #[allow(deprecated)]
        symbols.push(SymbolInformation {
            name: name.to_string(),
            kind: SymbolKind::FUNCTION,
            tags: None,
            deprecated: None,
            location: Location {
                uri: uri.clone(),
                range: template.document.line_index.range(template.call.span),
            },
            container_name: None,
        });
    }

    symbols
}
