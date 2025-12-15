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

use tower_lsp::lsp_types::{Range, TextEdit};

use crate::analyzer::{AnalyzedFile, AnalyzedStatement};

fn get_import<'p>(statement: &AnalyzedStatement<'p>) -> Option<&'p str> {
    match statement {
        AnalyzedStatement::Import(import) => Some(import.name),
        _ => None,
    }
}

pub fn create_import_edit(current_file: &AnalyzedFile, import: &str) -> TextEdit {
    // Find the first top-level import block.
    let first_import_block: Vec<_> = current_file
        .analyzed_root
        .get()
        .statements
        .iter()
        .skip_while(|statement| get_import(statement).is_none())
        .take_while(|statement| get_import(statement).is_some())
        .collect();

    let (insert_offset, prefix, suffix) = if first_import_block.is_empty() {
        if let Some(first_statement) = current_file.analyzed_root.get().statements.first() {
            (first_statement.span().start(), "", "\n\n")
        } else {
            (current_file.document.data.len(), "", "\n\n")
        }
    } else if let Some(next_import) = first_import_block
        .iter()
        .copied()
        .find(|statement| get_import(statement).unwrap() > import)
    {
        (next_import.span().start(), "", "\n")
    } else {
        (first_import_block.last().unwrap().span().end(), "\n", "")
    };

    let insert_pos = current_file.document.line_index.position(insert_offset);

    TextEdit {
        range: Range::new(insert_pos, insert_pos),
        new_text: format!("{prefix}import(\"{import}\"){suffix}"),
    }
}
