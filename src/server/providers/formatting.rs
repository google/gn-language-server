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

use std::process::Stdio;

use pest::Span;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::Command,
};
use tower_lsp::lsp_types::{DocumentFormattingParams, TextEdit};

use crate::{
    common::{
        binary::find_gn_binary,
        error::{Error, Result},
    },
    server::{providers::utils::get_text_document_path, RequestContext},
};

pub async fn formatting(
    context: &RequestContext,
    params: DocumentFormattingParams,
) -> Result<Option<Vec<TextEdit>>> {
    let file_path = get_text_document_path(&params.text_document)?;

    let configs = context.client.configurations().await;
    let gn_path = if let Some(gn_path) = &configs.binary_path {
        if gn_path.exists() {
            gn_path.to_path_buf()
        } else {
            return Err(Error::General(format!(
                "gn binary not found at {}; check configuration value gn.binaryPath",
                gn_path.display()
            )));
        }
    } else if let Some(gn_path) =
        find_gn_binary(context.analyzer.workspace_finder().find_for(&file_path))
    {
        gn_path
    } else {
        return Err(Error::General(
            "gn binary not found; specify configuration value gn.binaryPath".to_string(),
        ));
    };

    let document = context.storage.lock().unwrap().read(&file_path);

    let mut process = Command::new(gn_path)
        .args(["format", "--stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;

    let write_task = {
        let mut stdin = process.stdin.take().unwrap();
        let document = document.clone();
        async move {
            // Drop stdin on completion of the task to close the pipe.
            stdin.write_all(document.data.as_bytes()).await
        }
    };

    let mut stdout = process.stdout.take().unwrap();
    let mut formatted = String::new();
    let read_task = stdout.read_to_string(&mut formatted);

    let io_result = tokio::try_join!(write_task, read_task);

    // Check the status first.
    let status = process.wait().await.unwrap();
    if !status.success() {
        return Err(Error::General(format!(
            "gn format failed with status {}",
            status.code().unwrap_or(-1)
        )));
    }

    // Check the IO result then.
    io_result?;

    let whole_range = document
        .line_index
        .range(Span::new(&document.data, 0, document.data.len()).unwrap());
    Ok(Some(vec![TextEdit {
        range: whole_range,
        new_text: formatted,
    }]))
}
