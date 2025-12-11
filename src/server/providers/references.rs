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
    analyzer::{AnalyzedBlock, AnalyzedFile, AnalyzedLink},
    common::error::{Error, Result},
    server::{
        providers::utils::{get_text_document_path, lookup_target_name_string_at},
        RequestContext,
    },
};

fn get_overlapping_targets<'i>(root: &AnalyzedBlock<'i, '_>, prefix: &str) -> Vec<&'i str> {
    root.targets()
        .filter(|target| target.name.len() > prefix.len() && target.name.starts_with(prefix))
        .map(|target| target.name)
        .collect()
}

pub async fn target_references(
    context: &RequestContext,
    current_file: &AnalyzedFile,
    target_name: &str,
) -> Result<Option<Vec<Location>>> {
    let bad_prefixes = get_overlapping_targets(&current_file.analyzed_root, target_name);

    // Wait for the workspace indexing to finish.
    let workspace_root = &current_file.workspace_root;
    let Some(indexed) = context.indexed.lock().unwrap().get(workspace_root).cloned() else {
        return Err(Error::General(format!(
            "Indexing for {} not started",
            workspace_root.display()
        )));
    };
    indexed.wait().await;

    let cached_files = context.analyzer.cached_files(workspace_root);

    let mut references: Vec<Location> = Vec::new();
    for file in cached_files {
        for link in &file.links {
            let AnalyzedLink::Target { path, name, span } = link else {
                continue;
            };
            if path != &current_file.document.path {
                continue;
            }
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

    Ok(Some(references))
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
    let current_file = context.analyzer.analyze_file(&path, context.request_time)?;

    let Some(pos) = current_file
        .document
        .line_index
        .offset(params.text_document_position.position)
    else {
        return Ok(None);
    };

    if let Some(target) = lookup_target_name_string_at(&current_file, pos) {
        return target_references(context, &current_file, target.name).await;
    };

    Ok(None)
}
