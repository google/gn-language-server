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
    collections::{BTreeMap, HashSet},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, RwLock},
    time::Instant,
};

use either::Either;

use crate::{
    analyzer::{dotgn::evaluate_dot_gn, links::collect_links, symbols::collect_symbols},
    common::{
        builtins::{DECLARE_ARGS, FOREACH, FORWARD_VARIABLES_FROM, IMPORT, SET_DEFAULTS, TEMPLATE},
        error::{Error, Result},
        storage::{Document, DocumentStorage},
        utils::{is_exported, parse_simple_literal},
        workspace::WorkspaceFinder,
    },
    parser::{
        parse, Block, Call, Comments, Condition, Expr, LValue, Node, OwnedBlock, PrimaryExpr,
        Statement,
    },
};

pub use data::{
    AnalyzedAssignment, AnalyzedBlock, AnalyzedBuiltinCall, AnalyzedCondition, AnalyzedDeclareArgs,
    AnalyzedFile, AnalyzedForeach, AnalyzedForwardVariablesFrom, AnalyzedImport, AnalyzedLink,
    AnalyzedStatement, AnalyzedTarget, AnalyzedTemplate, Environment, FileExports,
    MutableFileExports, OwnedAnalyzedBlock, OwnedEnvironment, OwnedFileExports, OwnedLinkIndex,
    Target, Template, Variable, VariableAssignment, WorkspaceContext,
};

pub use toplevel::TopLevelStatementsExt;

mod cache;
mod data;
mod dotgn;
mod links;
mod symbols;
mod tests;
mod toplevel;
mod utils;

pub struct Analyzer {
    storage: Arc<Mutex<DocumentStorage>>,
    workspace_finder: WorkspaceFinder,
    workspaces: RwLock<BTreeMap<PathBuf, Arc<Mutex<WorkspaceAnalyzer>>>>,
}

impl Analyzer {
    pub fn new(storage: &Arc<Mutex<DocumentStorage>>, workspace_finder: WorkspaceFinder) -> Self {
        Self {
            storage: storage.clone(),
            workspace_finder,
            workspaces: Default::default(),
        }
    }

    pub fn analyze_file(&self, path: &Path, request_time: Instant) -> Result<Arc<AnalyzedFile>> {
        Ok(self
            .workspace_for(path)?
            .lock()
            .unwrap()
            .analyze_file(path, request_time))
    }

    pub fn analyze_at(
        &self,
        file: &Arc<AnalyzedFile>,
        pos: usize,
        request_time: Instant,
    ) -> Result<OwnedEnvironment> {
        Ok(self
            .workspace_for(&file.document.path)?
            .lock()
            .unwrap()
            .analyze_at(file, pos, request_time))
    }

    pub fn workspaces(&self) -> BTreeMap<PathBuf, Arc<Mutex<WorkspaceAnalyzer>>> {
        self.workspaces.read().unwrap().clone()
    }

    pub fn workspace_finder(&self) -> &WorkspaceFinder {
        &self.workspace_finder
    }

    pub fn workspace_for(&self, path: &Path) -> Result<Arc<Mutex<WorkspaceAnalyzer>>> {
        if !path.is_absolute() {
            return Err(Error::General("Path must be absolute".to_string()));
        }

        let workspace_root = self
            .workspace_finder
            .find_for(path)
            .ok_or(Error::General("Workspace not found".to_string()))?;
        let dot_gn_path = workspace_root.join(".gn");
        let dot_gn_version = {
            let storage = self.storage.lock().unwrap();
            storage.read_version(&dot_gn_path)
        };

        {
            let read_lock = self.workspaces.read().unwrap();
            if let Some(analyzer) = read_lock.get(workspace_root) {
                if analyzer.lock().unwrap().context().dot_gn_version == dot_gn_version {
                    return Ok(analyzer.clone());
                }
            }
        }

        let build_config = {
            let storage = self.storage.lock().unwrap();
            let document = storage.read(&dot_gn_path);
            evaluate_dot_gn(workspace_root, &document.data)?
        };

        let context = WorkspaceContext {
            root: workspace_root.to_path_buf(),
            dot_gn_version,
            build_config,
        };

        let analyzer = Arc::new(Mutex::new(WorkspaceAnalyzer::new(&context, &self.storage)));

        let mut write_lock = self.workspaces.write().unwrap();
        Ok(write_lock
            .entry(workspace_root.to_path_buf())
            .or_insert(analyzer)
            .clone())
    }
}

pub struct WorkspaceAnalyzer {
    context: WorkspaceContext,
    storage: Arc<Mutex<DocumentStorage>>,
    cache: BTreeMap<PathBuf, Arc<AnalyzedFile>>,
}

impl WorkspaceAnalyzer {
    pub fn new(context: &WorkspaceContext, storage: &Arc<Mutex<DocumentStorage>>) -> Self {
        Self {
            context: context.clone(),
            storage: storage.clone(),
            cache: Default::default(),
        }
    }

    pub fn context(&self) -> &WorkspaceContext {
        &self.context
    }

    pub fn cached_files_for_symbols(&self) -> Vec<Arc<AnalyzedFile>> {
        self.cache
            .values()
            .filter(|file| !file.external)
            .cloned()
            .collect()
    }

    pub fn cached_files_for_references(&self) -> Vec<Arc<AnalyzedFile>> {
        self.cache.values().cloned().collect()
    }

    pub fn analyze_file(&mut self, path: &Path, request_time: Instant) -> Arc<AnalyzedFile> {
        if let Some(cached_file) = self.cache.get(path) {
            if cached_file
                .key
                .verify(request_time, &self.storage.lock().unwrap())
            {
                return cached_file.clone();
            }
        }

        let new_file = Arc::new(self.analyze_file_uncached(path, request_time));
        self.cache.insert(path.to_path_buf(), new_file.clone());
        new_file
    }

    pub fn analyze_files(&mut self, path: &Path, request_time: Instant) -> OwnedEnvironment {
        let mut files: Vec<Arc<AnalyzedFile>> = Vec::new();
        self.collect_imports(path, request_time, &mut files, &mut HashSet::new());

        OwnedEnvironment::new(files, |files| {
            let mut environment = Environment::new();
            for file in files.iter().rev() {
                environment
                    .variables
                    .extend(file.exports.get().variables.clone());
                environment
                    .templates
                    .extend(file.exports.get().templates.clone());
            }
            environment
        })
    }

    pub fn analyze_at(
        &mut self,
        file: &Arc<AnalyzedFile>,
        pos: usize,
        request_time: Instant,
    ) -> OwnedEnvironment {
        let mut files: Vec<Arc<AnalyzedFile>> = vec![file.clone()];
        let mut visited = HashSet::from([file.document.path.clone()]);

        // Collect BUILDCONFIG.gn.
        self.collect_imports(
            &self.context.build_config.clone(),
            request_time,
            &mut files,
            &mut visited,
        );

        // Collect imported files.
        for child_path in &file.exports.get().children {
            self.collect_imports(child_path, request_time, &mut files, &mut visited);
        }

        OwnedEnvironment::new(files, |files| {
            let mut environment = Environment::new();
            let (current_file, imported_files) = files.split_first().unwrap();

            for file in imported_files.iter().rev() {
                environment
                    .variables
                    .extend(file.exports.get().variables.clone());
                environment
                    .templates
                    .extend(file.exports.get().templates.clone());
            }

            environment
                .variables
                .extend(current_file.local_variables_at(pos));
            environment
                .templates
                .extend(current_file.local_templates_at(pos));

            environment
        })
    }

    fn collect_imports(
        &mut self,
        path: &Path,
        request_time: Instant,
        files: &mut Vec<Arc<AnalyzedFile>>,
        visited: &mut HashSet<PathBuf>,
    ) {
        if !visited.insert(path.to_path_buf()) {
            return;
        }
        let file = self.analyze_file(path, request_time);
        files.push(file.clone());
        for child_path in &file.exports.get().children {
            self.collect_imports(child_path, request_time, files, visited);
        }
    }

    fn analyze_file_uncached(&mut self, path: &Path, request_time: Instant) -> AnalyzedFile {
        let document = self.storage.lock().unwrap().read(path);
        let ast = OwnedBlock::new(document.clone(), |document| parse(&document.data));

        let analyzed_root = OwnedAnalyzedBlock::new(ast.clone(), |ast| {
            self.analyze_block(ast.get(), ast.document())
        });
        let exports = OwnedFileExports::new(ast.clone(), |ast| {
            self.analyze_exports(ast.get(), ast.document())
        });
        let link_index = OwnedLinkIndex::new(ast.clone(), |ast| {
            collect_links(ast.get(), path, &self.context)
        });
        let symbols = collect_symbols(ast.get(), &document.line_index);

        AnalyzedFile::new(
            document,
            self.context.root.clone(),
            ast,
            analyzed_root,
            exports,
            link_index,
            symbols,
            request_time,
        )
    }

    fn analyze_block<'p>(
        &mut self,
        block: &'p Block<'p>,
        document: &'p Document,
    ) -> AnalyzedBlock<'p> {
        let mut statements: Vec<AnalyzedStatement> = Vec::new();

        for statement in &block.statements {
            match statement {
                Statement::Assignment(assignment) => {
                    let (identifier, mut expr_scopes) = match &assignment.lvalue {
                        LValue::Identifier(identifier) => (identifier.as_ref(), Vec::new()),
                        LValue::ArrayAccess(array_access) => (
                            &array_access.array,
                            self.analyze_expr(&array_access.index, document),
                        ),
                        LValue::ScopeAccess(scope_access) => (&scope_access.scope, Vec::new()),
                    };
                    expr_scopes.extend(self.analyze_expr(&assignment.rvalue, document));
                    statements.push(AnalyzedStatement::Assignment(Box::new(
                        AnalyzedAssignment {
                            assignment,
                            primary_variable: identifier.span,
                            comments: assignment.comments.clone(),
                            expr_scopes,
                        },
                    )));
                }
                Statement::Call(call) => {
                    statements.push(self.analyze_call(call, document));
                }
                Statement::Condition(condition) => {
                    statements.push(AnalyzedStatement::Conditions(Box::new(
                        self.analyze_condition(condition, document),
                    )));
                }
                Statement::Error(_) => {}
            }
        }

        AnalyzedBlock {
            statements,
            document,
            span: block.span,
        }
    }

    fn analyze_call<'p>(
        &mut self,
        call: &'p Call<'p>,
        document: &'p Document,
    ) -> AnalyzedStatement<'p> {
        let body_block = call
            .block
            .as_ref()
            .map(|block| self.analyze_block(block, document));

        let body_block = match (call.function.name, body_block) {
            (DECLARE_ARGS, Some(body_block)) => {
                return AnalyzedStatement::DeclareArgs(Box::new(AnalyzedDeclareArgs {
                    call,
                    body_block,
                }));
            }
            (FOREACH, Some(body_block)) => {
                if call.args.len() == 2 {
                    if let Some(loop_variable) = call.args[0].as_primary_identifier() {
                        let loop_items = &call.args[1];
                        let expr_scopes = self.analyze_expr(loop_items, document);
                        return AnalyzedStatement::Foreach(Box::new(AnalyzedForeach {
                            call,
                            loop_variable,
                            loop_items,
                            expr_scopes,
                            body_block,
                        }));
                    }
                }
                Some(body_block)
            }
            (FORWARD_VARIABLES_FROM, None) => {
                if call.args.len() == 2 || call.args.len() == 3 {
                    let expr_scopes = call
                        .args
                        .iter()
                        .flat_map(|expr| self.analyze_expr(expr, document))
                        .collect();
                    return AnalyzedStatement::ForwardVariablesFrom(Box::new(
                        AnalyzedForwardVariablesFrom {
                            call,
                            expr_scopes,
                            includes: &call.args[1],
                        },
                    ));
                }
                None
            }
            (IMPORT, None) => {
                if let Some(name) = call.only_arg().and_then(|expr| expr.as_simple_string()) {
                    let path = self
                        .context
                        .resolve_path(name, document.path.parent().unwrap());
                    return AnalyzedStatement::Import(Box::new(AnalyzedImport { call, name, path }));
                }
                None
            }
            (TEMPLATE, Some(body_block)) => {
                if let Some(name) = call.only_arg() {
                    let expr_scopes = call
                        .args
                        .iter()
                        .flat_map(|expr| self.analyze_expr(expr, document))
                        .collect();
                    return AnalyzedStatement::Template(Box::new(AnalyzedTemplate {
                        call,
                        name,
                        comments: call.comments.clone(),
                        expr_scopes,
                        body_block,
                    }));
                }
                Some(body_block)
            }
            (name, Some(body_block)) if name != SET_DEFAULTS => {
                if let Some(name) = call.only_arg() {
                    let expr_scopes = call
                        .args
                        .iter()
                        .flat_map(|expr| self.analyze_expr(expr, document))
                        .collect();
                    return AnalyzedStatement::Target(Box::new(AnalyzedTarget {
                        call,
                        name,
                        expr_scopes,
                        body_block,
                    }));
                }
                Some(body_block)
            }
            (_, body_block) => body_block,
        };

        let expr_scopes = call
            .args
            .iter()
            .flat_map(|expr| self.analyze_expr(expr, document))
            .collect();
        AnalyzedStatement::BuiltinCall(Box::new(AnalyzedBuiltinCall {
            call,
            expr_scopes,
            body_block,
        }))
    }

    fn analyze_condition<'p>(
        &mut self,
        condition: &'p Condition<'p>,
        document: &'p Document,
    ) -> AnalyzedCondition<'p> {
        let expr_scopes = self.analyze_expr(&condition.condition, document);
        let then_block = self.analyze_block(&condition.then_block, document);
        let else_block = match &condition.else_block {
            None => None,
            Some(Either::Left(next_condition)) => Some(Either::Left(Box::new(
                self.analyze_condition(next_condition, document),
            ))),
            Some(Either::Right(last_block)) => Some(Either::Right(Box::new(
                self.analyze_block(last_block, document),
            ))),
        };
        AnalyzedCondition {
            condition,
            expr_scopes,
            then_block,
            else_block,
        }
    }

    fn analyze_expr<'p>(
        &mut self,
        expr: &'p Expr<'p>,
        document: &'p Document,
    ) -> Vec<AnalyzedBlock<'p>> {
        match expr {
            Expr::Primary(primary_expr) => match primary_expr.as_ref() {
                PrimaryExpr::Block(block) => {
                    let analyzed_block = self.analyze_block(block, document);
                    vec![analyzed_block]
                }
                PrimaryExpr::Call(call) => {
                    let mut analyzed_blocks: Vec<AnalyzedBlock> = call
                        .args
                        .iter()
                        .flat_map(|expr| self.analyze_expr(expr, document))
                        .collect();
                    if let Some(block) = &call.block {
                        let analyzed_block = self.analyze_block(block, document);
                        analyzed_blocks.push(analyzed_block);
                    }
                    analyzed_blocks
                }
                PrimaryExpr::ParenExpr(paren_expr) => self.analyze_expr(&paren_expr.expr, document),
                PrimaryExpr::List(list_literal) => list_literal
                    .values
                    .iter()
                    .flat_map(|expr| self.analyze_expr(expr, document))
                    .collect(),
                PrimaryExpr::Identifier(_)
                | PrimaryExpr::Integer(_)
                | PrimaryExpr::String(_)
                | PrimaryExpr::ArrayAccess(_)
                | PrimaryExpr::ScopeAccess(_)
                | PrimaryExpr::Error(_) => Vec::new(),
            },
            Expr::Unary(unary_expr) => self.analyze_expr(&unary_expr.expr, document),
            Expr::Binary(binary_expr) => {
                let mut analyzed_blocks = self.analyze_expr(&binary_expr.lhs, document);
                analyzed_blocks.extend(self.analyze_expr(&binary_expr.rhs, document));
                analyzed_blocks
            }
        }
    }

    fn analyze_exports<'p>(
        &mut self,
        block: &'p Block<'p>,
        document: &'p Document,
    ) -> FileExports<'p> {
        let mut exports = MutableFileExports::new();
        let mut declare_args_stack: Vec<&Call> = Vec::new();

        for statement in block.top_level_statements() {
            while let Some(last_declare_args) = declare_args_stack.last() {
                if statement.span().start_pos() <= last_declare_args.span.end_pos() {
                    break;
                }
                declare_args_stack.pop();
            }
            match statement {
                Statement::Assignment(assignment) => {
                    let identifier = match &assignment.lvalue {
                        LValue::Identifier(identifier) => identifier,
                        LValue::ArrayAccess(array_access) => &array_access.array,
                        LValue::ScopeAccess(scope_access) => &scope_access.scope,
                    };
                    if is_exported(identifier.name) {
                        exports
                            .variables
                            .entry(identifier.name)
                            .or_insert_with(|| Variable::new(!declare_args_stack.is_empty()))
                            .assignments
                            .push(VariableAssignment {
                                document,
                                assignment_or_call: Either::Left(assignment),
                                primary_variable: identifier.span,
                                comments: assignment.comments.clone(),
                            });
                    }
                }
                Statement::Call(call) => match call.function.name {
                    IMPORT => {
                        if let Some(name) = call.only_arg().and_then(|expr| expr.as_simple_string())
                        {
                            let path = self
                                .context
                                .resolve_path(name, document.path.parent().unwrap());
                            exports.children.push(path);
                        }
                    }
                    TEMPLATE => {
                        if let Some(name) = call.only_arg().and_then(|expr| expr.as_simple_string())
                        {
                            if is_exported(name) {
                                exports.templates.insert(
                                    name,
                                    Template {
                                        document,
                                        call,
                                        name,
                                        comments: call.comments.clone(),
                                    },
                                );
                            }
                        }
                    }
                    DECLARE_ARGS => {
                        declare_args_stack.push(call);
                    }
                    FOREACH | SET_DEFAULTS => {}
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
                            for string in strings {
                                if let Some(name) = parse_simple_literal(string.raw_value) {
                                    if is_exported(name) {
                                        exports
                                            .variables
                                            .entry(name)
                                            .or_insert_with(|| {
                                                Variable::new(!declare_args_stack.is_empty())
                                            })
                                            .assignments
                                            .push(VariableAssignment {
                                                document,
                                                assignment_or_call: Either::Right(call),
                                                primary_variable: string.span,
                                                comments: Comments::default(),
                                            });
                                    }
                                }
                            }
                        }
                    }
                    _ => {
                        if let Some(name) = call.only_arg().and_then(|expr| expr.as_simple_string())
                        {
                            exports.targets.insert(
                                name,
                                Target {
                                    document,
                                    call,
                                    name,
                                },
                            );
                        }
                    }
                },
                Statement::Condition(_) | Statement::Error(_) => {}
            }
        }

        exports.finalize()
    }
}
