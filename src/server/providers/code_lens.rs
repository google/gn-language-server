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

use std::path::PathBuf;

use serde_json::Value;
use tower_lsp::lsp_types::{CodeLens, CodeLensParams, Command, Position};

use crate::{
    common::error::Result,
    server::{
        providers::{
            references::target_references,
            utils::{format_path, get_text_document_path},
        },
        RequestContext,
    },
};

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(tag = "type")]
enum CodeLensData {
    TargetReferences(CodeLensDataTargetReferences),
}

#[derive(serde::Serialize, serde::Deserialize)]
struct CodeLensDataTargetReferences {
    pub path: PathBuf,
    pub position: Position,
    pub target_name: String,
}

pub async fn code_lens(
    context: &RequestContext,
    params: CodeLensParams,
) -> Result<Option<Vec<CodeLens>>> {
    if !context
        .client
        .configurations()
        .await
        .experimental
        .target_lens
    {
        return Ok(None);
    }

    let path = get_text_document_path(&params.text_document)?;
    let current_file = context.analyzer.analyze_file(&path, context.request_time)?;

    Ok(Some(
        current_file
            .analyzed_root
            .targets()
            .flat_map(|target| {
                let range = current_file.document.line_index.range(target.call.span);
                let label = format!(
                    "{}:{}",
                    format_path(
                        current_file.document.path.parent().unwrap(),
                        &current_file.workspace_root
                    ),
                    target.name
                );
                let position = current_file
                    .document
                    .line_index
                    .position(target.call.span.start());
                [
                    CodeLens {
                        range,
                        command: None,
                        data: Some(
                            serde_json::to_value(CodeLensData::TargetReferences(
                                CodeLensDataTargetReferences {
                                    path: path.clone(),
                                    position,
                                    target_name: target.name.to_string(),
                                },
                            ))
                            .unwrap(),
                        ),
                    },
                    CodeLens {
                        range,
                        command: Some(Command {
                            title: "Copy".to_string(),
                            command: "gn.copyTargetLabel".to_string(),
                            arguments: Some(vec![Value::String(label)]),
                        }),
                        data: None,
                    },
                ]
            })
            .collect(),
    ))
}

pub async fn code_lens_resolve(
    context: &RequestContext,
    partial_lens: CodeLens,
) -> Result<CodeLens> {
    let data = serde_json::from_value::<CodeLensData>(partial_lens.data.unwrap())?;
    match data {
        CodeLensData::TargetReferences(CodeLensDataTargetReferences {
            path,
            position,
            target_name,
        }) => {
            let current_file = context.analyzer.analyze_file(&path, context.request_time)?;
            let references = target_references(context, &current_file, &target_name)
                .await?
                .unwrap_or_default();
            let title = match references.len() {
                0 => "No references".to_string(),
                1 => "1 reference".to_string(),
                n => format!("{n} references"),
            };
            Ok(CodeLens {
                range: partial_lens.range,
                command: Some(Command {
                    command: "gn.showTargetReferences".to_string(),
                    title,
                    arguments: Some(vec![
                        serde_json::to_value(position).unwrap(),
                        serde_json::to_value(references).unwrap(),
                    ]),
                }),
                data: None,
            })
        }
    }
}
