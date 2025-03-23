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

use std::fmt::Display;

use itertools::Itertools;
use tower_lsp::lsp_types::{ConfigurationItem, Diagnostic, MessageType, Url};

use crate::config::Configurations;

#[derive(Clone)]
pub struct TestableClient {
    client: Option<tower_lsp::Client>,
}

impl TestableClient {
    pub fn new(client: tower_lsp::Client) -> Self {
        Self {
            client: Some(client),
        }
    }

    #[cfg(test)]
    pub fn new_for_testing() -> Self {
        Self { client: None }
    }

    pub async fn log_message<M: Display>(&self, typ: MessageType, message: M) {
        if let Some(client) = &self.client {
            client.log_message(typ, message).await;
        }
    }

    pub async fn configurations(&self) -> Configurations {
        let Some(client) = &self.client else {
            return Configurations::default();
        };

        let Ok(values) = client
            .configuration(vec![ConfigurationItem {
                scope_uri: None,
                section: Some("gn".to_string()),
            }])
            .await
        else {
            return Configurations::default();
        };

        let Ok(value) = values.into_iter().exactly_one() else {
            return Configurations::default();
        };

        serde_json::from_value(value).unwrap_or_default()
    }

    pub async fn publish_diagnostics(
        &self,
        uri: Url,
        diags: Vec<Diagnostic>,
        version: Option<i32>,
    ) {
        if let Some(client) = &self.client {
            client.publish_diagnostics(uri, diags, version).await;
        };
    }
}
