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

use tower_lsp::lsp_types::{SymbolInformation, WorkspaceSymbolParams};

use crate::{
    common::error::Result,
    server::{symbols::SymbolSet, RequestContext},
};

pub async fn workspace_symbol(
    context: &RequestContext,
    params: WorkspaceSymbolParams,
) -> Result<Option<Vec<SymbolInformation>>> {
    let symbols = SymbolSet::global(&context.analyzer).await;

    let query = params.query.to_lowercase();
    let symbols = symbols
        .symbol_informations()
        .filter(|symbol| symbol.name.starts_with(&query))
        .collect();

    Ok(Some(symbols))
}
