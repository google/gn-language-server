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

use std::path::Path;

use itertools::Itertools;
use tower_lsp::lsp_types::{
    Command, CompletionItem, CompletionItemKind, CompletionParams, CompletionResponse,
    Documentation, MarkupContent, MarkupKind,
};

use crate::{
    ast::{Block, Node, Statement},
    builtins::BUILTINS,
    error::{Error, Result},
    server::RequestContext,
};

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

fn get_prefix_string_for_completion<'i>(ast_root: &Block<'i>, offset: usize) -> Option<&'i str> {
    ast_root
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

pub async fn completion(
    context: &RequestContext,
    params: CompletionParams,
) -> Result<Option<CompletionResponse>> {
    let Ok(path) = params
        .text_document_position
        .text_document
        .uri
        .to_file_path()
    else {
        return Err(Error::General(format!(
            "invalid file URI: {}",
            params.text_document_position.text_document.uri
        )));
    };

    let current_file = context
        .analyzer
        .lock()
        .unwrap()
        .analyze(&path, context.cache_config)?;

    let offset = current_file
        .document
        .line_index
        .offset(params.text_document_position.position)
        .unwrap_or(0);

    // Handle string completions.
    if let Some(prefix) = get_prefix_string_for_completion(&current_file.ast_root, offset) {
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
    // If the cursor is after a dot, we can't make suggestions.
    if is_after_dot(&current_file.document.data, offset) {
        return Ok(None);
    }

    let scope = current_file.scope_at(offset);
    let templates = current_file.templates_at(offset);

    // Enumerate variables at the current scope.
    let variable_items = scope
        .all_variables()
        .into_iter()
        .filter_map(|(name, variable)| {
            let first_assignment = variable
                .assignments
                .iter()
                .sorted_by_key(|a| (&a.document.path, a.statement.span().start()))
                .next()?;
            let single_assignment = variable.assignments.len() == 1;
            let snippet = if single_assignment {
                match first_assignment.statement {
                    Statement::Assignment(assignment) => {
                        let raw_value = assignment.rvalue.span().as_str();
                        let display_value = if raw_value.lines().count() <= 5 {
                            raw_value
                        } else {
                            "..."
                        };
                        format!(
                            "{} {} {}",
                            assignment.lvalue.span().as_str(),
                            assignment.op,
                            display_value
                        )
                    }
                    Statement::Call(call) => {
                        assert_eq!(call.function.name, "forward_variables_from");
                        call.span.as_str().to_string()
                    }
                    _ => unreachable!(),
                }
            } else {
                format!("{} = ...", name)
            };
            Some(CompletionItem {
                label: name.to_string(),
                kind: Some(CompletionItemKind::VARIABLE),
                documentation: Some(Documentation::MarkupContent(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value: format!("```gn\n{}\n```\n", snippet),
                })),
                ..Default::default()
            })
        });

    // Enumerate templates defined at the current position.
    let template_items = templates.iter().map(|template| {
        let doc_header = format!("```gn\ntemplate(\"{}\") {{ ... }}\n```\n", template.name);

        let doc_comments = if template.comments.is_empty() {
            String::new()
        } else {
            format!("```text\n{}\n```\n", template.comments)
        };

        let doc = [doc_header, doc_comments].concat();

        CompletionItem {
            label: template.name.to_string(),
            kind: Some(CompletionItemKind::FUNCTION),
            documentation: Some(Documentation::MarkupContent(MarkupContent {
                kind: MarkupKind::Markdown,
                value: doc,
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

    let items: Vec<CompletionItem> = variable_items
        .chain(template_items)
        .chain(builtin_function_items)
        .chain(builtin_variable_items)
        .chain(keyword_items)
        .collect();
    Ok(Some(CompletionResponse::Array(items)))
}
