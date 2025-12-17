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

use std::{collections::HashSet, path::Path, sync::Arc};

use itertools::Itertools;
use tower_lsp::lsp_types::{
    Command, CompletionItem, CompletionItemKind, CompletionItemLabelDetails, CompletionParams,
    CompletionResponse, Documentation, MarkupContent, MarkupKind,
};

use crate::{
    analyzer::{AnalyzedFile, Template, Variable, WorkspaceAnalyzer},
    common::{builtins::BUILTINS, error::Result, utils::format_path},
    parser::{Block, Node},
    server::{
        imports::create_import_edit, providers::utils::get_text_document_path, symbols::SymbolSet,
        RequestContext,
    },
};

fn get_prefix_string_for_completion<'i>(parsed_root: &Block<'i>, offset: usize) -> Option<&'i str> {
    parsed_root
        .walk()
        .filter_map(|node| {
            if let Some(string) = node.as_string() {
                if string.span.start() < offset && offset < string.span.end() {
                    return Some(&string.raw_value[0..(offset - string.span.start() - 1)]);
                }
            }
            None
        })
        .next()
}

fn build_filename_completions(path: &Path, prefix: &str) -> Option<Vec<CompletionItem>> {
    let current_dir = path.parent()?;
    let components: Vec<&str> = prefix.split(std::path::MAIN_SEPARATOR).collect();
    let (basename_prefix, subdirs) = components.split_last().unwrap();
    let complete_dir = current_dir.join(subdirs.join(std::path::MAIN_SEPARATOR_STR));
    Some(
        std::fs::read_dir(&complete_dir)
            .ok()?
            .filter_map(|entry| {
                let entry = entry.ok()?;
                let basename = entry.file_name().to_str()?.to_string();
                basename.strip_prefix(basename_prefix)?;
                let is_dir = entry.file_type().ok()?.is_dir();
                let type_suffix = if is_dir {
                    std::path::MAIN_SEPARATOR_STR
                } else {
                    ""
                };
                Some(CompletionItem {
                    label: format!("{basename}{type_suffix}"),
                    kind: Some(CompletionItemKind::FILE),
                    command: is_dir.then_some(Command {
                        command: "editor.action.triggerSuggest".to_string(),
                        ..Default::default()
                    }),
                    ..Default::default()
                })
            })
            .sorted_by_key(|item| item.label.clone())
            .collect(),
    )
}

fn is_after_dot(data: &str, offset: usize) -> bool {
    for ch in data[..offset].chars().rev() {
        match ch {
            '.' => return true,
            'A'..='Z' | 'a'..='z' | '0'..='9' | '_' => continue,
            _ => return false,
        }
    }
    false
}

impl Variable<'_> {
    fn as_completion_item(&self, current_file: &AnalyzedFile, need_import: bool) -> CompletionItem {
        let first_assignment = self.assignments.first().unwrap();
        let import_path = format_path(
            &first_assignment.document.path,
            &current_file.workspace_root,
        );
        let additional_text_edits = if need_import {
            Some(vec![create_import_edit(current_file, &import_path)])
        } else {
            None
        };
        let label_details = if first_assignment.document.path == current_file.document.path {
            None
        } else {
            Some(CompletionItemLabelDetails {
                detail: None,
                description: Some(import_path),
            })
        };
        CompletionItem {
            label: self.name.to_string(),
            kind: Some(CompletionItemKind::VARIABLE),
            documentation: Some(Documentation::MarkupContent(MarkupContent {
                kind: MarkupKind::Markdown,
                value: self.format_help(&current_file.workspace_root).join("\n\n"),
            })),
            label_details,
            additional_text_edits,
            ..Default::default()
        }
    }
}

impl Template<'_> {
    fn as_completion_item(&self, current_file: &AnalyzedFile, need_import: bool) -> CompletionItem {
        let import_path = format_path(&self.document.path, &current_file.workspace_root);
        let additional_text_edits = if need_import {
            Some(vec![create_import_edit(current_file, &import_path)])
        } else {
            None
        };
        let label_details = if self.document.path == current_file.document.path {
            None
        } else {
            Some(CompletionItemLabelDetails {
                detail: None,
                description: Some(import_path),
            })
        };
        CompletionItem {
            label: self.name.to_string(),
            kind: Some(CompletionItemKind::FUNCTION),
            documentation: Some(Documentation::MarkupContent(MarkupContent {
                kind: MarkupKind::Markdown,
                value: self.format_help(&current_file.workspace_root).join("\n\n"),
            })),
            label_details,
            additional_text_edits,
            ..Default::default()
        }
    }
}

async fn build_identifier_completions(
    context: &RequestContext,
    current_file: &Arc<AnalyzedFile>,
    workspace: &WorkspaceAnalyzer,
    offset: usize,
    workspace_completion: bool,
) -> Result<Vec<CompletionItem>> {
    // Handle identifier completions.
    // If the cursor is after a dot, we can't make suggestions.
    if is_after_dot(&current_file.document.data, offset) {
        return Ok(Vec::new());
    }

    let environment = workspace.analyze_at(current_file, offset, context.request_time);
    let symbols = SymbolSet::workspace(workspace).await;

    let builtin_variables: HashSet<&str> = BUILTINS
        .predefined_variables
        .iter()
        .chain(BUILTINS.target_variables.iter())
        .map(|symbol| symbol.name)
        .collect();

    // Enumerate variables/templates already in the scope.
    let known_variables: HashSet<&str> = environment.get().variables.keys().copied().collect();
    let known_templates: HashSet<&str> = environment.get().templates.keys().copied().collect();

    // Enumerate local variables/templates.
    let local_variable_items = environment
        .get()
        .variables
        .values()
        .filter(|variable| !builtin_variables.contains(variable.name))
        .map(|variable| variable.as_completion_item(current_file, false));
    let local_template_items = environment
        .get()
        .templates
        .values()
        .map(|template| template.as_completion_item(current_file, false));

    // Enumerate workspace variables/templates.
    let workspace_items: Vec<_> = if workspace_completion {
        let workspace_variable_items = symbols
            .variables()
            .filter(|variable| !builtin_variables.contains(variable.name))
            .filter(|variable| !known_variables.contains(variable.name))
            .map(|variable| variable.as_completion_item(current_file, true));
        let workspace_template_items = symbols
            .templates()
            .filter(|template| !known_templates.contains(template.name))
            .map(|template| template.as_completion_item(current_file, true));
        workspace_variable_items
            .chain(workspace_template_items)
            .collect()
    } else {
        Vec::new()
    };

    // Enumerate builtins.
    let builtin_function_items = BUILTINS
        .functions
        .iter()
        .chain(BUILTINS.targets.iter())
        .map(|symbol| CompletionItem {
            label: symbol.name.to_string(),
            kind: Some(CompletionItemKind::FUNCTION),
            documentation: Some(Documentation::MarkupContent(MarkupContent {
                kind: MarkupKind::Markdown,
                value: symbol.doc.to_string(),
            })),
            ..Default::default()
        });

    let builtin_variable_items = BUILTINS
        .predefined_variables
        .iter()
        .chain(BUILTINS.target_variables.iter())
        .map(|symbol| CompletionItem {
            label: symbol.name.to_string(),
            kind: Some(CompletionItemKind::VARIABLE),
            documentation: Some(Documentation::MarkupContent(MarkupContent {
                kind: MarkupKind::Markdown,
                value: symbol.doc.to_string(),
            })),
            ..Default::default()
        });

    // Keywords.
    let keyword_items = ["true", "false", "if", "else"].map(|name| CompletionItem {
        label: name.to_string(),
        kind: Some(CompletionItemKind::KEYWORD),
        ..Default::default()
    });

    Ok(keyword_items
        .into_iter()
        .chain(builtin_function_items)
        .chain(builtin_variable_items)
        .chain(local_variable_items)
        .chain(local_template_items)
        .chain(workspace_items)
        .collect())
}

pub async fn completion(
    context: &RequestContext,
    params: CompletionParams,
) -> Result<Option<CompletionResponse>> {
    let config = context.client.configurations().await;
    let path = get_text_document_path(&params.text_document_position.text_document)?;
    let workspace = context.analyzer.workspace_for(&path)?;
    let current_file = workspace.analyze_file(&path, context.request_time);

    let offset = current_file
        .document
        .line_index
        .offset(params.text_document_position.position)
        .unwrap_or(0);

    // Handle string completions.
    if let Some(prefix) = get_prefix_string_for_completion(current_file.parsed_root.get(), offset) {
        // Target completions are not supported yet.
        if prefix.starts_with('/')
            || prefix.starts_with(':')
            || prefix.starts_with(std::path::MAIN_SEPARATOR)
        {
            return Ok(None);
        }
        if let Some(items) = build_filename_completions(&current_file.document.path, prefix) {
            return Ok(Some(CompletionResponse::Array(items)));
        }
        return Ok(None);
    }

    // Handle identifier completions.
    let items = build_identifier_completions(
        context,
        &current_file,
        &workspace,
        offset,
        config.workspace_completion,
    )
    .await?;
    Ok(Some(CompletionResponse::Array(items)))
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use tower_lsp::lsp_types::{Position, TextDocumentIdentifier, TextDocumentPositionParams, Url};

    use crate::common::testutils::testdata;

    use super::*;

    #[tokio::test]
    async fn test_smoke() {
        let response = completion(
            &RequestContext::new_for_testing(Some(&testdata("workspaces/completion"))),
            CompletionParams {
                text_document_position: TextDocumentPositionParams {
                    text_document: TextDocumentIdentifier {
                        uri: Url::from_file_path(testdata("workspaces/completion/BUILD.gn"))
                            .unwrap(),
                    },
                    position: Position::new(36, 0),
                },
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
                context: Default::default(),
            },
        )
        .await
        .unwrap()
        .unwrap();

        let CompletionResponse::Array(items) = response else {
            panic!();
        };

        // Don't return duplicates.
        let duplicates: Vec<_> = items
            .iter()
            .filter(|item| item.label != "cflags" && item.label != "pool")
            .map(|item| item.label.as_str())
            .duplicates()
            .collect();
        assert!(
            duplicates.is_empty(),
            "Duplicates in completion items: {}",
            duplicates.iter().sorted().join(", ")
        );

        // Check items.
        let names: HashSet<_> = items.iter().map(|item| item.label.as_str()).collect();

        let expectation = [
            ("config_variable", true),
            ("_config_variable", false),
            ("config_template", true),
            ("_config_template", false),
            ("import_variable", true),
            ("_import_variable", false),
            ("import_template", true),
            ("_import_template", false),
            ("indirect_variable", true),
            ("_indirect_variable", false),
            ("indirect_template", true),
            ("_indirect_template", false),
            ("outer_variable", true),
            ("_outer_variable", true),
            ("outer_template", true),
            ("_outer_template", true),
            ("inner_variable", true),
            ("_inner_variable", true),
            ("inner_template", true),
            ("_inner_template", true),
            ("child_variable", false),
            ("_child_variable", false),
            ("child_template", false),
            ("_child_template", false),
        ];

        for (name, want) in expectation {
            let got = names.contains(name);
            assert_eq!(got, want, "{name}: got {got}, want {want}");
        }
    }
}
