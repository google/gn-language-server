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
    borrow::Cow,
    sync::{Arc, Mutex},
};

use tower_lsp::{lsp_types::Position, Client};

use crate::{
    analyze::{AnalyzedFile, Analyzer},
    ast::{Identifier, Node},
    storage::DocumentStorage,
};

pub mod completion;
pub mod document;
pub mod document_link;
pub mod document_symbol;
pub mod goto_definition;
pub mod hover;
pub mod indexing;

pub type RpcResult<T> = tower_lsp::jsonrpc::Result<T>;

#[derive(Clone)]
pub struct ProviderContext {
    pub storage: Arc<Mutex<DocumentStorage>>,
    pub analyzer: Arc<Mutex<Analyzer>>,
    pub client: Client,
}

pub fn into_rpc_error(err: std::io::Error) -> tower_lsp::jsonrpc::Error {
    let mut rpc_err = tower_lsp::jsonrpc::Error::internal_error();
    rpc_err.message = Cow::from(err.to_string());
    rpc_err
}

pub fn lookup_identifier_at(file: &AnalyzedFile, position: Position) -> Option<&Identifier> {
    let offset = file.document.line_index.offset(position)?;
    file.ast_root
        .identifiers()
        .find(|ident| ident.span.start() <= offset && offset <= ident.span.end())
}
