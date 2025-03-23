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
    sync::{Arc, Mutex},
};

use dotgn::evaluate_dot_gn;
use either::Either;
use pest::Span;
use shallow::ShallowAnalyzer;
use tower_lsp::lsp_types::{DocumentSymbol, SymbolKind};

use crate::{
    ast::{parse, Block, Comments, Expr, LValue, Node, PrimaryExpr, Statement},
    builtins::{DECLARE_ARGS, FOREACH, FORWARD_VARIABLES_FROM, IMPORT, SET_DEFAULTS, TEMPLATE},
    storage::{Document, DocumentStorage, DocumentVersion},
    util::{parse_simple_literal, LineIndex},
};

pub use base::{
    find_workspace_root, AnalyzedAssignment, AnalyzedBlock, AnalyzedEvent, AnalyzedFile,
    AnalyzedImport, AnalyzedTarget, AnalyzedTemplate, Link, ShallowAnalyzedFile, WorkspaceContext,
};

mod base;
mod dotgn;
mod shallow;
mod tests;

#[allow(clippy::manual_map)]
fn resolve_target<'s>(
    label: &'s str,
    current_path: &Path,
    workspace: &WorkspaceContext,
) -> Option<(PathBuf, &'s str)> {
    if let Some((prefix, name)) = label.split_once(':') {
        if prefix.is_empty() {
            Some((current_path.to_path_buf(), name))
        } else if let Some(rel_dir) = prefix.strip_prefix("//") {
            Some((workspace.root.join(rel_dir).join("BUILD.gn"), name))
        } else {
            None
        }
    } else if let Some(rel_dir) = label.strip_prefix("//") {
        if !rel_dir.is_empty() {
            Some((
                workspace.root.join(rel_dir).join("BUILD.gn"),
                rel_dir.split('/').last().unwrap(),
            ))
        } else {
            None
        }
    } else {
        None
    }
}

fn collect_links<'i>(
    ast_root: &Block<'i>,
    path: &Path,
    workspace: &WorkspaceContext,
) -> Vec<Link<'i>> {
    ast_root
        .strings()
        .filter_map(|string| {
            let content = parse_simple_literal(string.raw_value)?;
            if !content.contains(":") && content.contains(".") {
                let path = workspace.resolve_path(content, path.parent().unwrap());
                if let Ok(true) = path.try_exists() {
                    return Some(Link::File {
                        path: path.to_path_buf(),
                        span: string.span,
                    });
                }
            } else if let Some((build_gn_path, name)) = resolve_target(content, path, workspace) {
                return Some(Link::Target {
                    path: build_gn_path,
                    name,
                    span: string.span,
                });
            }
            None
        })
        .collect()
}

#[allow(deprecated)]
fn collect_symbols(node: &dyn Node, line_index: &LineIndex) -> Vec<DocumentSymbol> {
    let mut symbols = Vec::new();
    if let Some(statement) = node.as_statement() {
        match statement {
            Statement::Assignment(assignment) => {
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
                    children: Some(collect_symbols(assignment.rvalue.as_node(), line_index)),
                });
            }
            Statement::Call(call) => {
                if let Some(block) = &call.block {
                    symbols.push(DocumentSymbol {
                        name: call.function.name.to_string(),
                        detail: None,
                        kind: SymbolKind::FUNCTION,
                        tags: None,
                        deprecated: None,
                        range: line_index.range(call.span()),
                        selection_range: line_index.range(call.function.span()),
                        children: Some(collect_symbols(block.as_node(), line_index)),
                    });
                }
            }
            Statement::Condition(top_condition) => {
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
                    current_children.extend(collect_symbols(
                        current_condition.then_block.as_node(),
                        line_index,
                    ));
                    match &current_condition.else_block {
                        None => break,
                        Some(Either::Left(next_condition)) => {
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
                            current_children.push(DocumentSymbol {
                                name: "else".to_string(),
                                detail: None,
                                kind: SymbolKind::NAMESPACE,
                                tags: None,
                                deprecated: None,
                                range: line_index.range(else_block.span()),
                                selection_range: line_index.range(else_block.span()),
                                children: Some(collect_symbols(else_block.as_node(), line_index)),
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
            symbols.extend(collect_symbols(child, line_index));
        }
    }
    symbols
}

struct WorkspaceCache {
    dot_gn_version: DocumentVersion,
    context: WorkspaceContext,
    files: BTreeMap<PathBuf, Pin<Arc<AnalyzedFile>>>,
}

pub struct Analyzer {
    shallow_analyzer: ShallowAnalyzer,
    storage: Arc<Mutex<DocumentStorage>>,
    cache: BTreeMap<PathBuf, WorkspaceCache>,
}

impl Analyzer {
    pub fn new(storage: &Arc<Mutex<DocumentStorage>>) -> Self {
        Self {
            storage: storage.clone(),
            shallow_analyzer: ShallowAnalyzer::new(storage),
            cache: BTreeMap::new(),
        }
    }

    pub fn analyze(&mut self, path: &Path) -> std::io::Result<Pin<Arc<AnalyzedFile>>> {
        if !path.is_absolute() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Path must be absolute",
            ));
        }
        self.analyze_cached(path)
    }

    fn workspace_cache_for(&mut self, path: &Path) -> std::io::Result<&mut WorkspaceCache> {
        let workspace_root = find_workspace_root(path)?;
        let dot_gn_path = workspace_root.join(".gn");
        let dot_gn_version = {
            let storage = self.storage.lock().unwrap();
            storage.read_version(&dot_gn_path)?
        };

        let cache_hit = self
            .cache
            .get(workspace_root)
            .is_some_and(|workspace_cache| workspace_cache.dot_gn_version == dot_gn_version);
        if cache_hit {
            return Ok(self.cache.get_mut(workspace_root).unwrap());
        }

        let build_config = {
            let storage = self.storage.lock().unwrap();
            let document = storage.read(&dot_gn_path)?;
            evaluate_dot_gn(workspace_root, &document.data)?
        };

        let context = WorkspaceContext {
            root: workspace_root.to_path_buf(),
            build_config,
        };

        let workspace_cache = WorkspaceCache {
            dot_gn_version,
            context,
            files: BTreeMap::new(),
        };
        Ok(self
            .cache
            .entry(workspace_root.to_path_buf())
            .or_insert(workspace_cache))
    }

    fn analyze_cached(&mut self, path: &Path) -> std::io::Result<Pin<Arc<AnalyzedFile>>> {
        let (cached_file, context) = {
            let workspace_cache = self.workspace_cache_for(path)?;
            (
                workspace_cache.files.get(path).cloned(),
                workspace_cache.context.clone(),
            )
        };
        if let Some(cached_file) = cached_file {
            let storage = self.storage.lock().unwrap();
            if cached_file.is_fresh(&storage)? {
                return Ok(cached_file);
            }
        }

        let new_file = self.analyze_uncached(path, &context)?;
        self.workspace_cache_for(path)?
            .files
            .insert(path.to_path_buf(), new_file.clone());
        Ok(new_file)
    }

    fn analyze_uncached(
        &mut self,
        path: &Path,
        workspace: &WorkspaceContext,
    ) -> std::io::Result<Pin<Arc<AnalyzedFile>>> {
        let document = self.storage.lock().unwrap().read(path)?;
        let ast_root = Box::pin(parse(&document.data));

        let mut deps = Vec::new();
        let mut analyzed_root = self.analyze_block(&ast_root, workspace, &document, &mut deps)?;

        // Insert a synthetic import of BUILDCONFIG.gn.
        let dot_gn_file = self
            .shallow_analyzer
            .analyze(&workspace.build_config, workspace)?;
        analyzed_root.events.insert(
            0,
            AnalyzedEvent::Import(AnalyzedImport {
                file: dot_gn_file.clone(),
                span: Span::new(&document.data, 0, 0).unwrap(),
            }),
        );
        deps.push(dot_gn_file);

        let links = collect_links(&ast_root, path, workspace);
        let symbols = collect_symbols(ast_root.as_node(), &document.line_index);

        // SAFETY: links' contents are backed by pinned document.
        let links = unsafe { std::mem::transmute::<Vec<Link>, Vec<Link>>(links) };
        // SAFETY: analyzed_root's contents are backed by pinned document and pinned ast_root.
        let analyzed_root =
            unsafe { std::mem::transmute::<AnalyzedBlock, AnalyzedBlock>(analyzed_root) };
        // SAFETY: ast_root's contents are backed by pinned document.
        let ast_root = unsafe { std::mem::transmute::<Pin<Box<Block>>, Pin<Box<Block>>>(ast_root) };

        Ok(Arc::pin(AnalyzedFile {
            document,
            workspace: workspace.clone(),
            ast_root,
            analyzed_root,
            deps,
            links,
            symbols,
        }))
    }

    fn analyze_block<'i, 'p>(
        &mut self,
        block: &'p Block<'i>,
        workspace: &WorkspaceContext,
        document: &'i Document,
        deps: &mut Vec<Pin<Arc<ShallowAnalyzedFile>>>,
    ) -> std::io::Result<AnalyzedBlock<'i, 'p>> {
        let events: Vec<AnalyzedEvent> = block
            .statements
            .iter()
            .map(|statement| -> std::io::Result<Vec<AnalyzedEvent>> {
                match statement {
                    Statement::Assignment(assignment) => {
                        let mut events = Vec::new();
                        let identifier = match &assignment.lvalue {
                            LValue::Identifier(identifier) => identifier,
                            LValue::ArrayAccess(array_access) => &array_access.array,
                            LValue::ScopeAccess(scope_access) => &scope_access.scope,
                        };
                        events.push(AnalyzedEvent::Assignment(AnalyzedAssignment {
                            name: identifier.name,
                            comments: assignment.comments.clone(),
                            statement,
                            document,
                            variable_span: identifier.span,
                        }));
                        events.extend(self.analyze_expr(
                            &assignment.rvalue,
                            workspace,
                            document,
                            deps,
                        )?);
                        Ok(events)
                    }
                    Statement::Call(call) => {
                        match call.function.name {
                            IMPORT => {
                                if let Some(name) = call
                                    .only_arg()
                                    .and_then(|expr| expr.as_primary_string())
                                    .and_then(|s| parse_simple_literal(s.raw_value))
                                {
                                    let path = workspace
                                        .resolve_path(name, document.path.parent().unwrap());
                                    let file = match self.shallow_analyzer.analyze(&path, workspace)
                                    {
                                        Err(err) if err.kind() == ErrorKind::NotFound => {
                                            // Ignore missing imports as they might be imported conditionally.
                                            ShallowAnalyzedFile::empty(&path, workspace)
                                        }
                                        other => other?,
                                    };
                                    deps.push(file.clone());
                                    Ok(vec![AnalyzedEvent::Import(AnalyzedImport {
                                        file,
                                        span: call.span(),
                                    })])
                                } else {
                                    Ok(Vec::new())
                                }
                            }
                            TEMPLATE => {
                                let mut events = Vec::new();
                                if let Some(name) = call
                                    .only_arg()
                                    .and_then(|expr| expr.as_primary_string())
                                    .and_then(|s| parse_simple_literal(s.raw_value))
                                {
                                    events.push(AnalyzedEvent::Template(AnalyzedTemplate {
                                        name,
                                        comments: call.comments.clone(),
                                        document,
                                        header: call.function.span,
                                        span: call.span,
                                    }));
                                }
                                if let Some(block) = &call.block {
                                    events.push(AnalyzedEvent::NewScope(
                                        self.analyze_block(block, workspace, document, deps)?,
                                    ));
                                }
                                Ok(events)
                            }
                            DECLARE_ARGS => {
                                if let Some(block) = &call.block {
                                    let analyzed_root =
                                        self.analyze_block(block, workspace, document, deps)?;
                                    Ok(vec![AnalyzedEvent::DeclareArgs(analyzed_root)])
                                } else {
                                    Ok(Vec::new())
                                }
                            }
                            FOREACH => {
                                if let Some(block) = &call.block {
                                    Ok(self.analyze_block(block, workspace, document, deps)?.events)
                                } else {
                                    Ok(Vec::new())
                                }
                            }
                            SET_DEFAULTS => {
                                if let Some(block) = &call.block {
                                    let analyzed_root =
                                        self.analyze_block(block, workspace, document, deps)?;
                                    Ok(vec![AnalyzedEvent::NewScope(analyzed_root)])
                                } else {
                                    Ok(Vec::new())
                                }
                            }
                            FORWARD_VARIABLES_FROM => {
                                if let Some(strings) = call
                                    .args
                                    .get(1)
                                    .and_then(|expr| expr.as_primary_list())
                                    .map(|list| {
                                        list.values
                                            .iter()
                                            .filter_map(|expr| expr.as_primary_string())
                                            .collect::<Vec<_>>()
                                    })
                                {
                                    return Ok(strings
                                        .into_iter()
                                        .filter_map(|string| {
                                            parse_simple_literal(string.raw_value).map(|name| {
                                                AnalyzedEvent::Assignment(AnalyzedAssignment {
                                                    name,
                                                    comments: Comments::default(),
                                                    statement,
                                                    document,
                                                    variable_span: string.span,
                                                })
                                            })
                                        })
                                        .collect());
                                }
                                Ok(Vec::new())
                            }
                            _ => {
                                let mut events = Vec::new();
                                if let Some(name) = call
                                    .only_arg()
                                    .and_then(|expr| expr.as_primary_string())
                                    .and_then(|s| parse_simple_literal(s.raw_value))
                                {
                                    events.push(AnalyzedEvent::Target(AnalyzedTarget {
                                        name,
                                        document,
                                        header: call.args[0].span(),
                                        span: call.span,
                                    }));
                                }
                                if let Some(block) = &call.block {
                                    events.push(AnalyzedEvent::NewScope(
                                        self.analyze_block(block, workspace, document, deps)?,
                                    ));
                                }
                                Ok(events)
                            }
                        }
                    }
                    Statement::Condition(condition) => {
                        let mut events = Vec::new();
                        let mut condition_blocks = Vec::new();
                        let mut current_condition = condition;
                        loop {
                            events.extend(self.analyze_expr(
                                &current_condition.condition,
                                workspace,
                                document,
                                deps,
                            )?);
                            condition_blocks.push(self.analyze_block(
                                &current_condition.then_block,
                                workspace,
                                document,
                                deps,
                            )?);
                            match &current_condition.else_block {
                                None => break,
                                Some(Either::Left(next_condition)) => {
                                    current_condition = next_condition;
                                }
                                Some(Either::Right(block)) => {
                                    condition_blocks.push(
                                        self.analyze_block(block, workspace, document, deps)?,
                                    );
                                    break;
                                }
                            }
                        }
                        events.push(AnalyzedEvent::Conditions(condition_blocks));
                        Ok(events)
                    }
                    Statement::Error(_) => Ok(Vec::new()),
                }
            })
            .collect::<std::io::Result<Vec<_>>>()?
            .into_iter()
            .flatten()
            .collect();

        Ok(AnalyzedBlock {
            events,
            span: block.span,
        })
    }

    fn analyze_expr<'i, 'p>(
        &mut self,
        expr: &'p Expr<'i>,
        workspace: &WorkspaceContext,
        document: &'i Document,
        deps: &mut Vec<Pin<Arc<ShallowAnalyzedFile>>>,
    ) -> std::io::Result<Vec<AnalyzedEvent<'i, 'p>>> {
        match expr {
            Expr::Primary(primary_expr) => match primary_expr.as_ref() {
                PrimaryExpr::Block(block) => {
                    let analyzed_root = self.analyze_block(block, workspace, document, deps)?;
                    Ok(vec![AnalyzedEvent::NewScope(analyzed_root)])
                }
                PrimaryExpr::Call(call) => {
                    let mut events: Vec<AnalyzedEvent> = call
                        .args
                        .iter()
                        .map(|expr| self.analyze_expr(expr, workspace, document, deps))
                        .collect::<std::io::Result<Vec<_>>>()?
                        .into_iter()
                        .flatten()
                        .collect();
                    if let Some(block) = &call.block {
                        let analyzed_root = self.analyze_block(block, workspace, document, deps)?;
                        events.push(AnalyzedEvent::NewScope(analyzed_root));
                    }
                    Ok(events)
                }
                PrimaryExpr::ParenExpr(paren_expr) => {
                    self.analyze_expr(&paren_expr.expr, workspace, document, deps)
                }
                PrimaryExpr::List(list_literal) => Ok(list_literal
                    .values
                    .iter()
                    .map(|expr| self.analyze_expr(expr, workspace, document, deps))
                    .collect::<std::io::Result<Vec<_>>>()?
                    .into_iter()
                    .flatten()
                    .collect()),
                PrimaryExpr::Identifier(_)
                | PrimaryExpr::Integer(_)
                | PrimaryExpr::String(_)
                | PrimaryExpr::ArrayAccess(_)
                | PrimaryExpr::ScopeAccess(_) => Ok(Vec::new()),
            },
            Expr::Unary(unary_expr) => {
                self.analyze_expr(&unary_expr.expr, workspace, document, deps)
            }
            Expr::Binary(binary_expr) => {
                let mut events = self.analyze_expr(&binary_expr.lhs, workspace, document, deps)?;
                events.extend(self.analyze_expr(&binary_expr.rhs, workspace, document, deps)?);
                Ok(events)
            }
        }
    }
}
