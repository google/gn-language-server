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

use crate::util::LineIndex;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DocumentVersion {
    OnDisk { modified: SystemTime },
    InMemory { revision: i32 },
    Missing,
}

#[derive(Clone)]
pub struct Document {
    pub path: PathBuf,
    pub data: String,
    pub version: DocumentVersion,
    pub line_index: LineIndex<'static>,
}

impl Document {
    pub fn new(path: &Path, data: String, version: DocumentVersion) -> Arc<Self> {
        let line_index = LineIndex::new(&data);
        // SAFETY: line_index is backed by data, which is guaranteed to be valid
        // for the lifetime of Document as long as it's immutable.
        let line_index = unsafe { std::mem::transmute::<LineIndex, LineIndex>(line_index) };
        Arc::new(Self {
            path: path.to_path_buf(),
            data,
            version,
            line_index,
        })
    }

    pub fn empty(path: &Path) -> Arc<Self> {
        Self::new(path, String::new(), DocumentVersion::Missing)
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

    pub fn read_version(&self, path: &Path) -> std::io::Result<DocumentVersion> {
        if let Some(doc) = self.memory_docs.get(path) {
            return Ok(doc.version);
        }

        let metadata = fs_err::metadata(path)?;
        let modified = metadata.modified()?;
        Ok(DocumentVersion::OnDisk { modified })
    }

    pub fn read(&self, path: &Path) -> std::io::Result<Arc<Document>> {
        if let Some(doc) = self.memory_docs.get(path) {
            return Ok(doc.clone());
        }
        let data = fs_err::read_to_string(path)?;
        let version = self.read_version(path)?;
        Ok(Document::new(path, data, version))
    }

    pub fn load_to_memory(&mut self, path: &Path, data: &str, revision: i32) {
        self.memory_docs.insert(
            path.to_path_buf(),
            Document::new(
                path,
                data.to_string(),
                DocumentVersion::InMemory { revision },
            ),
        );
    }

    pub fn unload_from_memory(&mut self, path: &Path) {
        self.memory_docs.remove(path);
    }
}
