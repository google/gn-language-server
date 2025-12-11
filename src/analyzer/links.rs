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
    collections::HashMap,
    path::{Path, PathBuf},
};

use crate::{
    analyzer::{data::AnalyzedLink, WorkspaceContext},
    common::utils::parse_simple_literal,
    parser::{Block, Node},
};

#[allow(clippy::manual_map)]
fn resolve_target<'s>(
    label: &'s str,
    current_path: &Path,
    workspace: &WorkspaceContext,
) -> Option<(PathBuf, &'s str)> {
    if let Some((prefix, name)) = label.split_once(':') {
        if let Some(rel_dir) = prefix.strip_prefix("//") {
            Some((workspace.root.join(rel_dir).join("BUILD.gn"), name))
        } else {
            let build_path = current_path.parent().unwrap().join(prefix).join("BUILD.gn");
            build_path.exists().then_some((build_path, name))
        }
    } else if let Some(rel_dir) = label.strip_prefix("//") {
        if !rel_dir.is_empty() {
            Some((
                workspace.root.join(rel_dir).join("BUILD.gn"),
                rel_dir.split('/').next_back().unwrap(),
            ))
        } else {
            None
        }
    } else {
        None
    }
}

pub fn collect_links<'i>(
    ast: &Block<'i>,
    path: &Path,
    workspace: &WorkspaceContext,
) -> HashMap<PathBuf, Vec<AnalyzedLink<'i>>> {
    let links = ast.strings().filter_map(|string| {
        let content = parse_simple_literal(string.raw_value)?;
        if !content.contains(":") && content.contains(".") {
            let path = workspace.resolve_path(content, path.parent().unwrap());
            if let Ok(true) = path.try_exists() {
                return Some(AnalyzedLink::File {
                    path: path.to_path_buf(),
                    span: string.span,
                });
            }
        } else if let Some((build_gn_path, name)) = resolve_target(content, path, workspace) {
            return Some(AnalyzedLink::Target {
                path: build_gn_path,
                name,
                span: string.span,
            });
        }
        None
    });

    let mut links_map: HashMap<PathBuf, Vec<AnalyzedLink>> = HashMap::new();
    for link in links {
        links_map
            .entry(link.path().to_path_buf())
            .or_default()
            .push(link);
    }

    links_map
}
