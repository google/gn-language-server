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
    collections::{BTreeMap, HashMap, HashSet},
    path::{Path, PathBuf},
    pin::Pin,
    sync::{Arc, Mutex},
    time::Instant,
};

use either::Either;

use crate::{
    analyzer::{
        data::{
            AnalyzedBuiltinCall, AnalyzedCondition, AnalyzedDeclareArgs, AnalyzedForeach,
            AnalyzedForwardVariablesFrom, AnalyzedStatement, FileExports, MutableFileExports,
            PathSpan, TemplateMap, VariableAssignment,
        },
        links::collect_links,
        symbols::collect_symbols,
        AnalyzedAssignment, AnalyzedBlock, AnalyzedFile, AnalyzedImport, AnalyzedLink,
        AnalyzedTarget, AnalyzedTemplate, Environment, Target, Template, TopLevelStatementsExt,
        Variable, VariableMap, WorkspaceContext,
    },
    common::{
        builtins::{DECLARE_ARGS, FOREACH, FORWARD_VARIABLES_FROM, IMPORT, SET_DEFAULTS, TEMPLATE},
        storage::{Document, DocumentStorage},
        utils::{is_exported, parse_simple_literal},
    },
    parser::{parse, Block, Call, Comments, Condition, Expr, LValue, Node, PrimaryExpr, Statement},
};

pub struct Analyzer {
    context: WorkspaceContext,
    storage: Arc<Mutex<DocumentStorage>>,
    #[allow(clippy::type_complexity)]
    cache: BTreeMap<PathBuf, Pin<Arc<AnalyzedFile>>>,
}

impl Analyzer {
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

    pub fn cached_files(&self) -> Vec<Pin<Arc<AnalyzedFile>>> {
        self.cache.values().cloned().collect()
    }

    pub fn analyze_file(&mut self, path: &Path, request_time: Instant) -> Pin<Arc<AnalyzedFile>> {
        if let Some(cached_file) = self.cache.get(path) {
            if cached_file
                .key
                .verify(request_time, &self.storage.lock().unwrap())
            {
                return cached_file.clone();
            }
        }

        let new_file = self.analyze_file_uncached(path, request_time);
        self.cache.insert(path.to_path_buf(), new_file.clone());
        new_file
    }

    pub fn analyze_environment(
        &mut self,
        file: &Pin<Arc<AnalyzedFile>>,
        pos: usize,
        request_time: Instant,
    ) -> Environment {
        let mut environment = Environment::new();
        let mut visited = HashSet::from([file.document.path.clone()]);

        // Collect from BUILDCONFIG.gn.
        self.collect_environments(
            &self.context.build_config.clone(),
            request_time,
            &mut visited,
            &mut environment,
        );

        // Collect from imported files.
        for child_path in file.exports.children.as_ref() {
            self.collect_environments(child_path, request_time, &mut visited, &mut environment);
        }

        // Collect from the local file.
        // SAFETY: variables and templates are backed by file.
        unsafe {
            environment
                .variables
                .extend(std::mem::transmute::<VariableMap, VariableMap>(
                    file.local_variables_at(pos),
                ));
            environment
                .templates
                .extend(std::mem::transmute::<TemplateMap, TemplateMap>(
                    file.local_templates_at(pos),
                ));
        }
        environment.files.push(file.clone());

        environment
    }

    fn collect_environments(
        &mut self,
        path: &Path,
        request_time: Instant,
        visited: &mut HashSet<PathBuf>,
        environment: &mut Environment,
    ) {
        if !visited.insert(path.to_path_buf()) {
            return;
        }
        let file = self.analyze_file(path, request_time);
        for child_path in file.exports.children.as_ref() {
            self.collect_environments(child_path, request_time, visited, environment);
        }
        environment
            .variables
            .extend(file.exports.variables.as_ref().clone());
        environment
            .templates
            .extend(file.exports.templates.as_ref().clone());
        environment.files.push(file.clone());
    }

    fn analyze_file_uncached(
        &mut self,
        path: &Path,
        request_time: Instant,
    ) -> Pin<Arc<AnalyzedFile>> {
        let document = self.storage.lock().unwrap().read(path);
        let ast = Box::pin(parse(&document.data));

        let analyzed_root = self.analyze_block(&ast, &document);
        let exports = self.analyze_exports(&ast, &document);

        let links_map = collect_links(&ast, path, &self.context);
        let symbols = collect_symbols(ast.as_node(), &document.line_index);

        // SAFETY: links' contents are backed by pinned document.
        let links_map = unsafe {
            std::mem::transmute::<
                HashMap<PathBuf, Vec<AnalyzedLink>>,
                HashMap<PathBuf, Vec<AnalyzedLink>>,
            >(links_map)
        };
        // SAFETY: exports' contents are backed by pinned document and pinned ast.
        let exports = unsafe { std::mem::transmute::<FileExports, FileExports>(exports) };
        // SAFETY: analyzed_root's contents are backed by pinned document and pinned ast.
        let analyzed_root =
            unsafe { std::mem::transmute::<AnalyzedBlock, AnalyzedBlock>(analyzed_root) };
        // SAFETY: ast's contents are backed by pinned document.
        let ast = unsafe { std::mem::transmute::<Pin<Box<Block>>, Pin<Box<Block>>>(ast) };

        AnalyzedFile::new(
            document,
            self.context.root.clone(),
            ast,
            analyzed_root,
            exports,
            links_map,
            symbols,
            request_time,
        )
    }

    fn analyze_block<'i, 'p>(
        &mut self,
        block: &'p Block<'i>,
        document: &'i Document,
    ) -> AnalyzedBlock<'i, 'p> {
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
            block,
            document,
            span: block.span,
        }
    }

    fn analyze_call<'i, 'p>(
        &mut self,
        call: &'p Call<'i>,
        document: &'i Document,
    ) -> AnalyzedStatement<'i, 'p> {
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
                    return AnalyzedStatement::Import(Box::new(AnalyzedImport { call, path }));
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

    fn analyze_condition<'i, 'p>(
        &mut self,
        condition: &'p Condition<'i>,
        document: &'i Document,
    ) -> AnalyzedCondition<'i, 'p> {
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

    fn analyze_expr<'i, 'p>(
        &mut self,
        expr: &'p Expr<'i>,
        document: &'i Document,
    ) -> Vec<AnalyzedBlock<'i, 'p>> {
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

    fn analyze_exports<'i, 'p>(
        &mut self,
        block: &'p Block<'i>,
        document: &'i Document,
    ) -> FileExports<'i, 'p> {
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
                            .insert(
                                PathSpan {
                                    path: &document.path,
                                    span: identifier.span,
                                },
                                VariableAssignment {
                                    document,
                                    assignment_or_call: Either::Left(assignment),
                                    primary_variable: identifier.span,
                                    comments: assignment.comments.clone(),
                                },
                            );
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
                                            .insert(
                                                PathSpan {
                                                    path: &document.path,
                                                    span: string.span,
                                                },
                                                VariableAssignment {
                                                    document,
                                                    assignment_or_call: Either::Right(call),
                                                    primary_variable: string.span,
                                                    comments: Comments::default(),
                                                },
                                            );
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
