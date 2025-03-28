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

use std::{borrow::Cow, path::PathBuf};

use tower_lsp::lsp_types::{DocumentLink, DocumentLinkParams, Url};

use crate::analyze::Link;

use super::{find_target_position, into_rpc_error, new_rpc_error, ProviderContext, RpcResult};

#[derive(serde::Serialize, serde::Deserialize)]
struct TargetLinkData {
    path: PathBuf,
    name: String,
}

pub async fn document_link(
    context: &ProviderContext,
    params: DocumentLinkParams,
) -> RpcResult<Option<Vec<DocumentLink>>> {
    let Ok(path) = params.text_document.uri.to_file_path() else {
        return Err(new_rpc_error(Cow::from(format!(
            "invalid file URI: {}",
            params.text_document.uri
        ))));
    };

    let current_file = context
        .analyzer
        .lock()
        .unwrap()
        .analyze(&path)
        .map_err(into_rpc_error)?;

    let links = current_file
        .links
        .iter()
        .map(|link| match link {
            Link::File { path, span } => DocumentLink {
                target: Some(Url::from_file_path(path).unwrap()),
                range: current_file.document.line_index.range(*span),
                tooltip: None,
                data: None,
            },
            Link::Target { path, name, span } => DocumentLink {
                target: None, // Resolve with positions later.
                range: current_file.document.line_index.range(*span),
                tooltip: None,
                data: Some(
                    serde_json::to_value(TargetLinkData {
                        path: path.to_path_buf(),
                        name: name.to_string(),
                    })
                    .unwrap(),
                ),
            },
        })
        .collect();

    Ok(Some(links))
}

pub async fn document_link_resolve(
    context: &ProviderContext,
    mut link: DocumentLink,
) -> RpcResult<DocumentLink> {
    let Some(data) = link
        .data
        .take()
        .and_then(|value| serde_json::from_value::<TargetLinkData>(value).ok())
    else {
        return Err(new_rpc_error(Cow::from("corrupted target link data")));
    };

    let target_file = context
        .analyzer
        .lock()
        .unwrap()
        .analyze(&data.path)
        .map_err(into_rpc_error)?;

    let position = find_target_position(&target_file, &data.name).unwrap_or_default();
    let mut uri = Url::from_file_path(&data.path).unwrap();
    uri.set_fragment(Some(&format!(
        "L{},{}",
        position.line + 1,
        position.character + 1,
    )));
    link.target = Some(uri);
    Ok(link)
}
