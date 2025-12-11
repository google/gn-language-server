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

use std::{path::Path, time::Instant};

use tower_lsp::lsp_types::MessageType;

use crate::{
    common::{
        error::{Error, Result},
        utils::find_gn_files,
    },
    server::RequestContext,
};

pub async fn index(context: &RequestContext, workspace_root: &Path) {
    context
        .client
        .log_message(
            MessageType::INFO,
            format!("Indexing {} in the background...", workspace_root.display()),
        )
        .await;

    let start_time = Instant::now();
    let mut count = 0;

    for path in find_gn_files(workspace_root) {
        context
            .analyzer
            .analyze_file(&path, context.request_time)
            .ok();
        count += 1;
    }

    let elapsed = start_time.elapsed();
    context
        .client
        .log_message(
            MessageType::INFO,
            format!(
                "Finished indexing {}: processed {} files in {:.1}s",
                workspace_root.display(),
                count,
                elapsed.as_secs_f64()
            ),
        )
        .await;
}

pub async fn wait_indexing(context: &RequestContext, workspace_root: &Path) -> Result<()> {
    let Some(indexed) = context.indexed.lock().unwrap().get(workspace_root).cloned() else {
        return Err(Error::General(format!(
            "Indexing for {} not started",
            workspace_root.display()
        )));
    };
    indexed.wait().await;
    Ok(())
}

pub fn check_indexing(context: &RequestContext, workspace_root: &Path) -> Result<bool> {
    let Some(indexed) = context.indexed.lock().unwrap().get(workspace_root).cloned() else {
        return Err(Error::General(format!(
            "Indexing for {} not started",
            workspace_root.display()
        )));
    };
    Ok(indexed.done())
}
