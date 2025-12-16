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
    path::Path,
    sync::{Arc, Mutex},
    time::Instant,
};

use futures::future::join_all;

use crate::{
    analyzer::{Analyzer, IndexingLevel},
    common::{
        storage::DocumentStorage, utils::find_gn_in_workspace_for_scan, workspace::WorkspaceFinder,
    },
};

pub async fn run_bench(workspace_root: &Path) {
    let storage = Arc::new(Mutex::new(DocumentStorage::new()));
    let analyzer = Arc::new(Analyzer::new(
        &storage,
        WorkspaceFinder::new(Some(workspace_root)),
        IndexingLevel::Disabled,
    ));

    let start_time = Instant::now();
    let mut count = 0;

    let mut tasks = Vec::new();
    for path in find_gn_in_workspace_for_scan(workspace_root) {
        let analyzer = analyzer.clone();
        tasks.push(tokio::spawn(async move {
            if let Ok(file) = analyzer.analyze_file(&path, start_time) {
                let diagnostics =
                    crate::diagnostics::compute_diagnostics(&file, &analyzer, start_time);
                for d in diagnostics {
                    println!(
                        "{}:{}:{}: {}",
                        path.display(),
                        d.range.start.line + 1,
                        d.range.start.character + 1,
                        d.message
                    );
                }
            }
            eprint!(".");
        }));
        count += 1;
    }
    join_all(tasks).await;

    let elapsed = start_time.elapsed();

    eprintln!();
    eprintln!("Processed {} files in {:.1}s", count, elapsed.as_secs_f64());
}
