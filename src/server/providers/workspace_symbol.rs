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

use std::collections::HashSet;

use either::Either;
use tower_lsp::lsp_types::{Location, SymbolInformation, SymbolKind, Url, WorkspaceSymbolParams};

use crate::{analyzer::AnalyzedFile, common::error::Result, server::RequestContext};

pub async fn workspace_symbol(
    context: &RequestContext,
    params: WorkspaceSymbolParams,
) -> Result<Option<Vec<SymbolInformation>>> {
    if !context
        .client
        .configurations()
        .await
        .experimental
        .workspace_symbols
    {
        return Ok(None);
    }

    let mut symbols = Vec::new();
    let query = params.query.to_lowercase();
    let workspace_roots = context.analyzers.workspace_roots();

    for workspace_root in workspace_roots {
        let signal = context
            .indexed
            .lock()
            .unwrap()
            .get(&workspace_root)
            .cloned();
        if let Some(signal) = signal {
            signal.wait().await;
        }

        let files = context.analyzers.cached_files(&workspace_root);
        for file in files {
            symbols.extend(extract_symbols(&file, &query));
        }
    }

    // If any occurrence of a variable is within declare_args, exclude other
    // occurrences.
    let args: HashSet<String> = symbols
        .iter()
        .filter_map(|symbol| (symbol.kind == SymbolKind::CONSTANT).then_some(symbol.name.clone()))
        .collect();

    let symbols = symbols
        .into_iter()
        .filter(|symbol| symbol.kind == SymbolKind::CONSTANT || !args.contains(&symbol.name))
        .collect();

    Ok(Some(symbols))
}

#[allow(deprecated)]
fn extract_symbols(file: &AnalyzedFile, query: &str) -> Vec<SymbolInformation> {
    let mut symbols = Vec::new();
    let uri = Url::from_file_path(&file.document.path).unwrap();

    for (name, variable) in file.exports.variables.as_ref() {
        if !name.to_lowercase().contains(query) {
            continue;
        }
        if let Some(assignment) = variable.assignments.values().next() {
            let span = match assignment.assignment_or_call {
                Either::Left(assignment) => assignment.span,
                Either::Right(call) => call.span,
            };
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
                    range: assignment.document.line_index.range(span),
                },
                container_name: None,
            });
        }
    }

    for (name, template) in file.exports.templates.as_ref() {
        if !name.to_lowercase().contains(query) {
            continue;
        }
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
