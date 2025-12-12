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

use either::Either;
use tower_lsp::lsp_types::{
    GotoDefinitionParams, GotoDefinitionResponse, Location, LocationLink, Position, Range, Url,
};

use crate::{
    analyzer::AnalyzedLink,
    common::error::Result,
    parser::Node,
    server::{
        providers::utils::{
            find_target, get_text_document_path, lookup_identifier_at, lookup_target_name_string_at,
        },
        RequestContext,
    },
};

pub async fn goto_definition(
    context: &RequestContext,
    params: GotoDefinitionParams,
) -> Result<Option<GotoDefinitionResponse>> {
    let path = get_text_document_path(&params.text_document_position_params.text_document)?;
    let current_file = context
        .analyzers
        .analyze_file(&path, context.request_time)?;

    let Some(pos) = current_file
        .document
        .line_index
        .offset(params.text_document_position_params.position)
    else {
        return Ok(None);
    };

    // Check links first.
    if let Some(link) = current_file
        .links_map
        .values()
        .flatten()
        .find(|link| link.span().start() <= pos && pos <= link.span().end())
    {
        let (path, position) = match link {
            AnalyzedLink::File { path, .. } => (path, Position::default()),
            AnalyzedLink::Target { path, name, .. } => {
                let target_file = context.analyzers.analyze_file(path, context.request_time)?;
                (
                    path,
                    find_target(&target_file, name)
                        .map(|target| {
                            target
                                .document
                                .line_index
                                .position(target.call.span.start())
                        })
                        .unwrap_or_default(),
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

    // Check template target names.
    if let Some(target) = lookup_target_name_string_at(&current_file, pos) {
        // Return the self link to fall back to finding references.
        return Ok(Some(GotoDefinitionResponse::Scalar(Location {
            uri: params.text_document_position_params.text_document.uri,
            range: current_file
                .document
                .line_index
                .range(target.call.args[0].span()),
        })));
    }

    let Some(ident) = lookup_identifier_at(&current_file, pos) else {
        return Ok(None);
    };

    let environment =
        context
            .analyzers
            .analyze_environment(&current_file, pos, context.request_time)?;

    let mut links: Vec<LocationLink> = Vec::new();

    // Check templates.
    links.extend(environment.templates.get(ident.name).map(|template| {
        LocationLink {
            origin_selection_range: Some(current_file.document.line_index.range(ident.span)),
            target_uri: Url::from_file_path(&template.document.path).unwrap(),
            target_range: template.document.line_index.range(template.call.span),
            target_selection_range: template
                .document
                .line_index
                .range(template.call.function.span),
        }
    }));

    // Check variables.
    if let Some(variable) = environment.variables.get(ident.name) {
        links.extend(variable.assignments.values().map(|assignment| {
            let span = match &assignment.assignment_or_call {
                Either::Left(assignment) => assignment.span,
                Either::Right(call) => call.span,
            };
            LocationLink {
                origin_selection_range: Some(current_file.document.line_index.range(ident.span)),
                target_uri: Url::from_file_path(&assignment.document.path).unwrap(),
                target_range: assignment.document.line_index.range(span),
                target_selection_range: assignment
                    .document
                    .line_index
                    .range(assignment.primary_variable),
            }
        }))
    }

    Ok(Some(GotoDefinitionResponse::Link(links)))
}
