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

use futures::{future::join_all, FutureExt};
use tower_lsp::lsp_types::{DidChangeConfigurationParams, Url};

use super::{
    diagnostics::{publish_diagnostics, unpublish_diagnostics},
    ProviderContext,
};

pub async fn did_change_configuration(
    context: &ProviderContext,
    _params: DidChangeConfigurationParams,
) {
    let config = context.client.configurations().await;

    let documents = context.storage.lock().unwrap().memory_docs();

    let mut tasks = Vec::new();
    if config.experimental.error_reporting {
        for document in documents {
            tasks.push(
                async move {
                    publish_diagnostics(context, &Url::from_file_path(&document.path).unwrap())
                        .await
                }
                .boxed(),
            );
        }
    } else {
        for document in documents {
            tasks.push(
                async move {
                    unpublish_diagnostics(context, &Url::from_file_path(&document.path).unwrap())
                        .await
                }
                .boxed(),
            );
        }
    }

    join_all(tasks).await;
}
