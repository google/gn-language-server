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

use tower_lsp::lsp_types::{DocumentSymbolParams, DocumentSymbolResponse};

use crate::{
    common::error::Result,
    server::{providers::utils::get_text_document_path, RequestContext},
};

pub async fn document_symbol(
    context: &RequestContext,
    params: DocumentSymbolParams,
) -> Result<Option<DocumentSymbolResponse>> {
    let path = get_text_document_path(&params.text_document)?;
    let current_file = context.analyzer.analyze_file(&path, context.request_time)?;

    Ok(Some(DocumentSymbolResponse::Nested(
        current_file.outline.clone(),
    )))
}
