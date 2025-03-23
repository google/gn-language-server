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
    collections::HashSet,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use tokio::spawn;
use tower_lsp::{
    lsp_types::{
        CompletionOptions, CompletionParams, CompletionResponse, DidChangeConfigurationParams,
        DidChangeTextDocumentParams, DidCloseTextDocumentParams, DidOpenTextDocumentParams,
        DocumentFormattingParams, DocumentLink, DocumentLinkOptions, DocumentLinkParams,
        DocumentSymbolParams, DocumentSymbolResponse, GotoDefinitionParams, GotoDefinitionResponse,
        Hover, HoverParams, HoverProviderCapability, InitializeParams, InitializeResult,
        InitializedParams, MessageType, OneOf, ServerCapabilities, TextDocumentSyncCapability,
        TextDocumentSyncKind, TextEdit,
    },
    LanguageServer, LspService, Server,
};

use crate::{
    analyze::{find_workspace_root, Analyzer},
    client::TestableClient,
    providers::{ProviderContext, RpcResult},
    storage::DocumentStorage,
};

struct Backend {
    context: ProviderContext,
    indexed_workspaces: Mutex<HashSet<PathBuf>>,
}

impl Backend {
    pub fn new(
        storage: &Arc<Mutex<DocumentStorage>>,
        analyzer: &Arc<Mutex<Analyzer>>,
        client: TestableClient,
    ) -> Self {
        Self {
            context: ProviderContext {
                storage: storage.clone(),
                analyzer: analyzer.clone(),
                client,
            },
            indexed_workspaces: Mutex::new(HashSet::new()),
        }
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, _: InitializeParams) -> RpcResult<InitializeResult> {
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                definition_provider: Some(OneOf::Left(true)),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                document_link_provider: Some(DocumentLinkOptions {
                    resolve_provider: Some(true),
                    work_done_progress_options: Default::default(),
                }),
                document_symbol_provider: Some(OneOf::Left(true)),
                completion_provider: Some(CompletionOptions::default()),
                document_formatting_provider: Some(OneOf::Left(true)),
                ..Default::default()
            },
            ..Default::default()
        })
    }

    async fn initialized(&self, _params: InitializedParams) {
        self.context
            .client
            .log_message(MessageType::INFO, "GN language server initialized")
            .await;
    }

    async fn shutdown(&self) -> RpcResult<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let configurations = self.context.client.configurations().await;
        if configurations.experimental.background_indexing {
            if let Ok(path) = params.text_document.uri.to_file_path() {
                if let Ok(workspace_root) = find_workspace_root(&path) {
                    let workspace_root = workspace_root.to_path_buf();
                    let do_index = {
                        let mut indexed_workspaces = self.indexed_workspaces.lock().unwrap();
                        indexed_workspaces.insert(workspace_root.clone())
                    };
                    if do_index {
                        let context = self.context.clone();
                        spawn(async move {
                            crate::providers::indexing::index(&context, &workspace_root).await;
                        });
                    }
                }
            };
        }
        crate::providers::document::did_open(&self.context, params).await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        crate::providers::document::did_change(&self.context, params).await;
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        crate::providers::document::did_close(&self.context, params).await;
    }

    async fn did_change_configuration(&self, params: DidChangeConfigurationParams) {
        crate::providers::configuration::did_change_configuration(&self.context, params).await;
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> RpcResult<Option<GotoDefinitionResponse>> {
        crate::providers::goto_definition::goto_definition(&self.context, params).await
    }

    async fn hover(&self, params: HoverParams) -> RpcResult<Option<Hover>> {
        crate::providers::hover::hover(&self.context, params).await
    }

    async fn document_link(
        &self,
        params: DocumentLinkParams,
    ) -> RpcResult<Option<Vec<DocumentLink>>> {
        crate::providers::document_link::document_link(&self.context, params).await
    }

    async fn document_link_resolve(&self, link: DocumentLink) -> RpcResult<DocumentLink> {
        crate::providers::document_link::document_link_resolve(&self.context, link).await
    }

    async fn document_symbol(
        &self,
        params: DocumentSymbolParams,
    ) -> RpcResult<Option<DocumentSymbolResponse>> {
        crate::providers::document_symbol::document_symbol(&self.context, params).await
    }

    async fn completion(&self, params: CompletionParams) -> RpcResult<Option<CompletionResponse>> {
        crate::providers::completion::completion(&self.context, params).await
    }

    async fn formatting(
        &self,
        params: DocumentFormattingParams,
    ) -> RpcResult<Option<Vec<TextEdit>>> {
        crate::providers::formatting::formatting(&self.context, params).await
    }
}

pub async fn run() {
    let storage = Arc::new(Mutex::new(DocumentStorage::new()));
    let analyzer = Arc::new(Mutex::new(Analyzer::new(&storage)));
    let (service, socket) = LspService::new(move |client| {
        Backend::new(&storage, &analyzer, TestableClient::new(client))
    });

    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    Server::new(stdin, stdout, socket).serve(service).await;
}
