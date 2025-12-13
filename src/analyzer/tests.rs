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

#![cfg(test)]

use std::{
    sync::{Arc, Mutex},
    time::Instant,
};

use crate::{
    analyzer::Analyzer,
    common::{storage::DocumentStorage, testutils::testdata, workspace::WorkspaceFinder},
    parser::Statement,
};

#[test]
fn test_analyze_smoke() {
    let storage = Arc::new(Mutex::new(DocumentStorage::new()));
    let analyzer = Analyzer::new(&storage, WorkspaceFinder::new(None));

    let file = analyzer
        .analyze_file(&testdata("workspaces/smoke/BUILD.gn"), Instant::now())
        .unwrap();

    // No parse error.
    assert!(file
        .ast
        .statements
        .iter()
        .all(|s| !matches!(s, Statement::Error(_))));

    // Inspect the environment.
    let environment = analyzer.analyze_at(&file, 0, Instant::now()).unwrap();
    assert!(environment.variables.contains_key("enable_opt"));
    assert!(environment.variables.contains_key("_lib"));
    assert!(environment.variables.contains_key("is_linux"));
}

#[test]
fn test_analyze_cycles() {
    let request_time = Instant::now();
    let storage = Arc::new(Mutex::new(DocumentStorage::new()));
    let analyzer = Analyzer::new(&storage, WorkspaceFinder::new(None));

    assert!(analyzer
        .analyze_file(&testdata("workspaces/cycles/ok1.gni"), request_time)
        .is_ok());
    assert!(analyzer
        .analyze_file(&testdata("workspaces/cycles/bad1.gni"), request_time)
        .is_ok());
}
