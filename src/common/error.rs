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

use std::{borrow::Cow, io::ErrorKind};

pub type RpcError = tower_lsp::jsonrpc::Error;
pub type RpcResult<T> = tower_lsp::jsonrpc::Result<T>;

fn new_rpc_error(message: String) -> RpcError {
    tower_lsp::jsonrpc::Error {
        code: tower_lsp::jsonrpc::ErrorCode::ServerError(1),
        message: Cow::from(message),
        data: None,
    }
}

#[derive(thiserror::Error, Clone, Debug)]
pub enum Error {
    #[error("{0}")]
    General(String),
    #[error("File not found")]
    NotFound,
}

impl Error {
    pub fn is_not_found(&self) -> bool {
        matches!(self, Error::NotFound)
    }
}

impl From<Error> for RpcError {
    fn from(error: Error) -> Self {
        new_rpc_error(error.to_string())
    }
}

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        if error.kind() == ErrorKind::NotFound {
            Error::NotFound
        } else {
            Error::General(error.to_string())
        }
    }
}

impl From<serde_json::Error> for Error {
    fn from(error: serde_json::Error) -> Self {
        Error::General(error.to_string())
    }
}

pub type Result<T> = std::result::Result<T, Error>;
