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

use std::{path::Path, pin::Pin, sync::Arc};

use itertools::Itertools;
use tower_lsp::lsp_types::{
    Command, CompletionItem, CompletionItemKind, CompletionParams, CompletionResponse,
    Documentation, MarkupContent, MarkupKind,
};

use crate::{
    analyzer::AnalyzedFile,
    common::{builtins::BUILTINS, error::Result},
    parser::{Block, Node},
    server::{
        providers::utils::{format_template_help, format_variable_help, get_text_document_path},
        RequestContext,
    },
};

fn get_prefix_string_for_completion<'i>(ast: &Block<'i>, offset: usize) -> Option<&'i str> {
    ast.walk()
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

fn build_identifier_completions(
    context: &RequestContext,
    current_file: &Pin<Arc<AnalyzedFile>>,
    offset: usize,
) -> Result<Vec<CompletionItem>> {
    // Handle identifier completions.
    // If the cursor is after a dot, we can't make suggestions.
    if is_after_dot(&current_file.document.data, offset) {
        return Ok(Vec::new());
    }

    let environment =
        context
            .analyzer
            .analyze_environment(current_file, offset, context.request_time)?;

    // Enumerate variables at the current scope.
    let variable_items = environment.variables.iter().map(|(name, variable)| {
        let paragraphs = format_variable_help(variable, &current_file.workspace_root);
        CompletionItem {
            label: name.to_string(),
            kind: Some(CompletionItemKind::VARIABLE),
            documentation: Some(Documentation::MarkupContent(MarkupContent {
                kind: MarkupKind::Markdown,
                value: paragraphs.join("\n\n"),
            })),
            ..Default::default()
        }
    });

    // Enumerate templates defined at the current position.
    let template_items = environment.templates.values().map(|template| {
        let paragraphs = format_template_help(template, &current_file.workspace_root);
        CompletionItem {
            label: template.name.to_string(),
            kind: Some(CompletionItemKind::FUNCTION),
            documentation: Some(Documentation::MarkupContent(MarkupContent {
                kind: MarkupKind::Markdown,
                value: paragraphs.join("\n\n"),
            })),
            ..Default::default()
        }
    });

    // Enumerate buildins.
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

    Ok(variable_items
        .chain(template_items)
        .chain(builtin_function_items)
        .chain(builtin_variable_items)
        .chain(keyword_items)
        .collect())
}

pub async fn completion(
    context: &RequestContext,
    params: CompletionParams,
) -> Result<Option<CompletionResponse>> {
    let path = get_text_document_path(&params.text_document_position.text_document)?;
    let current_file = context.analyzer.analyze_file(&path, context.request_time)?;

    let offset = current_file
        .document
        .line_index
        .offset(params.text_document_position.position)
        .unwrap_or(0);

    // Handle string completions.
    if let Some(prefix) = get_prefix_string_for_completion(&current_file.ast, offset) {
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
    let items = build_identifier_completions(context, &current_file, offset)?;
    Ok(Some(CompletionResponse::Array(items)))
}
