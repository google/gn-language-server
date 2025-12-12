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
    collections::BTreeMap,
    path::{Path, PathBuf},
    pin::Pin,
    sync::{Arc, Mutex, RwLock},
    time::Instant,
};

pub use data::{
    AnalyzedAssignment, AnalyzedBlock, AnalyzedFile, AnalyzedImport, AnalyzedLink,
    AnalyzedStatement, AnalyzedTarget, AnalyzedTemplate, Environment, Target, Template, Variable,
    VariableMap,
};

pub use toplevel::TopLevelStatementsExt;

use crate::{
    analyzer::{analyzer::Analyzer, data::WorkspaceContext, dotgn::evaluate_dot_gn},
    common::{
        error::{Error, Result},
        storage::DocumentStorage,
        workspace::WorkspaceFinder,
    },
};

mod analyzer;
mod cache;
mod data;
mod dotgn;
mod links;
mod symbols;
mod tests;
mod toplevel;
mod utils;

pub struct AnalyzerSet {
    storage: Arc<Mutex<DocumentStorage>>,
    finder: WorkspaceFinder,
    analyzers: RwLock<BTreeMap<PathBuf, Arc<Mutex<Analyzer>>>>,
}

impl AnalyzerSet {
    pub fn new(storage: &Arc<Mutex<DocumentStorage>>, finder: WorkspaceFinder) -> Self {
        Self {
            storage: storage.clone(),
            finder,
            analyzers: Default::default(),
        }
    }

    pub fn analyze_file(
        &self,
        path: &Path,
        request_time: Instant,
    ) -> Result<Pin<Arc<AnalyzedFile>>> {
        Ok(self
            .get_for(path)?
            .lock()
            .unwrap()
            .analyze_file(path, request_time))
    }

    pub fn analyze_environment(
        &self,
        file: &Pin<Arc<AnalyzedFile>>,
        pos: usize,
        request_time: Instant,
    ) -> Result<Environment> {
        Ok(self
            .get_for(&file.document.path)?
            .lock()
            .unwrap()
            .analyze_environment(file, pos, request_time))
    }

    pub fn cached_files(&self, workspace_root: &Path) -> Vec<Pin<Arc<AnalyzedFile>>> {
        let Some(analyzer) = self.analyzers.read().unwrap().get(workspace_root).cloned() else {
            return Vec::new();
        };
        let cached_files = analyzer.lock().unwrap().cached_files();
        cached_files
    }

    pub fn workspace_roots(&self) -> Vec<PathBuf> {
        self.analyzers.read().unwrap().keys().cloned().collect()
    }

    pub fn finder(&self) -> &WorkspaceFinder {
        &self.finder
    }

    pub fn get_for(&self, path: &Path) -> Result<Arc<Mutex<Analyzer>>> {
        if !path.is_absolute() {
            return Err(Error::General("Path must be absolute".to_string()));
        }

        let workspace_root = self
            .finder
            .find_for(path)
            .ok_or(Error::General("Workspace not found".to_string()))?;
        let dot_gn_path = workspace_root.join(".gn");
        let dot_gn_version = {
            let storage = self.storage.lock().unwrap();
            storage.read_version(&dot_gn_path)
        };

        {
            let read_lock = self.analyzers.read().unwrap();
            if let Some(analyzer) = read_lock.get(workspace_root) {
                if analyzer.lock().unwrap().context().dot_gn_version == dot_gn_version {
                    return Ok(analyzer.clone());
                }
            }
        }

        let build_config = {
            let storage = self.storage.lock().unwrap();
            let document = storage.read(&dot_gn_path);
            evaluate_dot_gn(workspace_root, &document.data)?
        };

        let context = WorkspaceContext {
            root: workspace_root.to_path_buf(),
            dot_gn_version,
            build_config,
        };

        let analyzer = Arc::new(Mutex::new(Analyzer::new(&context, &self.storage)));

        let mut write_lock = self.analyzers.write().unwrap();
        Ok(write_lock
            .entry(workspace_root.to_path_buf())
            .or_insert(analyzer)
            .clone())
    }
}
