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

use std::path::{Path, PathBuf};

pub fn find_nearest_workspace_root(file: &Path) -> Option<&Path> {
    file.ancestors().find(|&dir| dir.join(".gn").exists())
}

#[derive(Clone, Debug)]
pub struct WorkspaceFinder {
    main_workspace_root: Option<PathBuf>,
}

impl WorkspaceFinder {
    pub fn new(client_root: Option<&Path>) -> Self {
        let main_workspace_root = client_root.and_then(find_nearest_workspace_root);
        Self {
            main_workspace_root: main_workspace_root.map(|path| path.to_path_buf()),
        }
    }

    pub fn find_for<'p>(&self, path: &'p Path) -> Option<&'p Path> {
        if let Some(main_workspace_root) = &self.main_workspace_root {
            if let Some(dir) = path.ancestors().find(|dir| dir == main_workspace_root) {
                return Some(dir);
            }
        }
        find_nearest_workspace_root(path)
    }
}
