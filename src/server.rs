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
        InitializedParams, Location, MessageType, OneOf, ReferenceParams, ServerCapabilities,
        TextDocumentSyncCapability, TextDocumentSyncKind, TextEdit,
    },
    LanguageServer, LspService, Server,
};

use crate::{
    analyze::{find_workspace_root, Analyzer},
    client::TestableClient,
    error::RpcResult,
    storage::DocumentStorage,
    util::CacheConfig,
};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum RequestType {
    Interactive,
    Background,
}

#[derive(Clone)]
pub struct ServerContext {
    pub storage: Arc<Mutex<DocumentStorage>>,
    pub analyzer: Arc<Mutex<Analyzer>>,
    pub client: TestableClient,
}

impl ServerContext {
    #[cfg(test)]
    pub fn new_for_testing() -> Self {
        let storage = Arc::new(Mutex::new(DocumentStorage::new()));
        let analyzer = Arc::new(Mutex::new(Analyzer::new(&storage)));
        Self {
            storage,
            analyzer,
            client: TestableClient::new_for_testing(),
        }
    }

    pub fn request(&self, request_type: RequestType) -> RequestContext {
        let update_shallow = matches!(request_type, RequestType::Interactive);
        RequestContext {
            storage: self.storage.clone(),
            analyzer: self.analyzer.clone(),
            client: self.client.clone(),
            cache_config: CacheConfig::new(update_shallow),
        }
    }
}

#[derive(Clone)]
pub struct RequestContext {
    pub storage: Arc<Mutex<DocumentStorage>>,
    pub analyzer: Arc<Mutex<Analyzer>>,
    pub client: TestableClient,
    pub cache_config: CacheConfig,
}

impl RequestContext {
    #[cfg(test)]
    pub fn new_for_testing(request_type: RequestType) -> Self {
        ServerContext::new_for_testing().request(request_type)
    }
}

struct Backend {
    context: ServerContext,
    indexed_workspaces: Mutex<HashSet<PathBuf>>,
}

impl Backend {
    pub fn new(
        storage: &Arc<Mutex<DocumentStorage>>,
        analyzer: &Arc<Mutex<Analyzer>>,
        client: TestableClient,
    ) -> Self {
        Self {
            context: ServerContext {
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
                references_provider: Some(OneOf::Left(true)),
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
        let context = self.context.request(RequestType::Interactive);
        let configurations = self.context.client.configurations().await;
        if configurations.background_indexing {
            if let Ok(path) = params.text_document.uri.to_file_path() {
                if let Ok(workspace_root) = find_workspace_root(&path) {
                    let workspace_root = workspace_root.to_path_buf();
                    let do_index = {
                        let mut indexed_workspaces = self.indexed_workspaces.lock().unwrap();
                        indexed_workspaces.insert(workspace_root.clone())
                    };
                    if do_index {
                        let context = context.clone();
                        spawn(async move {
                            crate::providers::indexing::index(&context, &path).await;
                        });
                    }
                }
            };
        }
        crate::providers::document::did_open(&context, params).await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        crate::providers::document::did_change(
            &self.context.request(RequestType::Background),
            params,
        )
        .await;
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        crate::providers::document::did_close(
            &self.context.request(RequestType::Interactive),
            params,
        )
        .await;
    }

    async fn did_change_configuration(&self, params: DidChangeConfigurationParams) {
        crate::providers::configuration::did_change_configuration(
            &self.context.request(RequestType::Interactive),
            params,
        )
        .await;
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> RpcResult<Option<GotoDefinitionResponse>> {
        Ok(crate::providers::goto_definition::goto_definition(
            &self.context.request(RequestType::Interactive),
            params,
        )
        .await?)
    }

    async fn hover(&self, params: HoverParams) -> RpcResult<Option<Hover>> {
        Ok(
            crate::providers::hover::hover(&self.context.request(RequestType::Background), params)
                .await?,
        )
    }

    async fn document_link(
        &self,
        params: DocumentLinkParams,
    ) -> RpcResult<Option<Vec<DocumentLink>>> {
        Ok(crate::providers::document_link::document_link(
            &self.context.request(RequestType::Background),
            params,
        )
        .await?)
    }

    async fn document_link_resolve(&self, link: DocumentLink) -> RpcResult<DocumentLink> {
        Ok(crate::providers::document_link::document_link_resolve(
            &self.context.request(RequestType::Interactive),
            link,
        )
        .await?)
    }

    async fn document_symbol(
        &self,
        params: DocumentSymbolParams,
    ) -> RpcResult<Option<DocumentSymbolResponse>> {
        Ok(crate::providers::document_symbol::document_symbol(
            &self.context.request(RequestType::Background),
            params,
        )
        .await?)
    }

    async fn completion(&self, params: CompletionParams) -> RpcResult<Option<CompletionResponse>> {
        Ok(crate::providers::completion::completion(
            &self.context.request(RequestType::Interactive),
            params,
        )
        .await?)
    }

    async fn references(&self, params: ReferenceParams) -> RpcResult<Option<Vec<Location>>> {
        Ok(crate::providers::references::references(
            &self.context.request(RequestType::Interactive),
            params,
        )
        .await?)
    }

    async fn formatting(
        &self,
        params: DocumentFormattingParams,
    ) -> RpcResult<Option<Vec<TextEdit>>> {
        Ok(crate::providers::formatting::formatting(
            &self.context.request(RequestType::Interactive),
            params,
        )
        .await?)
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
