use std::collections::HashMap;

use tower_lsp::lsp_types::{Range, TextEdit, Url, WorkspaceEdit};

use crate::analyzer::{AnalyzedFile, AnalyzedStatement};

fn get_import<'p>(statement: &AnalyzedStatement<'p>) -> Option<&'p str> {
    match statement {
        AnalyzedStatement::Import(import) => Some(import.name),
        _ => None,
    }
}

pub fn create_import_edit(current_file: &AnalyzedFile, import: &str) -> WorkspaceEdit {
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

    WorkspaceEdit {
        changes: Some(HashMap::from([(
            Url::from_file_path(&current_file.document.path).unwrap(),
            vec![TextEdit {
                range: Range::new(insert_pos, insert_pos),
                new_text: format!("{prefix}import(\"{import}\"){suffix}"),
            }],
        )])),
        ..Default::default()
    }
}
