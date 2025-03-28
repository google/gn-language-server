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

use std::borrow::Cow;

use tower_lsp::lsp_types::{
    GotoDefinitionParams, GotoDefinitionResponse, Location, LocationLink, Position, Range, Url,
};

use crate::{analyze::Link, ast::Node};

use super::{
    find_target_position, into_rpc_error, lookup_identifier_at, new_rpc_error, ProviderContext,
    RpcResult,
};

pub async fn goto_definition(
    context: &ProviderContext,
    params: GotoDefinitionParams,
) -> RpcResult<Option<GotoDefinitionResponse>> {
    let Ok(path) = params
        .text_document_position_params
        .text_document
        .uri
        .to_file_path()
    else {
        return Err(new_rpc_error(Cow::from(format!(
            "invalid file URI: {}",
            params.text_document_position_params.text_document.uri
        ))));
    };

    let current_file = context
        .analyzer
        .lock()
        .unwrap()
        .analyze(&path)
        .map_err(into_rpc_error)?;

    // Check links first.
    if let Some(offset) = current_file
        .document
        .line_index
        .offset(params.text_document_position_params.position)
    {
        if let Some(link) = current_file
            .links
            .iter()
            .find(|link| link.span().start() <= offset && offset <= link.span().end())
        {
            let (path, position) = match link {
                Link::File { path, .. } => (path, Position::default()),
                Link::Target { path, name, .. } => {
                    let target_file = context
                        .analyzer
                        .lock()
                        .unwrap()
                        .analyze(path)
                        .map_err(into_rpc_error)?;
                    (
                        path,
                        find_target_position(&target_file, name).unwrap_or_default(),
                    )
                }
            };
            return Ok(Some(GotoDefinitionResponse::Scalar(Location {
                uri: Url::from_file_path(path).unwrap(),
                range: Range {
                    start: position,
                    end: position,
                },
            })));
        }
    }

    let Some(ident) =
        lookup_identifier_at(&current_file, params.text_document_position_params.position)
    else {
        return Ok(None);
    };

    let mut links: Vec<LocationLink> = Vec::new();

    // Check templates.
    links.extend(
        current_file
            .templates_at(ident.span.start())
            .into_iter()
            .filter(|template| template.name == ident.name)
            .map(|template| LocationLink {
                origin_selection_range: Some(current_file.document.line_index.range(ident.span)),
                target_uri: Url::from_file_path(&template.document.path).unwrap(),
                target_range: template.document.line_index.range(template.span),
                target_selection_range: template.document.line_index.range(template.header),
            }),
    );

    // Check variables.
    let scope = current_file.scope_at(ident.span.start());
    if let Some(variable) = scope.get(ident.name) {
        links.extend(variable.assignments.iter().map(|assignment| {
            LocationLink {
                origin_selection_range: Some(current_file.document.line_index.range(ident.span)),
                target_uri: Url::from_file_path(&assignment.document.path).unwrap(),
                target_range: assignment
                    .document
                    .line_index
                    .range(assignment.statement.span()),
                target_selection_range: assignment
                    .document
                    .line_index
                    .range(assignment.variable_span),
            }
        }))
    }

    Ok(Some(GotoDefinitionResponse::Link(links)))
}
