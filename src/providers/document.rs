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

use tower_lsp::lsp_types::{
    DidChangeTextDocumentParams, DidCloseTextDocumentParams, DidOpenTextDocumentParams,
};

use super::{
    diagnostics::{publish_diagnostics, unpublish_diagnostics},
    ProviderContext,
};

pub async fn did_open(context: &ProviderContext, params: DidOpenTextDocumentParams) {
    let Ok(path) = params.text_document.uri.to_file_path() else {
        return;
    };

    context.storage.lock().unwrap().load_to_memory(
        &path,
        &params.text_document.text,
        params.text_document.version,
    );

    publish_diagnostics(context, &params.text_document.uri).await;
}

pub async fn did_change(context: &ProviderContext, params: DidChangeTextDocumentParams) {
    let Ok(path) = params.text_document.uri.to_file_path() else {
        return;
    };
    let Some(change) = params.content_changes.first() else {
        return;
    };

    context.storage.lock().unwrap().load_to_memory(
        &path,
        &change.text,
        params.text_document.version,
    );

    publish_diagnostics(context, &params.text_document.uri).await;
}

pub async fn did_close(context: &ProviderContext, params: DidCloseTextDocumentParams) {
    let Ok(path) = params.text_document.uri.to_file_path() else {
        return;
    };

    unpublish_diagnostics(context, &params.text_document.uri).await;

    context.storage.lock().unwrap().unload_from_memory(&path);
}
