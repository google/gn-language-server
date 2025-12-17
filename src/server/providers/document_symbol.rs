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

use either::Either;
use tower_lsp::lsp_types::{
    DocumentSymbol, DocumentSymbolParams, DocumentSymbolResponse, SymbolKind,
};

use crate::{
    common::{error::Result, utils::OwnedLineIndex},
    parser::{Node, Statement},
    server::{providers::utils::get_text_document_path, RequestContext},
};

fn compute_outline(node: &dyn Node<'_>, line_index: &OwnedLineIndex) -> Vec<DocumentSymbol> {
    let mut symbols = Vec::new();
    if let Some(statement) = node.as_statement() {
        match statement {
            Statement::Assignment(assignment) => {
                #[allow(deprecated)]
                symbols.push(DocumentSymbol {
                    name: format!(
                        "{} {} ...",
                        assignment.lvalue.span().as_str(),
                        assignment.op
                    ),
                    detail: None,
                    kind: SymbolKind::VARIABLE,
                    tags: None,
                    deprecated: None,
                    range: line_index.range(assignment.span()),
                    selection_range: line_index.range(assignment.lvalue.span()),
                    children: Some(compute_outline(assignment.rvalue.as_node(), line_index)),
                });
            }
            Statement::Call(call) => {
                if let Some(block) = &call.block {
                    let name = if call.args.is_empty() {
                        format!("{}()", call.function.name)
                    } else if let Some(string) =
                        call.only_arg().and_then(|arg| arg.as_primary_string())
                    {
                        format!("{}(\"{}\")", call.function.name, string.raw_value)
                    } else {
                        format!("{}(...)", call.function.name)
                    };
                    #[allow(deprecated)]
                    symbols.push(DocumentSymbol {
                        name,
                        detail: None,
                        kind: SymbolKind::FUNCTION,
                        tags: None,
                        deprecated: None,
                        range: line_index.range(call.span()),
                        selection_range: line_index.range(call.function.span()),
                        children: Some(compute_outline(block.as_node(), line_index)),
                    });
                }
            }
            Statement::Condition(top_condition) => {
                #[allow(deprecated)]
                let mut top_symbol = DocumentSymbol {
                    name: format!("if ({})", top_condition.condition.span().as_str()),
                    detail: None,
                    kind: SymbolKind::NAMESPACE,
                    tags: None,
                    deprecated: None,
                    range: line_index.range(top_condition.span()),
                    selection_range: line_index.range(top_condition.condition.span()),
                    children: Some(Vec::new()),
                };

                let mut current_condition = top_condition;
                let mut current_children = top_symbol.children.as_mut().unwrap();
                loop {
                    current_children.extend(compute_outline(
                        current_condition.then_block.as_node(),
                        line_index,
                    ));
                    match &current_condition.else_block {
                        None => break,
                        Some(Either::Left(next_condition)) => {
                            #[allow(deprecated)]
                            current_children.push(DocumentSymbol {
                                name: format!(
                                    "else if ({})",
                                    next_condition.condition.span().as_str()
                                ),
                                detail: None,
                                kind: SymbolKind::NAMESPACE,
                                tags: None,
                                deprecated: None,
                                range: line_index.range(next_condition.span()),
                                selection_range: line_index.range(next_condition.condition.span()),
                                children: Some(Vec::new()),
                            });
                            current_children = current_children
                                .last_mut()
                                .unwrap()
                                .children
                                .as_mut()
                                .unwrap();
                            current_condition = next_condition;
                        }
                        Some(Either::Right(else_block)) => {
                            #[allow(deprecated)]
                            current_children.push(DocumentSymbol {
                                name: "else".to_string(),
                                detail: None,
                                kind: SymbolKind::NAMESPACE,
                                tags: None,
                                deprecated: None,
                                range: line_index.range(else_block.span()),
                                selection_range: line_index.range(else_block.span()),
                                children: Some(compute_outline(else_block.as_node(), line_index)),
                            });
                            break;
                        }
                    }
                }

                symbols.push(top_symbol);
            }
            Statement::Error(_) => {}
        }
    } else {
        for child in node.children() {
            symbols.extend(compute_outline(child, line_index));
        }
    }
    symbols
}

pub async fn document_symbol(
    context: &RequestContext,
    params: DocumentSymbolParams,
) -> Result<Option<DocumentSymbolResponse>> {
    let path = get_text_document_path(&params.text_document)?;
    let current_file = context.analyzer.analyze_file(&path, context.request_time)?;

    Ok(Some(DocumentSymbolResponse::Nested(compute_outline(
        current_file.parsed_root.get(),
        &current_file.document.line_index,
    ))))
}
