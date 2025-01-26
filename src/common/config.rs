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

use std::path::PathBuf;

use crate::analyzer::IndexingLevel;

fn default_true() -> bool {
    true
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Configurations {
    #[cfg(not(target_family = "wasm"))]
    pub binary_path: Option<PathBuf>,
    #[serde(default = "default_true")]
    pub background_indexing: bool,
    #[serde(default = "default_true")]
    pub error_reporting: bool,
    #[serde(default = "default_true")]
    pub target_lens: bool,
    #[serde(default = "default_true")]
    pub parallel_indexing: bool,
    #[serde(default = "default_true")]
    pub workspace_completion: bool,
    pub experimental: ExperimentalConfigurations,
}

impl Default for Configurations {
    fn default() -> Self {
        Self {
            #[cfg(not(target_family = "wasm"))]
            binary_path: None,
            background_indexing: true,
            error_reporting: true,
            target_lens: true,
            parallel_indexing: true,
            workspace_completion: true,
            experimental: Default::default(),
        }
    }
}

impl Configurations {
    pub fn indexing_level(&self) -> IndexingLevel {
        match self.background_indexing {
            false => IndexingLevel::Disabled,
            true => IndexingLevel::Enabled {
                parallel: self.parallel_indexing,
            },
        }
    }
}

#[derive(Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExperimentalConfigurations {}
