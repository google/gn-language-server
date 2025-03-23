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
    io::ErrorKind,
    path::{Path, PathBuf},
    pin::Pin,
    sync::{Arc, RwLock},
    time::{Duration, Instant, SystemTime},
};

use crate::util::LineIndex;

const CHECK_INTERVAL: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DocumentVersion {
    OnDisk { modified: SystemTime },
    InMemory { revision: i32 },
    Missing,
}

pub struct Document {
    pub path: PathBuf,
    pub data: Pin<String>,
    pub version: DocumentVersion,
    pub last_checked: RwLock<Instant>,
    pub line_index: LineIndex<'static>,
}

impl Document {
    pub fn new(path: &Path, data: String, version: DocumentVersion) -> Self {
        let data = Pin::new(data);
        let line_index = LineIndex::new(&data);
        // SAFETY: line_index is backed by pinned data.
        let line_index = unsafe { std::mem::transmute::<LineIndex, LineIndex>(line_index) };
        Self {
            path: path.to_path_buf(),
            data,
            version,
            last_checked: RwLock::new(Instant::now()),
            line_index,
        }
    }

    pub fn empty(path: &Path) -> Self {
        Self::new(path, String::new(), DocumentVersion::Missing)
    }

    pub fn need_check(&self) -> bool {
        if matches!(self.version, DocumentVersion::OnDisk { .. }) {
            self.last_checked.read().unwrap().elapsed() >= CHECK_INTERVAL
        } else {
            // In-memory documents are cheap to check.
            true
        }
    }

    pub fn will_check(&self) -> bool {
        if self.need_check() {
            *self.last_checked.write().unwrap() = Instant::now();
            true
        } else {
            false
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
    memory_docs: BTreeMap<PathBuf, Pin<Arc<Document>>>,
}

impl DocumentStorage {
    pub fn new() -> Self {
        Default::default()
    }

    pub fn read_version(&self, path: &Path) -> std::io::Result<DocumentVersion> {
        if let Some(doc) = self.memory_docs.get(path) {
            return Ok(doc.version);
        }

        let metadata = match fs_err::metadata(path) {
            Err(err) if err.kind() == ErrorKind::NotFound => return Ok(DocumentVersion::Missing),
            other => other?,
        };
        let modified = metadata.modified()?;
        Ok(DocumentVersion::OnDisk { modified })
    }

    pub fn read(&self, path: &Path) -> std::io::Result<Pin<Arc<Document>>> {
        if let Some(doc) = self.memory_docs.get(path) {
            return Ok(doc.clone());
        }
        // Read the version first to be pesimistic about file changes.
        let version = self.read_version(path)?;
        let data = fs_err::read_to_string(path)?;
        Ok(Arc::pin(Document::new(path, data, version)))
    }

    pub fn load_to_memory(&mut self, path: &Path, data: &str, revision: i32) {
        self.memory_docs.insert(
            path.to_path_buf(),
            Arc::pin(Document::new(
                path,
                data.to_string(),
                DocumentVersion::InMemory { revision },
            )),
        );
    }

    pub fn unload_from_memory(&mut self, path: &Path) {
        self.memory_docs.remove(path);
    }

    pub fn memory_docs(&self) -> Vec<Pin<Arc<Document>>> {
        self.memory_docs.values().cloned().collect()
    }
}
