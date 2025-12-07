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

use tower_lsp::lsp_types::{Hover, HoverContents, HoverParams, MarkedString};

use crate::{
    common::{builtins::BUILTINS, error::Result},
    server::{
        providers::utils::{
            format_template_help, format_variable_help, get_text_document_path,
            lookup_identifier_at,
        },
        RequestContext,
    },
};

pub async fn hover(context: &RequestContext, params: HoverParams) -> Result<Option<Hover>> {
    let path = get_text_document_path(&params.text_document_position_params.text_document)?;
    let current_file = context.analyzer.analyze_file(&path, context.request_time)?;

    let Some(pos) = current_file
        .document
        .line_index
        .offset(params.text_document_position_params.position)
    else {
        return Ok(None);
    };

    let Some(ident) = lookup_identifier_at(&current_file, pos) else {
        return Ok(None);
    };

    let environment =
        context
            .analyzer
            .analyze_environment(&current_file, pos, context.request_time)?;

    let mut sections: Vec<Vec<MarkedString>> = Vec::new();

    // Check templates.
    if let Some(template) = environment.templates.get(ident.name) {
        sections.push(
            format_template_help(template, &current_file.workspace_root)
                .into_iter()
                .map(MarkedString::from_markdown)
                .collect(),
        );
    }

    // Check variables.
    if let Some(variable) = environment.variables.get(ident.name) {
        sections.push(
            format_variable_help(variable, &current_file.workspace_root)
                .into_iter()
                .map(MarkedString::from_markdown)
                .collect(),
        );
    }

    // Check builtin rules.
    if let Some(symbol) = BUILTINS.all().find(|symbol| symbol.name == ident.name) {
        sections.push(vec![MarkedString::from_markdown(symbol.doc.to_string())]);
    }

    if sections.is_empty() {
        return Ok(None);
    }

    let contents = sections.join(&MarkedString::from_markdown("---".to_string()));
    Ok(Some(Hover {
        contents: HoverContents::Array(contents),
        range: Some(current_file.document.line_index.range(ident.span)),
    }))
}

#[cfg(test)]
mod tests {
    use tower_lsp::lsp_types::{
        Position, Range, TextDocumentIdentifier, TextDocumentPositionParams, Url,
        WorkDoneProgressParams,
    };

    use crate::common::testutils::testdata;

    use super::*;

    #[tokio::test]
    async fn test_hover() {
        let uri = Url::from_file_path(testdata("workspaces/hover/BUILD.gn")).unwrap();
        let params = HoverParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri: uri.clone() },
                position: Position {
                    line: 17,
                    character: 0,
                },
            },
            work_done_progress_params: WorkDoneProgressParams::default(),
        };

        let response = hover(&RequestContext::new_for_testing(), params)
            .await
            .unwrap();

        assert_eq!(
            response,
            Some(Hover {
                contents: HoverContents::Array(vec![
                    MarkedString::from_markdown("```gn\na = 1\n```".to_string()),
                    MarkedString::from_markdown("```text\n\n```".to_string()),
                    MarkedString::from_markdown(format!(
                        "Defined at [//BUILD.gn:18:1]({uri}#L18,1)"
                    )),
                ]),
                range: Some(Range {
                    start: Position {
                        line: 17,
                        character: 0,
                    },
                    end: Position {
                        line: 17,
                        character: 1,
                    },
                }),
            })
        );
    }
}
