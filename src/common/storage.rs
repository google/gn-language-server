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
    sync::Arc,
    time::SystemTime,
};

use crate::common::utils::OwnedLineIndex;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DocumentVersion {
    OnDisk { modified: SystemTime },
    InMemory { revision: i32 },
    IoError,
}

impl DocumentVersion {
    pub fn is_error(self) -> bool {
        match self {
            DocumentVersion::IoError => true,
            DocumentVersion::OnDisk { .. } | DocumentVersion::InMemory { .. } => false,
        }
    }
}

#[derive(Debug)]
pub struct Document {
    pub path: PathBuf,
    pub data: Arc<String>,
    pub version: DocumentVersion,
    pub line_index: OwnedLineIndex,
}

impl Document {
    pub fn new(path: &Path, data: String, version: DocumentVersion) -> Self {
        let data = Arc::new(data);
        let line_index = OwnedLineIndex::new(data.clone());
        Self {
            path: path.to_path_buf(),
            data,
            version,
            line_index,
        }
    }
}

impl std::hash::Hash for Document {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.path.hash(state);
        self.data.hash(state);
        // Skip LineIndex as it's derived from data.
        self.version.hash(state);
    }
}

impl PartialEq for Document {
    fn eq(&self, other: &Self) -> bool {
        self.path == other.path && self.data == other.data && self.version == other.version
    }
}

impl Eq for Document {}

#[derive(Default)]
pub struct DocumentStorage {
    memory_docs: BTreeMap<PathBuf, Arc<Document>>,
}

impl DocumentStorage {
    pub fn new() -> Self {
        Default::default()
    }

    pub fn read_version(&self, path: &Path) -> DocumentVersion {
        if let Some(doc) = self.memory_docs.get(path) {
            return doc.version;
        }

        let modified = match std::fs::metadata(path).and_then(|metadata| metadata.modified()) {
            Ok(modified) => modified,
            Err(_) => return DocumentVersion::IoError,
        };
        DocumentVersion::OnDisk { modified }
    }

    pub fn read(&self, path: &Path) -> Arc<Document> {
        if let Some(doc) = self.memory_docs.get(path) {
            return doc.clone();
        }
        // Read the version first to be pesimistic about file changes.
        let version = self.read_version(path);
        let data = std::fs::read_to_string(path).unwrap_or_default();
        Arc::new(Document::new(path, data, version))
    }

    pub fn load_to_memory(&mut self, path: &Path, data: &str, revision: i32) {
        self.memory_docs.insert(
            path.to_path_buf(),
            Arc::new(Document::new(
                path,
                data.to_string(),
                DocumentVersion::InMemory { revision },
            )),
        );
    }

    pub fn unload_from_memory(&mut self, path: &Path) {
        self.memory_docs.remove(path);
    }

    pub fn memory_docs(&self) -> Vec<Arc<Document>> {
        self.memory_docs.values().cloned().collect()
    }
}
