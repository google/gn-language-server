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

use std::path::{Path, PathBuf};

use either::Either;
use itertools::Itertools;
use tower_lsp::lsp_types::{TextDocumentIdentifier, Url};

use crate::{
    analyzer::{AnalyzedFile, Target, Template, Variable},
    common::{
        builtins::{FOREACH, FORWARD_VARIABLES_FROM},
        error::{Error, Result},
    },
    parser::{Identifier, Node},
};

pub fn get_text_document_path(text_document: &TextDocumentIdentifier) -> Result<PathBuf> {
    text_document
        .uri
        .to_file_path()
        .map_err(|_| Error::General(format!("invalid file URI: {}", text_document.uri)))
}

pub fn lookup_identifier_at(file: &AnalyzedFile, pos: usize) -> Option<&Identifier<'_>> {
    file.ast
        .identifiers()
        .find(|ident| ident.span.start() <= pos && pos <= ident.span.end())
}

pub fn lookup_target_name_string_at(file: &AnalyzedFile, pos: usize) -> Option<Target<'_, '_>> {
    file.analyzed_root.targets().find(|target| {
        target.call.args[0].span().start() <= pos && pos <= target.call.args[0].span().end()
    })
}

pub fn find_target<'a>(file: &'a AnalyzedFile, name: &str) -> Option<&'a Target<'static, 'static>> {
    let targets: Vec<_> = file
        .exports
        .targets
        .values()
        .sorted_by_key(|target| (&target.document.path, target.call.span.start()))
        .collect();

    // Try target name prefixes.
    for name in (1..=name.len()).rev().map(|len| &name[..len]) {
        if let Some(target) = targets.iter().find(|t| t.name == name) {
            return Some(target);
        }
    }

    None
}

pub fn format_path(path: &Path, workspace_root: &Path) -> String {
    if let Ok(relative_path) = path.strip_prefix(workspace_root) {
        format!("//{}", relative_path.to_string_lossy())
    } else {
        path.to_string_lossy().to_string()
    }
}

pub fn format_variable_help(variable: &Variable, workspace_root: &Path) -> Vec<String> {
    let first_assignment = variable
        .assignments
        .iter()
        .sorted_by_key(|a| {
            let span = match &a.assignment_or_call {
                either::Either::Left(assignment) => assignment.span,
                either::Either::Right(call) => call.span,
            };
            (&a.document.path, span.start())
        })
        .next()
        .unwrap();
    let single_assignment = variable.assignments.len() == 1;

    let snippet = if single_assignment {
        match first_assignment.assignment_or_call {
            Either::Left(assignment) => {
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
            Either::Right(call) => {
                match call.function.name {
                    FORWARD_VARIABLES_FROM => call.span.as_str().to_string(),
                    // TODO: Include the entire foreach call (without block)
                    FOREACH => call.args[0].span().as_str().to_string(),
                    _ => panic!("Unexpected assignment: {}", call.function.name),
                }
            }
        }
    } else {
        format!("{} = ...", first_assignment.primary_variable.as_str())
    };

    let mut paragraphs = vec![format!("```gn\n{snippet}\n```")];

    if single_assignment {
        paragraphs.push(format!(
            "```text\n{}\n```",
            first_assignment.comments.to_string().trim()
        ));
    }

    let span = match &first_assignment.assignment_or_call {
        Either::Left(assignment) => assignment.span,
        Either::Right(call) => call.span,
    };
    let position = first_assignment.document.line_index.position(span.start());
    paragraphs.push(if single_assignment {
        format!(
            "Defined at [{}:{}:{}]({}#L{},{})",
            format_path(&first_assignment.document.path, workspace_root),
            position.line + 1,
            position.character + 1,
            Url::from_file_path(&first_assignment.document.path).unwrap(),
            position.line + 1,
            position.character + 1,
        )
    } else {
        format!(
            "Defined and modified in {} locations",
            variable.assignments.len()
        )
    });

    paragraphs
}

pub fn format_template_help(template: &Template, workspace_root: &Path) -> Vec<String> {
    let mut paragraphs = vec![format!(
        "```gn\ntemplate(\"{}\") {{ ... }}\n```",
        template.name
    )];
    if !template.comments.is_empty() {
        paragraphs.push(format!(
            "```text\n{}\n```",
            template.comments.to_string().trim()
        ));
    };
    let position = template
        .document
        .line_index
        .position(template.call.function.span.start());
    paragraphs.push(format!(
        "Defined at [{}:{}:{}]({}#L{},{})",
        format_path(&template.document.path, workspace_root),
        position.line + 1,
        position.character + 1,
        Url::from_file_path(&template.document.path).unwrap(),
        position.line + 1,
        position.character + 1,
    ));

    paragraphs
}
