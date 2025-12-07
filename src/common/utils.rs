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

use pest::Span;
use tokio::sync::SetOnce;
use tower_lsp::lsp_types::{Position, Range};
use walkdir::WalkDir;

pub fn is_exported(name: &str) -> bool {
    !name.starts_with("_")
}

pub fn walk_source_dirs(root: &Path) -> impl Iterator<Item = PathBuf> {
    WalkDir::new(root)
        .into_iter()
        .filter_entry(|entry| {
            !(entry.file_type().is_dir() && entry.path().join("args.gn").exists())
        })
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.into_path())
}

pub fn find_gn_files(root: &Path) -> impl Iterator<Item = PathBuf> {
    walk_source_dirs(root).filter(|path| {
        path.to_str()
            .is_some_and(|name| name.ends_with(".gn") || name.ends_with(".gni"))
    })
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
