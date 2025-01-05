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

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DocumentVersion {
    OnDisk { modified: SystemTime },
    InMemory { revision: i32 },
}

#[derive(Clone)]
pub struct Document {
    pub path: PathBuf,
    pub data: Vec<u8>,
    pub version: DocumentVersion,
}

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
        let data = fs_err::read(path)?;
        let version = self.read_version(path)?;
        Ok(Arc::new(Document {
            path: path.to_path_buf(),
            data,
            version,
        }))
    }

    pub fn load_to_memory(&mut self, path: &Path, data: &[u8], revision: i32) {
        self.memory_docs.insert(
            path.to_path_buf(),
            Arc::new(Document {
                path: path.to_path_buf(),
                data: data.to_vec(),
                version: DocumentVersion::InMemory { revision },
            }),
        );
    }

    pub fn unload_from_memory(&mut self, path: &Path) {
        self.memory_docs.remove(path);
    }
}
