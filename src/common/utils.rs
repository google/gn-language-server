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
    path::{Path, PathBuf},
    sync::Arc,
};

use self_cell::self_cell;

use pest::Span;
use tokio::sync::SetOnce;
use tower_lsp::lsp_types::{Position, Range};
use walkdir::{DirEntry, WalkDir};

pub fn is_exported(name: &str) -> bool {
    !name.starts_with("_")
}

pub fn format_path(path: &Path, workspace_root: &Path) -> String {
    if let Ok(relative_path) = path.strip_prefix(workspace_root) {
        format!("//{}", relative_path.to_string_lossy())
    } else {
        path.to_string_lossy().to_string()
    }
}

fn filter_source_entry(entry: &DirEntry) -> bool {
    // Drop dot files.
    if entry
        .file_name()
        .to_str()
        .is_some_and(|name| name.starts_with('.'))
    {
        return false;
    }
    // Drop output directories.
    if entry.file_type().is_dir() && entry.path().join("args.gn").exists() {
        return false;
    }
    true
}

pub fn walk_source_dirs(root: &Path) -> impl Iterator<Item = PathBuf> {
    WalkDir::new(root)
        .into_iter()
        .filter_entry(filter_source_entry)
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.into_path())
}

pub fn is_good_for_scan(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            name.ends_with(".gni") || name == "BUILD.gn" || name == "BUILDCONFIG.gn"
        })
}

pub fn find_gn_in_workspace_for_scan(workspace_root: &Path) -> impl Iterator<Item = PathBuf> {
    walk_source_dirs(workspace_root).filter(|path| is_good_for_scan(path))
}

#[derive(Clone, Debug)]
pub struct LineIndex<'i> {
    input: &'i str,
    lines: Vec<&'i str>,
}

impl<'i> LineIndex<'i> {
    pub fn new(input: &'i str) -> Self {
        let mut lines: Vec<&str> = input.split_inclusive('\n').collect();
        if input.is_empty() {
            lines.push(input);
        }
        if input.ends_with('\n') {
            lines.push(&input[input.len()..]);
        }
        Self { input, lines }
    }

    fn str_offset(&self, s: &str) -> usize {
        // SAFETY: s must be in the same string as input.
        unsafe { s.as_ptr().offset_from(self.input.as_ptr()) as usize }
    }

    pub fn position(&self, offset: usize) -> Position {
        let index = self
            .lines
            .binary_search_by_key(&offset, |line| self.str_offset(line))
            .unwrap_or_else(|index| index - 1);
        let line = self.lines[index];
        let bytes = offset - self.str_offset(line);
        let character = line
            .get(..bytes)
            .map(|s| s.encode_utf16().count())
            .unwrap_or(0);
        Position {
            line: index as u32,
            character: character as u32,
        }
    }

    pub fn range(&self, span: Span) -> Range {
        Range {
            start: self.position(span.start()),
            end: self.position(span.end()),
        }
    }

    pub fn offset(&self, position: Position) -> Option<usize> {
        let line = self.lines.get(position.line as usize)?;
        let mut character = 0;
        for (i, ch) in line.char_indices() {
            if character >= position.character as usize {
                return Some(self.str_offset(line) + i);
            }
            let mut buf = [0; 2];
            character += ch.encode_utf16(&mut buf).len();
        }
        if character >= position.character as usize {
            Some(self.str_offset(line) + line.len())
        } else {
            None
        }
    }
}

self_cell!(
    struct LineIndexCell {
        owner: Arc<String>,
        #[covariant]
        dependent: LineIndex,
    }
    impl {Debug}
);

#[derive(Clone, Debug)]
pub struct OwnedLineIndex(Arc<LineIndexCell>);

impl OwnedLineIndex {
    pub fn new(input: Arc<String>) -> Self {
        Self(Arc::new(LineIndexCell::new(input, |input| {
            LineIndex::new(input.as_str())
        })))
    }

    pub fn position(&self, offset: usize) -> Position {
        self.0.borrow_dependent().position(offset)
    }

    pub fn range(&self, span: Span) -> Range {
        self.0.borrow_dependent().range(span)
    }

    pub fn offset(&self, position: Position) -> Option<usize> {
        self.0.borrow_dependent().offset(position)
    }
}

pub fn parse_simple_literal(s: &str) -> Option<&str> {
    if s.contains(['\\', '$']) {
        None
    } else {
        Some(s)
    }
}

#[derive(Clone, Default)]
pub struct AsyncSignal {
    done: Arc<SetOnce<()>>,
}

impl AsyncSignal {
    pub fn new() -> Self {
        Default::default()
    }

    pub fn done(&self) -> bool {
        self.done.initialized()
    }

    pub async fn wait(&self) {
        self.done.wait().await;
    }

    pub fn set(&mut self) -> bool {
        self.done.set(()).is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_index() {
        let input = "\n\nfoo\n\n";
        let index = LineIndex::new(input);

        assert_eq!(index.position(0), Position::new(0, 0));
        assert_eq!(index.position(1), Position::new(1, 0));
        assert_eq!(index.position(2), Position::new(2, 0));
        assert_eq!(index.position(3), Position::new(2, 1));
        assert_eq!(index.position(4), Position::new(2, 2));
        assert_eq!(index.position(5), Position::new(2, 3));
        assert_eq!(index.position(6), Position::new(3, 0));
        assert_eq!(index.position(7), Position::new(4, 0));

        assert_eq!(index.offset(Position::new(0, 0)), Some(0));
        assert_eq!(index.offset(Position::new(1, 0)), Some(1));
        assert_eq!(index.offset(Position::new(2, 0)), Some(2));
        assert_eq!(index.offset(Position::new(2, 1)), Some(3));
        assert_eq!(index.offset(Position::new(2, 2)), Some(4));
        assert_eq!(index.offset(Position::new(2, 3)), Some(5));
        assert_eq!(index.offset(Position::new(3, 0)), Some(6));
        assert_eq!(index.offset(Position::new(4, 0)), Some(7));
        assert_eq!(index.offset(Position::new(4, 1)), None);
        assert_eq!(index.offset(Position::new(5, 0)), None);
    }

    #[test]
    fn line_index_no_last_newline() {
        let input = "\n\nfoo";
        let index = LineIndex::new(input);

        assert_eq!(index.position(0), Position::new(0, 0));
        assert_eq!(index.position(1), Position::new(1, 0));
        assert_eq!(index.position(2), Position::new(2, 0));
        assert_eq!(index.position(3), Position::new(2, 1));
        assert_eq!(index.position(4), Position::new(2, 2));
        assert_eq!(index.position(5), Position::new(2, 3));

        assert_eq!(index.offset(Position::new(0, 0)), Some(0));
        assert_eq!(index.offset(Position::new(1, 0)), Some(1));
        assert_eq!(index.offset(Position::new(2, 0)), Some(2));
        assert_eq!(index.offset(Position::new(2, 1)), Some(3));
        assert_eq!(index.offset(Position::new(2, 2)), Some(4));
        assert_eq!(index.offset(Position::new(2, 3)), Some(5));
        assert_eq!(index.offset(Position::new(3, 0)), None);
    }

    #[test]
    fn line_index_empty() {
        let input = "";
        let index = LineIndex::new(input);

        assert_eq!(index.position(0), Position::new(0, 0));

        assert_eq!(index.offset(Position::new(0, 0)), Some(0));
        assert_eq!(index.offset(Position::new(1, 0)), None);
        assert_eq!(index.offset(Position::new(0, 1)), None);
    }
}
