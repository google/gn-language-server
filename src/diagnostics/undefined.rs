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
    sync::{Arc, OnceLock},
    time::Instant,
};

use either::Either;
use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity};

use crate::{
    analyzer::{
        AnalyzedBlock, AnalyzedFile, AnalyzedStatement, Analyzer, TopLevelStatementsExt, Variable,
        VariableMap,
    },
    common::{builtins::BUILTINS, utils::is_exported},
    parser::{Expr, Identifier, LValue, PrimaryExpr},
};

fn builtin_scope() -> &'static Arc<VariableMap<'static, 'static>> {
    static SCOPE: OnceLock<Arc<VariableMap<'static, 'static>>> = OnceLock::new();
    SCOPE.get_or_init(|| {
        let mut scope = VariableMap::new();
        for keyword in ["true", "false"] {
            scope.insert(keyword, Variable::new(false));
        }
        for symbol in BUILTINS.all() {
            scope.insert(symbol.name, Variable::new(false));
        }
        Arc::new(scope)
    })
}

#[derive(Clone)]
enum VariablesTracker<'i, 'p> {
    Ok(VariableMap<'i, 'p>),
    Untrackable,
}

impl<'i, 'p> VariablesTracker<'i, 'p> {
    pub fn new() -> Self {
        Self::Ok(builtin_scope().as_ref().clone())
    }

    pub fn may_contain(&self, name: &str) -> bool {
        match self {
            VariablesTracker::Ok(env) => env.contains_key(name),
            VariablesTracker::Untrackable => true,
        }
    }

    pub fn insert(&mut self, name: &'i str) {
        match self {
            VariablesTracker::Ok(env) => {
                env.insert(name, Variable::new(false));
            }
            VariablesTracker::Untrackable => {}
        }
    }

    pub fn set_untrackable(&mut self) {
        *self = VariablesTracker::Untrackable;
    }
}

impl<'i, 'p> Extend<(&'i str, Variable<'i, 'p>)> for VariablesTracker<'i, 'p> {
    fn extend<T: IntoIterator<Item = (&'i str, Variable<'i, 'p>)>>(&mut self, iter: T) {
        match self {
            VariablesTracker::Ok(env) => env.extend(iter),
            VariablesTracker::Untrackable => {}
        }
    }
}

impl<'i> Identifier<'i> {
    fn collect_undefined_identifiers(
        &self,
        file: &'i AnalyzedFile,
        tracker: &VariablesTracker<'i, '_>,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        if !tracker.may_contain(self.name) {
            diagnostics.push(Diagnostic {
                range: file.document.line_index.range(self.span),
                severity: Some(DiagnosticSeverity::ERROR),
                message: format!("{} not defined", self.name),
                ..Default::default()
            })
        }
    }
}

impl<'i> PrimaryExpr<'i> {
    fn collect_undefined_identifiers(
        &self,
        file: &'i AnalyzedFile,
        analyzer: &Analyzer,
        request_time: Instant,
        tracker: &VariablesTracker<'i, '_>,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        match self {
            PrimaryExpr::Identifier(identifier) => {
                identifier.collect_undefined_identifiers(file, tracker, diagnostics);
            }
            PrimaryExpr::Call(call) => {
                call.function
                    .collect_undefined_identifiers(file, tracker, diagnostics);
                for expr in &call.args {
                    expr.collect_undefined_identifiers(
                        file,
                        analyzer,
                        request_time,
                        tracker,
                        diagnostics,
                    );
                }
            }
            PrimaryExpr::ArrayAccess(array_access) => {
                array_access
                    .array
                    .collect_undefined_identifiers(file, tracker, diagnostics);
                array_access.index.collect_undefined_identifiers(
                    file,
                    analyzer,
                    request_time,
                    tracker,
                    diagnostics,
                );
            }
            PrimaryExpr::ScopeAccess(scope_access) => {
                scope_access
                    .scope
                    .collect_undefined_identifiers(file, tracker, diagnostics);
            }
            PrimaryExpr::ParenExpr(paren_expr) => {
                paren_expr.expr.collect_undefined_identifiers(
                    file,
                    analyzer,
                    request_time,
                    tracker,
                    diagnostics,
                );
            }
            PrimaryExpr::List(list_literal) => {
                for expr in &list_literal.values {
                    expr.collect_undefined_identifiers(
                        file,
                        analyzer,
                        request_time,
                        tracker,
                        diagnostics,
                    );
                }
            }
            PrimaryExpr::Integer(_)
            | PrimaryExpr::String(_)
            | PrimaryExpr::Block(_)
            | PrimaryExpr::Error(_) => {}
        }
    }
}

impl<'i> Expr<'i> {
    fn collect_undefined_identifiers(
        &self,
        file: &'i AnalyzedFile,
        analyzer: &Analyzer,
        request_time: Instant,
        tracker: &VariablesTracker<'i, '_>,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        match self {
            Expr::Primary(primary_expr) => {
                primary_expr.collect_undefined_identifiers(
                    file,
                    analyzer,
                    request_time,
                    tracker,
                    diagnostics,
                );
            }
            Expr::Unary(unary_expr) => {
                unary_expr.expr.collect_undefined_identifiers(
                    file,
                    analyzer,
                    request_time,
                    tracker,
                    diagnostics,
                );
            }
            Expr::Binary(binary_expr) => {
                binary_expr.lhs.collect_undefined_identifiers(
                    file,
                    analyzer,
                    request_time,
                    tracker,
                    diagnostics,
                );
                binary_expr.rhs.collect_undefined_identifiers(
                    file,
                    analyzer,
                    request_time,
                    tracker,
                    diagnostics,
                );
            }
        }
    }
}

impl<'i, 'p> AnalyzedBlock<'i, 'p> {
    fn collect_undefined_identifiers(
        &self,
        file: &AnalyzedFile,
        analyzer: &Analyzer,
        request_time: Instant,
        tracker: &mut VariablesTracker<'i, 'p>,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        for statement in self.top_level_statements() {
            // Collect undefined identifiers in expressions.
            match statement {
                AnalyzedStatement::Assignment(assignment) => {
                    if let LValue::ArrayAccess(array_access) = &assignment.assignment.lvalue {
                        array_access.index.collect_undefined_identifiers(
                            file,
                            analyzer,
                            request_time,
                            tracker,
                            diagnostics,
                        );
                    }
                    assignment.assignment.rvalue.collect_undefined_identifiers(
                        file,
                        analyzer,
                        request_time,
                        tracker,
                        diagnostics,
                    );
                }
                AnalyzedStatement::Conditions(condition) => {
                    let mut current_condition = condition;
                    loop {
                        current_condition
                            .condition
                            .condition
                            .collect_undefined_identifiers(
                                file,
                                analyzer,
                                request_time,
                                tracker,
                                diagnostics,
                            );
                        match &current_condition.else_block {
                            Some(Either::Left(next_condition)) => {
                                current_condition = next_condition;
                            }
                            Some(Either::Right(_)) | None => break,
                        }
                    }
                }
                AnalyzedStatement::Foreach(foreach) => {
                    foreach.loop_items.collect_undefined_identifiers(
                        file,
                        analyzer,
                        request_time,
                        tracker,
                        diagnostics,
                    );
                }
                AnalyzedStatement::ForwardVariablesFrom(forward_variables_from) => {
                    for expr in &forward_variables_from.call.args {
                        expr.collect_undefined_identifiers(
                            file,
                            analyzer,
                            request_time,
                            tracker,
                            diagnostics,
                        );
                    }
                }
                AnalyzedStatement::Target(target) => {
                    for expr in &target.call.args {
                        expr.collect_undefined_identifiers(
                            file,
                            analyzer,
                            request_time,
                            tracker,
                            diagnostics,
                        );
                    }
                }
                AnalyzedStatement::Template(template) => {
                    for expr in &template.call.args {
                        expr.collect_undefined_identifiers(
                            file,
                            analyzer,
                            request_time,
                            tracker,
                            diagnostics,
                        );
                    }
                }
                AnalyzedStatement::BuiltinCall(builtin_call) => {
                    builtin_call.call.function.collect_undefined_identifiers(
                        file,
                        tracker,
                        diagnostics,
                    );
                    for expr in &builtin_call.call.args {
                        expr.collect_undefined_identifiers(
                            file,
                            analyzer,
                            request_time,
                            tracker,
                            diagnostics,
                        );
                    }
                }
                AnalyzedStatement::DeclareArgs(_) | AnalyzedStatement::Import(_) => {}
            }

            // Collect undefined identifiers in subscopes.
            for subscope in statement.subscopes() {
                subscope.collect_undefined_identifiers(
                    file,
                    analyzer,
                    request_time,
                    &mut tracker.clone(),
                    diagnostics,
                );
            }

            // Update variables.
            match statement {
                AnalyzedStatement::Assignment(assignment) => {
                    if let LValue::Identifier(identifier) = &assignment.assignment.lvalue {
                        tracker.insert(identifier.name);
                    }
                }
                AnalyzedStatement::Foreach(foreach) => {
                    tracker.insert(foreach.loop_variable.name);
                }
                AnalyzedStatement::ForwardVariablesFrom(forward_variables_from) => {
                    if let Some(includes) = forward_variables_from.includes.as_simple_string_list()
                    {
                        for include in includes {
                            tracker.insert(include);
                        }
                    } else {
                        tracker.set_untrackable();
                    }
                }
                AnalyzedStatement::Import(import) => {
                    if let Ok(imported_file) = analyzer.analyze_file(&import.path, request_time) {
                        if let Ok(imported_environment) = analyzer.analyze_environment(
                            &imported_file,
                            imported_file.document.data.len(),
                            request_time,
                        ) {
                            tracker.extend(
                                imported_environment
                                    .variables
                                    .into_iter()
                                    .filter(|(name, _)| is_exported(name)),
                            );
                        }
                    }
                }
                AnalyzedStatement::Conditions(_)
                | AnalyzedStatement::DeclareArgs(_)
                | AnalyzedStatement::Target(_)
                | AnalyzedStatement::Template(_)
                | AnalyzedStatement::BuiltinCall(_) => {}
            }
        }
    }
}

pub fn collect_undefined_identifiers(
    file: &AnalyzedFile,
    analyzer: &Analyzer,
    request_time: Instant,
    diagnostics: &mut Vec<Diagnostic>,
) {
    file.analyzed_root.collect_undefined_identifiers(
        file,
        analyzer,
        request_time,
        &mut VariablesTracker::new(),
        diagnostics,
    );
}
