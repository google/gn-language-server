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

use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity, Url};

use crate::{ast::Node, storage::DocumentVersion};

use super::ProviderContext;

pub async fn publish_diagnostics(context: &ProviderContext, uri: &Url) {
    let Ok(path) = uri.to_file_path() else {
        return;
    };

    let config = context.client.configurations().await;
    if !config.experimental.error_reporting {
        return;
    }

    let Ok(current_file) = context.analyzer.lock().unwrap().analyze(&path) else {
        return;
    };

    let version = if let DocumentVersion::InMemory { revision } = current_file.document.version {
        Some(revision)
    } else {
        None
    };

    let diags = current_file
        .ast_root
        .errors()
        .map(|error| Diagnostic {
            range: current_file.document.line_index.range(error.span()),
            severity: Some(DiagnosticSeverity::ERROR),
            message: "Syntax error".to_string(),
            ..Default::default()
        })
        .collect();

    context
        .client
        .publish_diagnostics(uri.clone(), diags, version)
        .await;
}

pub async fn unpublish_diagnostics(context: &ProviderContext, uri: &Url) {
    context
        .client
        .publish_diagnostics(uri.clone(), Vec::new(), None)
        .await;
}
