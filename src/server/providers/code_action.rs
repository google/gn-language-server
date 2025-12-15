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
    Diagnostic, NumberOrString, Range, TextEdit, Url, WorkspaceEdit,
};

use crate::{
    analyzer::{AnalyzedFile, AnalyzedStatement},
    common::{error::Result, utils::format_path},
    diagnostics::{DiagnosticDataUndefined, DIAGNOSTIC_CODE_UNDEFINED},
    server::{providers::utils::get_text_document_path, RequestContext},
};

#[derive(serde::Serialize, serde::Deserialize)]
struct ChooseImportCandidatesData {
    pub candidates: Vec<ImportCandidate>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct ImportCandidate {
    pub import: String,
    pub edit: TextEdit,
}

fn get_import<'p>(statement: &AnalyzedStatement<'p>) -> Option<&'p str> {
    match statement {
        AnalyzedStatement::Import(import) => Some(import.name),
        _ => None,
    }
}

fn compute_import_edit(current_file: &AnalyzedFile, import: &str) -> TextEdit {
    // Find the first top-level import block.
    let first_import_block: Vec<_> = current_file
        .analyzed_root
        .get()
        .statements
        .iter()
        .skip_while(|statement| get_import(statement).is_none())
        .take_while(|statement| get_import(statement).is_some())
        .collect();

    let (insert_offset, prefix, suffix) = if first_import_block.is_empty() {
        if let Some(first_statement) = current_file.analyzed_root.get().statements.first() {
            (first_statement.span().start(), "", "\n\n")
        } else {
            (current_file.document.data.len(), "", "\n\n")
        }
    } else if let Some(next_import) = first_import_block
        .iter()
        .copied()
        .find(|statement| get_import(statement).unwrap() > import)
    {
        (next_import.span().start(), "", "\n")
    } else {
        (first_import_block.last().unwrap().span().end(), "\n", "")
    };

    let insert_pos = current_file.document.line_index.position(insert_offset);
    TextEdit {
        range: Range::new(insert_pos, insert_pos),
        new_text: format!("{prefix}import(\"{import}\"){suffix}"),
    }
}

fn compute_import_actions(
    context: &RequestContext,
    path: &Path,
    name: &str,
    diagnostic: &Diagnostic,
) -> Vec<CodeActionOrCommand> {
    let Ok(workspace) = context.analyzer.workspace_for(path) else {
        return Vec::new();
    };
    let current_file = workspace.analyze_file(path, context.request_time);
    let workspace_files = workspace.cached_files_for_symbols();
    let imports: Vec<String> = workspace_files
        .into_iter()
        .filter(|file| file.exports.get().variables.contains_key(name))
        .map(|file| format_path(&file.document.path, &file.workspace_root))
        .sorted()
        .collect();
    if imports.is_empty() {
        return Vec::new();
    }
    if let Ok(only_import) = imports.iter().exactly_one() {
        let edit = WorkspaceEdit {
            changes: Some(HashMap::from([(
                Url::from_file_path(&current_file.document.path).unwrap(),
                vec![compute_import_edit(&current_file, only_import)],
            )])),
            ..Default::default()
        };
        return vec![CodeActionOrCommand::CodeAction(CodeAction {
            title: format!("Import `{name}` from `{only_import}`"),
            kind: Some(CodeActionKind::QUICKFIX),
            diagnostics: Some(vec![diagnostic.clone()]),
            edit: Some(edit),
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
                edit: compute_import_edit(&current_file, import),
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
                actions.extend(compute_import_actions(
                    context, &path, &data.name, diagnostic,
                ));
            }
            _ => {}
        }
    }

    Ok(Some(actions))
}
