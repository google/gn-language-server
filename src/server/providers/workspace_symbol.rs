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
use tower_lsp::lsp_types::{Location, SymbolInformation, SymbolKind, Url, WorkspaceSymbolParams};

use crate::{
    analyzer::{Template, Variable},
    common::error::Result,
    server::{symbols::SymbolSet, RequestContext},
};

impl Variable<'_> {
    #[allow(deprecated)]
    fn as_symbol_information(&self) -> SymbolInformation {
        let first_assignment = self.assignments.first().unwrap();
        SymbolInformation {
            name: self.name.to_string(),
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

pub async fn workspace_symbol(
    context: &RequestContext,
    params: WorkspaceSymbolParams,
) -> Result<Option<Vec<SymbolInformation>>> {
    let symbols = SymbolSet::global(&context.analyzer).await;

    let query = params.query.to_ascii_lowercase();

    let variable_symbols = symbols
        .variables()
        .filter(|variable| variable.name.to_ascii_lowercase().starts_with(&query))
        .map(|variable| variable.as_symbol_information());
    let template_symbols = symbols
        .templates()
        .filter(|template| template.name.to_ascii_lowercase().starts_with(&query))
        .map(|template| template.as_symbol_information());

    Ok(Some(variable_symbols.chain(template_symbols).collect()))
}
