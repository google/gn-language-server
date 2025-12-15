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

use tower_lsp::lsp_types::{Location, ReferenceParams, Url};

use crate::{
    analyzer::{AnalyzedBlock, AnalyzedFile, AnalyzedLink, WorkspaceAnalyzer},
    common::error::Result,
    server::{
        providers::utils::{get_text_document_path, lookup_target_name_string_at},
        RequestContext,
    },
};

fn get_overlapping_targets<'p>(root: &AnalyzedBlock<'p>, prefix: &str) -> Vec<&'p str> {
    root.targets()
        .filter(|target| target.name.len() > prefix.len() && target.name.starts_with(prefix))
        .map(|target| target.name)
        .collect()
}

pub fn target_references(
    workspace: &WorkspaceAnalyzer,
    current_file: &AnalyzedFile,
    target_name: &str,
) -> Result<Vec<Location>> {
    let bad_prefixes = get_overlapping_targets(current_file.analyzed_root.get(), target_name);

    let cached_files = workspace.cached_files_for_references();

    let mut references: Vec<Location> = Vec::new();
    for file in cached_files {
        let Some(links) = file.link_index.get().get(&current_file.document.path) else {
            continue;
        };
        for link in links {
            let AnalyzedLink::Target { name, span, .. } = link else {
                continue;
            };
            if bad_prefixes
                .iter()
                .any(|bad_prefix| name.starts_with(bad_prefix))
            {
                continue;
            }
            if !name.starts_with(target_name) {
                continue;
            }
            references.push(Location {
                uri: Url::from_file_path(&file.document.path).unwrap(),
                range: file.document.line_index.range(*span),
            });
        }
    }

    Ok(references)
}

pub async fn references(
    context: &RequestContext,
    params: ReferenceParams,
) -> Result<Option<Vec<Location>>> {
    // Require background indexing.
    if !context.client.configurations().await.background_indexing {
        return Ok(None);
    }

    let path = get_text_document_path(&params.text_document_position.text_document)?;
    let workspace = context.analyzer.workspace_for(&path)?;
    let current_file = workspace.analyze_file(&path, context.request_time);

    let Some(pos) = current_file
        .document
        .line_index
        .offset(params.text_document_position.position)
    else {
        return Ok(None);
    };

    if let Some(target) = lookup_target_name_string_at(&current_file, pos) {
        // Wait for the workspace indexing to finish.
        workspace.indexed().wait().await;
        return Ok(Some(target_references(
            &workspace,
            &current_file,
            target.name,
        )?));
    };

    Ok(None)
}
