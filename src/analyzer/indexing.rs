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

use std::{sync::Arc, time::Instant};

use futures::{future::join_all, FutureExt};

use crate::{analyzer::WorkspaceAnalyzer, common::utils::find_gn_in_workspace_for_scan};

pub async fn build_index(analyzer: &Arc<WorkspaceAnalyzer>, parallel: bool) {
    eprintln!(
        "Indexing {} in the background...",
        analyzer.context().root.display()
    );

    let start_time = Instant::now();
    let mut tasks = Vec::new();
    let mut count = 0;

    for path in find_gn_in_workspace_for_scan(&analyzer.context().root) {
        let analyzer = analyzer.clone();
        let task = async move {
            analyzer.analyze_file(&path, start_time);
        };
        let task = if parallel {
            async move {
                tokio::spawn(task);
            }
            .boxed()
        } else {
            task.boxed()
        };
        tasks.push(task);
        count += 1;
    }

    join_all(tasks).await;

    let elapsed = start_time.elapsed();
    eprintln!(
        "Finished indexing {}: processed {} files in {:.1}s",
        analyzer.context().root.display(),
        count,
        elapsed.as_secs_f64()
    );
}
