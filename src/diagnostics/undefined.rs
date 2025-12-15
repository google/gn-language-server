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

use std::{collections::HashSet, sync::OnceLock, time::Instant};

use either::Either;
use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity, NumberOrString};

use crate::{
    analyzer::{
        AnalyzedBlock, AnalyzedFile, AnalyzedStatement, Analyzer, TopLevelStatementsExt,
        WorkspaceAnalyzer,
    },
    common::builtins::{BUILTINS, DEFINED},
    diagnostics::{DiagnosticDataUndefined, DIAGNOSTIC_CODE_UNDEFINED},
    parser::{Expr, Identifier, LValue, PrimaryExpr},
};

fn builtin_scope() -> HashSet<String> {
    static SCOPE: OnceLock<HashSet<String>> = OnceLock::new();
    SCOPE
        .get_or_init(|| {
            let mut scope = HashSet::new();
            for keyword in ["true", "false"] {
                scope.insert(keyword.to_string());
            }
            for symbol in BUILTINS.all() {
                scope.insert(symbol.name.to_string());
            }
            scope
        })
        .clone()
}

#[derive(Clone)]
enum EnvironmentTracker {
    Ok(HashSet<String>),
    Untrackable,
}

impl EnvironmentTracker {
    pub fn new() -> Self {
        Self::Ok(builtin_scope())
    }

    pub fn may_contain(&self, name: &str) -> bool {
        match self {
            EnvironmentTracker::Ok(env) => env.contains(name),
            EnvironmentTracker::Untrackable => true,
        }
    }

    pub fn insert(&mut self, name: &str) {
        match self {
            EnvironmentTracker::Ok(env) => {
                env.insert(name.to_string());
            }
            EnvironmentTracker::Untrackable => {}
        }
    }

    pub fn set_untrackable(&mut self) {
        *self = EnvironmentTracker::Untrackable;
    }
}

impl<'s> Extend<&'s str> for EnvironmentTracker {
    fn extend<T: IntoIterator<Item = &'s str>>(&mut self, iter: T) {
        match self {
            EnvironmentTracker::Ok(env) => env.extend(iter.into_iter().map(|s| s.to_string())),
            EnvironmentTracker::Untrackable => {}
        }
    }
}

impl<'p> Identifier<'p> {
    fn collect_undefined_identifiers(
        &self,
        file: &'p AnalyzedFile,
        tracker: &EnvironmentTracker,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        if !tracker.may_contain(self.name) {
            diagnostics.push(Diagnostic {
                range: file.document.line_index.range(self.span),
                severity: Some(DiagnosticSeverity::ERROR),
                code: Some(NumberOrString::String(
                    DIAGNOSTIC_CODE_UNDEFINED.to_string(),
                )),
                message: format!("{} not defined", self.name),
                data: Some(
                    serde_json::to_value(DiagnosticDataUndefined {
                        name: self.name.to_string(),
                    })
                    .unwrap(),
                ),
                ..Default::default()
            })
        }
    }
}

impl<'p> PrimaryExpr<'p> {
    fn collect_undefined_identifiers(
        &self,
        file: &'p AnalyzedFile,
        analyzer: &mut WorkspaceAnalyzer,
        request_time: Instant,
        tracker: &EnvironmentTracker,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        match self {
            PrimaryExpr::Identifier(identifier) => {
                identifier.collect_undefined_identifiers(file, tracker, diagnostics);
            }
            PrimaryExpr::Call(call) => {
                call.function
                    .collect_undefined_identifiers(file, tracker, diagnostics);
                if call.function.name != DEFINED {
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

impl<'p> Expr<'p> {
    fn collect_undefined_identifiers(
        &self,
        file: &'p AnalyzedFile,
        analyzer: &mut WorkspaceAnalyzer,
        request_time: Instant,
        tracker: &EnvironmentTracker,
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

impl<'p> AnalyzedBlock<'p> {
    fn collect_undefined_identifiers(
        &self,
        file: &AnalyzedFile,
        analyzer: &mut WorkspaceAnalyzer,
        request_time: Instant,
        tracker: &mut EnvironmentTracker,
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
                    let imported_environment = analyzer.analyze_files(&import.path, request_time);
                    tracker.extend(imported_environment.get().variables.keys().copied());
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
) -> Vec<Diagnostic> {
    let Ok(analyzer) = analyzer.workspace_for(&file.workspace_root) else {
        return Vec::new();
    };
    let mut analyzer = analyzer.lock().unwrap();

    // Process BUILDCONFIG.gn.
    let mut tracker = EnvironmentTracker::new();
    let build_config = analyzer.context().build_config.clone();
    let environment = analyzer.analyze_files(&build_config, request_time);
    tracker.extend(environment.get().variables.keys().copied());

    let mut diagnostics: Vec<Diagnostic> = Vec::new();
    file.analyzed_root.get().collect_undefined_identifiers(
        file,
        &mut analyzer,
        request_time,
        &mut tracker,
        &mut diagnostics,
    );
    diagnostics
}
