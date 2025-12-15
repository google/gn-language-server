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
    collections::{btree_map::Entry, BTreeMap},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
    time::Instant,
};

use tokio::spawn;
use tower_lsp::{
    lsp_types::{
        CodeActionKind, CodeActionOptions, CodeActionParams, CodeActionProviderCapability,
        CodeActionResponse, CodeLens, CodeLensOptions, CodeLensParams, CompletionOptions,
        CompletionParams, CompletionResponse, DidChangeConfigurationParams,
        DidChangeTextDocumentParams, DidCloseTextDocumentParams, DidOpenTextDocumentParams,
        DocumentFormattingParams, DocumentLink, DocumentLinkOptions, DocumentLinkParams,
        DocumentSymbolParams, DocumentSymbolResponse, GotoDefinitionParams, GotoDefinitionResponse,
        Hover, HoverParams, HoverProviderCapability, InitializeParams, InitializeResult,
        InitializedParams, Location, MessageType, OneOf, ReferenceParams, ServerCapabilities,
        SymbolInformation, TextDocumentSyncCapability, TextDocumentSyncKind, TextEdit, Url,
        WorkspaceSymbolParams,
    },
    LanguageServer, LspService, Server,
};

use crate::{
    analyzer::Analyzer,
    common::{
        client::TestableClient, error::RpcResult, storage::DocumentStorage, utils::AsyncSignal,
        workspace::WorkspaceFinder,
    },
};

mod indexing;
mod providers;

struct ServerContext {
    pub storage: Arc<Mutex<DocumentStorage>>,
    pub analyzer: OnceLock<Arc<Analyzer>>,
    pub indexed: Arc<Mutex<BTreeMap<PathBuf, AsyncSignal>>>,
    pub client: TestableClient,
}

impl ServerContext {
    pub fn new(storage: Arc<Mutex<DocumentStorage>>, client: TestableClient) -> Self {
        Self {
            storage,
            analyzer: OnceLock::new(),
            indexed: Default::default(),
            client,
        }
    }

    #[cfg(test)]
    pub fn new_for_testing(client_root: Option<&Path>) -> Self {
        let storage = Arc::new(Mutex::new(DocumentStorage::new()));
        let analyzer = OnceLock::new();
        let _ = analyzer.set(Arc::new(Analyzer::new(
            &storage,
            WorkspaceFinder::new(client_root),
        )));
        Self {
            storage,
            analyzer,
            indexed: Default::default(),
            client: TestableClient::new_for_testing(),
        }
    }

    pub fn request(&self) -> RequestContext {
        RequestContext {
            storage: self.storage.clone(),
            analyzer: self.analyzer.get().unwrap().clone(),
            indexed: self.indexed.clone(),
            client: self.client.clone(),
            request_time: Instant::now(),
        }
    }
}

#[derive(Clone)]
pub struct RequestContext {
    pub storage: Arc<Mutex<DocumentStorage>>,
    pub analyzer: Arc<Analyzer>,
    pub indexed: Arc<Mutex<BTreeMap<PathBuf, AsyncSignal>>>,
    pub client: TestableClient,
    pub request_time: Instant,
}

impl RequestContext {
    #[cfg(test)]
    pub fn new_for_testing(client_root: Option<&Path>) -> Self {
        ServerContext::new_for_testing(client_root).request()
    }
}

struct Backend {
    context: ServerContext,
}

impl Backend {
    pub fn new(storage: Arc<Mutex<DocumentStorage>>, client: TestableClient) -> Self {
        Self {
            context: ServerContext::new(storage, client),
        }
    }

    async fn maybe_index_workspace_for(&self, context: &RequestContext, path: &Path) {
        let Some(workspace_root) = context.analyzer.workspace_finder().find_for(path) else {
            return;
        };
        let workspace_root = workspace_root.to_path_buf();

        let mut indexed = match context
            .indexed
            .lock()
            .unwrap()
            .entry(workspace_root.to_path_buf())
        {
            Entry::Occupied(_) => return,
            Entry::Vacant(entry) => entry.insert(AsyncSignal::new()).clone(),
        };

        let context = context.clone();
        spawn(async move {
            indexing::index(&context, &workspace_root).await;
            indexed.set();
        });
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, params: InitializeParams) -> RpcResult<InitializeResult> {
        let finder = WorkspaceFinder::new(
            params
                .root_uri
                .and_then(|root_uri| root_uri.to_file_path().ok())
                .as_deref(),
        );
        let analyzer = Arc::new(Analyzer::new(&self.context.storage, finder));
        self.context.analyzer.set(analyzer).ok();

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
                workspace_symbol_provider: Some(OneOf::Left(true)),
                code_lens_provider: Some(CodeLensOptions {
                    resolve_provider: Some(true),
                }),
                code_action_provider: Some(CodeActionProviderCapability::Options(
                    CodeActionOptions {
                        code_action_kinds: Some(vec![CodeActionKind::QUICKFIX]),
                        resolve_provider: Some(true),
                        ..Default::default()
                    },
                )),
                ..Default::default()
            },
            ..Default::default()
        })
    }

    async fn initialized(&self, _params: InitializedParams) {
        let context = self.context.request();
        context
            .client
            .log_message(MessageType::INFO, "GN language server initialized")
            .await;

        let configurations = self.context.client.configurations().await;
        if !configurations.background_indexing {
            return;
        }
    }

    async fn shutdown(&self) -> RpcResult<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let context = self.context.request();
        let Ok(path) = Url::to_file_path(&params.text_document.uri) else {
            return;
        };
        let configurations = self.context.client.configurations().await;
        if configurations.background_indexing {
            self.maybe_index_workspace_for(&context, &path).await;
        }
        providers::document::did_open(&self.context.request(), params).await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        providers::document::did_change(&self.context.request(), params).await;
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        providers::document::did_close(&self.context.request(), params).await;
    }

    async fn did_change_configuration(&self, params: DidChangeConfigurationParams) {
        providers::configuration::did_change_configuration(&self.context.request(), params).await;
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> RpcResult<Option<GotoDefinitionResponse>> {
        Ok(providers::goto_definition::goto_definition(&self.context.request(), params).await?)
    }

    async fn hover(&self, params: HoverParams) -> RpcResult<Option<Hover>> {
        Ok(providers::hover::hover(&self.context.request(), params).await?)
    }

    async fn document_link(
        &self,
        params: DocumentLinkParams,
    ) -> RpcResult<Option<Vec<DocumentLink>>> {
        Ok(providers::document_link::document_link(&self.context.request(), params).await?)
    }

    async fn document_link_resolve(&self, link: DocumentLink) -> RpcResult<DocumentLink> {
        Ok(providers::document_link::document_link_resolve(&self.context.request(), link).await?)
    }

    async fn document_symbol(
        &self,
        params: DocumentSymbolParams,
    ) -> RpcResult<Option<DocumentSymbolResponse>> {
        Ok(providers::document_symbol::document_symbol(&self.context.request(), params).await?)
    }

    async fn completion(&self, params: CompletionParams) -> RpcResult<Option<CompletionResponse>> {
        Ok(providers::completion::completion(&self.context.request(), params).await?)
    }

    async fn references(&self, params: ReferenceParams) -> RpcResult<Option<Vec<Location>>> {
        Ok(providers::references::references(&self.context.request(), params).await?)
    }

    async fn formatting(
        &self,
        params: DocumentFormattingParams,
    ) -> RpcResult<Option<Vec<TextEdit>>> {
        Ok(providers::formatting::formatting(&self.context.request(), params).await?)
    }

    async fn symbol(
        &self,
        params: WorkspaceSymbolParams,
    ) -> RpcResult<Option<Vec<SymbolInformation>>> {
        Ok(providers::workspace_symbol::workspace_symbol(&self.context.request(), params).await?)
    }

    async fn code_lens(&self, params: CodeLensParams) -> RpcResult<Option<Vec<CodeLens>>> {
        Ok(providers::code_lens::code_lens(&self.context.request(), params).await?)
    }

    async fn code_lens_resolve(&self, partial_lens: CodeLens) -> RpcResult<CodeLens> {
        Ok(providers::code_lens::code_lens_resolve(&self.context.request(), partial_lens).await?)
    }

    async fn code_action(&self, params: CodeActionParams) -> RpcResult<Option<CodeActionResponse>> {
        Ok(providers::code_action::code_action(&self.context.request(), params).await?)
    }
}

pub async fn run() {
    let storage = Arc::new(Mutex::new(DocumentStorage::new()));
    let (service, socket) =
        LspService::new(move |client| Backend::new(storage, TestableClient::new(client)));

    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    Server::new(stdin, stdout, socket).serve(service).await;
}
