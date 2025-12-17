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

use std::{collections::HashMap, path::Path};

use itertools::Itertools;
use tower_lsp::lsp_types::{
    CodeAction, CodeActionKind, CodeActionOrCommand, CodeActionParams, CodeActionResponse, Command,
    Diagnostic, NumberOrString, Url, WorkspaceEdit,
};

use crate::{
    common::{error::Result, utils::format_path},
    diagnostics::{DiagnosticDataUndefined, DIAGNOSTIC_CODE_UNDEFINED},
    server::{
        imports::create_import_edit, providers::utils::get_text_document_path, symbols::SymbolSet,
        RequestContext,
    },
};

#[derive(serde::Serialize, serde::Deserialize)]
struct ChooseImportCandidatesData {
    pub candidates: Vec<ImportCandidate>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct ImportCandidate {
    pub import: String,
    pub edit: WorkspaceEdit,
}

async fn compute_import_actions(
    context: &RequestContext,
    path: &Path,
    name: &str,
    diagnostic: &Diagnostic,
) -> Vec<CodeActionOrCommand> {
    let Ok(workspace) = context.analyzer.workspace_for(path) else {
        return Vec::new();
    };
    let current_file = workspace.analyze_file(path, context.request_time);
    let symbols = SymbolSet::workspace(&workspace).await;

    let imports: Vec<String> = symbols
        .variables()
        .filter(|variable| variable.name == name)
        .map(|variable| {
            format_path(
                &variable.assignments.first().unwrap().document.path,
                &workspace.context().root,
            )
        })
        .sorted()
        .collect();

    if imports.is_empty() {
        return Vec::new();
    }
    if let Ok(only_import) = imports.iter().exactly_one() {
        return vec![CodeActionOrCommand::CodeAction(CodeAction {
            title: format!("Import `{name}` from `{only_import}`"),
            kind: Some(CodeActionKind::QUICKFIX),
            diagnostics: Some(vec![diagnostic.clone()]),
            edit: Some(WorkspaceEdit {
                changes: Some(HashMap::from([(
                    Url::from_file_path(&current_file.document.path).unwrap(),
                    vec![create_import_edit(&current_file, only_import)],
                )])),
                ..Default::default()
            }),
            command: None,
            is_preferred: Some(true),
            ..Default::default()
        })];
    }

    let data = ChooseImportCandidatesData {
        candidates: imports
            .iter()
            .map(|import| ImportCandidate {
                import: import.clone(),
                edit: WorkspaceEdit {
                    changes: Some(HashMap::from([(
                        Url::from_file_path(&current_file.document.path).unwrap(),
                        vec![create_import_edit(&current_file, import)],
                    )])),
                    ..Default::default()
                },
            })
            .collect(),
    };

    vec![CodeActionOrCommand::CodeAction(CodeAction {
        title: format!("Import `{}`", name),
        kind: Some(CodeActionKind::QUICKFIX),
        diagnostics: Some(vec![diagnostic.clone()]),
        edit: None,
        command: Some(Command {
            title: format!("Import `{}` from ...", name),
            command: "gn.chooseImportCandidates".to_string(),
            arguments: Some(vec![serde_json::to_value(data).unwrap()]),
        }),
        is_preferred: Some(true),
        ..Default::default()
    })]
}

pub async fn code_action(
    context: &RequestContext,
    params: CodeActionParams,
) -> Result<Option<CodeActionResponse>> {
    let mut actions: Vec<CodeActionOrCommand> = Vec::new();

    let path = get_text_document_path(&params.text_document)?;

    for diagnostic in &params.context.diagnostics {
        match &diagnostic.code {
            Some(NumberOrString::String(code)) if code == DIAGNOSTIC_CODE_UNDEFINED => {
                let Some(data) = &diagnostic.data else {
                    continue;
                };
                let Ok(data) = serde_json::from_value::<DiagnosticDataUndefined>(data.clone())
                else {
                    continue;
                };
                actions
                    .extend(compute_import_actions(context, &path, &data.name, diagnostic).await);
            }
            _ => {}
        }
    }

    Ok(Some(actions))
}
