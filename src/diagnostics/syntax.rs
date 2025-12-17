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

use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity};

use crate::{
    common::storage::Document,
    parser::{Block, Node},
};

pub fn collect_syntax_errors(parsed_root: &Block, document: &Document) -> Vec<Diagnostic> {
    parsed_root
        .errors()
        .map(|error| Diagnostic {
            range: document.line_index.range(error.span()),
            severity: Some(DiagnosticSeverity::ERROR),
            message: error.diagnosis().to_string(),
            ..Default::default()
        })
        .collect()
}
