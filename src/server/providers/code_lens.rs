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

use std::{
    path::{Path, PathBuf},
    time::Instant,
};

use serde_json::Value;
use tower_lsp::lsp_types::{CodeLens, CodeLensParams, Command, Range};

use crate::{
    analyzer::WorkspaceAnalyzer,
    common::{error::Result, utils::format_path},
    server::{
        providers::{references::target_references, utils::get_text_document_path},
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
    pub target_name: String,
}

async fn compute_references_lens(
    workspace: &WorkspaceAnalyzer,
    path: &Path,
    range: Range,
    target_name: &str,
    request_time: Instant,
) -> Result<CodeLens> {
    let current_file = workspace.analyze_file(path, request_time);
    let references = target_references(workspace, &current_file, target_name).await?;
    let title = match references.len() {
        0 => "No references".to_string(),
        1 => "1 reference".to_string(),
        n => format!("{n} references"),
    };
    Ok(CodeLens {
        range,
        command: Some(Command {
            command: "gn.showTargetReferences".to_string(),
            title,
            arguments: Some(vec![
                serde_json::to_value(range.start).unwrap(),
                serde_json::to_value(references).unwrap(),
            ]),
        }),
        data: None,
    })
}

pub async fn code_lens(
    context: &RequestContext,
    params: CodeLensParams,
) -> Result<Option<Vec<CodeLens>>> {
    let configs = context.client.configurations().await;
    if !configs.experimental.target_lens {
        return Ok(None);
    }

    let path = get_text_document_path(&params.text_document)?;
    let workspace = context.analyzer.workspace_for(&path)?;
    let current_file = workspace.analyze_file(&path, context.request_time);

    let targets: Vec<_> = current_file.analyzed_root.get().targets().collect();

    let mut lens: Vec<CodeLens> = Vec::new();

    if configs.background_indexing {
        if workspace.indexed().done() {
            for target in &targets {
                lens.push(
                    compute_references_lens(
                        &workspace,
                        &path,
                        current_file.document.line_index.range(target.call.span),
                        target.name,
                        context.request_time,
                    )
                    .await?,
                );
            }
        } else {
            lens.extend(targets.iter().map(|target| {
                let range = current_file.document.line_index.range(target.call.span);
                CodeLens {
                    range,
                    command: None,
                    data: Some(
                        serde_json::to_value(CodeLensData::TargetReferences(
                            CodeLensDataTargetReferences {
                                path: path.clone(),
                                target_name: target.name.to_string(),
                            },
                        ))
                        .unwrap(),
                    ),
                }
            }))
        }
    }

    lens.extend(targets.iter().map(|target| {
        let range = current_file.document.line_index.range(target.call.span);
        let dir_path = format_path(
            current_file.document.path.parent().unwrap(),
            &current_file.workspace_root,
        );
        let label = if dir_path.ends_with(&format!("/{}", target.name)) {
            dir_path
        } else {
            format!("{}:{}", dir_path, target.name)
        };
        CodeLens {
            range,
            command: Some(Command {
                title: "copy label".to_string(),
                command: "gn.copyTargetLabel".to_string(),
                arguments: Some(vec![Value::String(label)]),
            }),
            data: None,
        }
    }));

    Ok(Some(lens))
}

pub async fn code_lens_resolve(
    context: &RequestContext,
    partial_lens: CodeLens,
) -> Result<CodeLens> {
    let data = serde_json::from_value::<CodeLensData>(partial_lens.data.unwrap())?;
    match data {
        CodeLensData::TargetReferences(CodeLensDataTargetReferences { path, target_name }) => {
            let workspace = context.analyzer.workspace_for(&path)?;
            compute_references_lens(
                &workspace,
                &path,
                partial_lens.range,
                &target_name,
                context.request_time,
            )
            .await
        }
    }
}
