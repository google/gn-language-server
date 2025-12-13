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
    collections::HashMap,
    path::{Path, PathBuf},
    pin::Pin,
    sync::Arc,
    time::Instant,
};

use either::Either;
use pest::Span;
use tower_lsp::lsp_types::DocumentSymbol;

use crate::{
    analyzer::{cache::CacheKey, toplevel::TopLevelStatementsExt, utils::resolve_path},
    common::{
        storage::{Document, DocumentVersion},
        utils::parse_simple_literal,
    },
    parser::{Assignment, Block, Call, Comments, Condition, Expr, Identifier},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspaceContext {
    pub root: PathBuf,
    pub dot_gn_version: DocumentVersion,
    pub build_config: PathBuf,
}

impl WorkspaceContext {
    pub fn resolve_path(&self, name: &str, current_dir: &Path) -> PathBuf {
        resolve_path(name, &self.root, current_dir)
    }
}

pub type StrKeyedMap<'i, T> = HashMap<&'i str, T>;
pub type VariableMap<'i, 'p> = StrKeyedMap<'i, Variable<'i, 'p>>;
pub type TemplateMap<'i, 'p> = StrKeyedMap<'i, Template<'i, 'p>>;
pub type TargetMap<'i, 'p> = StrKeyedMap<'i, Target<'i, 'p>>;

#[derive(Default)]
pub struct MutableFileExports<'i, 'p> {
    pub variables: VariableMap<'i, 'p>,
    pub templates: TemplateMap<'i, 'p>,
    pub targets: TargetMap<'i, 'p>,
    pub children: Vec<PathBuf>,
}

impl MutableFileExports<'_, '_> {
    pub fn new() -> Self {
        Default::default()
    }
}

impl<'i, 'p> MutableFileExports<'i, 'p> {
    pub fn finalize(self) -> FileExports<'i, 'p> {
        FileExports {
            variables: Arc::new(self.variables),
            templates: Arc::new(self.templates),
            targets: Arc::new(self.targets),
            children: Arc::new(self.children),
        }
    }
}

#[derive(Default)]
pub struct FileExports<'i, 'p> {
    pub variables: Arc<VariableMap<'i, 'p>>,
    pub templates: Arc<TemplateMap<'i, 'p>>,
    pub targets: Arc<TargetMap<'i, 'p>>,
    pub children: Arc<Vec<PathBuf>>,
}

#[derive(Default)]
pub struct Environment {
    pub variables: VariableMap<'static, 'static>,
    pub templates: TemplateMap<'static, 'static>,
    pub files: Vec<Pin<Arc<AnalyzedFile>>>,
}

impl Environment {
    pub fn new() -> Self {
        Default::default()
    }
}

pub struct AnalyzedFile {
    pub document: Pin<Arc<Document>>,
    pub workspace_root: PathBuf,
    pub ast: Pin<Box<Block<'static>>>,
    pub analyzed_root: AnalyzedBlock<'static, 'static>,
    pub exports: FileExports<'static, 'static>,
    pub links_map: HashMap<PathBuf, Vec<AnalyzedLink<'static>>>,
    pub symbols: Vec<DocumentSymbol>,
    pub key: Arc<CacheKey>,
}

impl AnalyzedFile {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        document: Pin<Arc<Document>>,
        workspace_root: PathBuf,
        ast: Pin<Box<Block<'static>>>,
        analyzed_root: AnalyzedBlock<'static, 'static>,
        exports: FileExports<'static, 'static>,
        links_map: HashMap<PathBuf, Vec<AnalyzedLink<'static>>>,
        symbols: Vec<DocumentSymbol>,
        request_time: Instant,
    ) -> Pin<Arc<Self>> {
        let key = CacheKey::new(document.path.clone(), document.version, request_time);
        Arc::pin(Self {
            document,
            workspace_root,
            ast,
            analyzed_root,
            exports,
            links_map,
            symbols,
            key,
        })
    }

    pub fn local_variables_at(&self, pos: usize) -> VariableMap<'_, '_> {
        self.analyzed_root.local_variables_at(pos)
    }

    pub fn local_templates_at(&self, pos: usize) -> TemplateMap<'_, '_> {
        self.analyzed_root.local_templates_at(pos)
    }
}

#[derive(Clone)]
pub struct AnalyzedBlock<'i, 'p> {
    pub statements: Vec<AnalyzedStatement<'i, 'p>>,
    pub block: &'p Block<'i>,
    pub document: &'i Document,
    pub span: Span<'i>,
}

impl<'i, 'p> AnalyzedBlock<'i, 'p> {
    pub fn targets<'a>(&'a self) -> impl Iterator<Item = Target<'i, 'p>> + 'a {
        self.top_level_statements().filter_map(|event| match event {
            AnalyzedStatement::Target(target) => target.as_target(self.document),
            _ => None,
        })
    }

    pub fn local_variables_at(&self, pos: usize) -> VariableMap<'i, 'p> {
        let mut variables = VariableMap::new();

        // First pass: Collect all variables in the scope.
        let mut declare_args_stack: Vec<&AnalyzedDeclareArgs> = Vec::new();
        for statement in self.top_level_statements() {
            while let Some(last_declare_args) = declare_args_stack.last() {
                if statement.span().start_pos() <= last_declare_args.call.span.end_pos() {
                    break;
                }
                declare_args_stack.pop();
            }
            match statement {
                AnalyzedStatement::Assignment(assignment) => {
                    let assignment = assignment.as_variable_assignment(self.document);
                    variables
                        .entry(assignment.primary_variable.as_str())
                        .or_insert_with(|| Variable::new(!declare_args_stack.is_empty()))
                        .assignments
                        .push(assignment);
                }
                AnalyzedStatement::Foreach(foreach) => {
                    let assignment = foreach.as_variable_assignment(self.document);
                    variables
                        .entry(assignment.primary_variable.as_str())
                        .or_insert_with(|| Variable::new(!declare_args_stack.is_empty()))
                        .assignments
                        .push(assignment);
                }
                AnalyzedStatement::ForwardVariablesFrom(forward_variables_from) => {
                    for assignment in forward_variables_from.as_variable_assignment(self.document) {
                        variables
                            .entry(assignment.primary_variable.as_str())
                            .or_insert_with(|| Variable::new(!declare_args_stack.is_empty()))
                            .assignments
                            .push(assignment);
                    }
                }
                AnalyzedStatement::DeclareArgs(declare_args) => {
                    declare_args_stack.push(declare_args);
                }
                AnalyzedStatement::Import(_)
                | AnalyzedStatement::Conditions(_)
                | AnalyzedStatement::Target(_)
                | AnalyzedStatement::Template(_)
                | AnalyzedStatement::BuiltinCall(_) => {}
            }
        }

        // Second pass: Find the subscope that contains the position, and merge
        // its variables.
        for statement in self.top_level_statements() {
            for scope in statement.subscopes() {
                if scope.span.start() < pos && pos < scope.span.end() {
                    variables.extend(scope.local_variables_at(pos));
                }
            }
        }

        variables
    }

    pub fn local_templates_at(&self, pos: usize) -> TemplateMap<'i, 'p> {
        let mut templates = TemplateMap::new();

        // First pass: Collect all templates in the scope.
        for statement in self.top_level_statements() {
            match statement {
                AnalyzedStatement::Template(template) => {
                    if let Some(template) = template.as_template(self.document) {
                        templates.insert(template.name, template);
                    }
                }
                AnalyzedStatement::Import(_)
                | AnalyzedStatement::Assignment(_)
                | AnalyzedStatement::Conditions(_)
                | AnalyzedStatement::DeclareArgs(_)
                | AnalyzedStatement::Foreach(_)
                | AnalyzedStatement::ForwardVariablesFrom(_)
                | AnalyzedStatement::Target(_)
                | AnalyzedStatement::BuiltinCall(_) => {}
            }
        }

        // Second pass: Find the subscope that contains the position, and merge
        // its templates.
        for statement in self.top_level_statements() {
            for scope in statement.subscopes() {
                if scope.span.start() < pos && pos < scope.span.end() {
                    templates.extend(scope.local_templates_at(pos));
                }
            }
        }

        templates
    }
}

#[derive(Clone)]
pub enum AnalyzedStatement<'i, 'p> {
    Assignment(Box<AnalyzedAssignment<'i, 'p>>),
    Conditions(Box<AnalyzedCondition<'i, 'p>>),
    DeclareArgs(Box<AnalyzedDeclareArgs<'i, 'p>>),
    Foreach(Box<AnalyzedForeach<'i, 'p>>),
    ForwardVariablesFrom(Box<AnalyzedForwardVariablesFrom<'i, 'p>>),
    Import(Box<AnalyzedImport<'i, 'p>>),
    Target(Box<AnalyzedTarget<'i, 'p>>),
    Template(Box<AnalyzedTemplate<'i, 'p>>),
    BuiltinCall(Box<AnalyzedBuiltinCall<'i, 'p>>),
}

impl<'i, 'p> AnalyzedStatement<'i, 'p> {
    pub fn span(&self) -> Span<'i> {
        match self {
            AnalyzedStatement::Assignment(assignment) => assignment.assignment.span,
            AnalyzedStatement::Conditions(condition) => condition.condition.span,
            AnalyzedStatement::DeclareArgs(declare_args) => declare_args.call.span,
            AnalyzedStatement::Foreach(foreach) => foreach.call.span,
            AnalyzedStatement::ForwardVariablesFrom(forward_variables_from) => {
                forward_variables_from.call.span
            }
            AnalyzedStatement::Import(import) => import.call.span,
            AnalyzedStatement::Target(target) => target.call.span,
            AnalyzedStatement::Template(template) => template.call.span,
            AnalyzedStatement::BuiltinCall(builtin_call) => builtin_call.call.span,
        }
    }

    pub fn body_scope(&self) -> Option<&AnalyzedBlock<'i, 'p>> {
        match self {
            AnalyzedStatement::Target(target) => Some(&target.body_block),
            AnalyzedStatement::Template(template) => Some(&template.body_block),
            AnalyzedStatement::BuiltinCall(builtin_call) => builtin_call.body_block.as_ref(),
            AnalyzedStatement::Assignment(_)
            | AnalyzedStatement::Conditions(_)
            | AnalyzedStatement::DeclareArgs(_)
            | AnalyzedStatement::Foreach(_)
            | AnalyzedStatement::ForwardVariablesFrom(_)
            | AnalyzedStatement::Import(_) => None,
        }
    }

    pub fn expr_scopes(&self) -> impl IntoIterator<Item = &AnalyzedBlock<'i, 'p>> {
        match self {
            AnalyzedStatement::Assignment(assignment) => {
                Either::Left(assignment.expr_scopes.as_slice())
            }
            AnalyzedStatement::Conditions(condition) => {
                let mut expr_scopes = Vec::new();
                let mut current_condition = condition;
                loop {
                    expr_scopes.extend(&current_condition.expr_scopes);
                    match &current_condition.else_block {
                        Some(Either::Left(next_condition)) => {
                            current_condition = next_condition;
                        }
                        Some(Either::Right(_)) => break,
                        None => break,
                    }
                }
                Either::Right(expr_scopes)
            }
            AnalyzedStatement::Foreach(foreach) => Either::Left(foreach.expr_scopes.as_slice()),
            AnalyzedStatement::ForwardVariablesFrom(forward_variables_from) => {
                Either::Left(forward_variables_from.expr_scopes.as_slice())
            }
            AnalyzedStatement::Target(target) => Either::Left(target.expr_scopes.as_slice()),
            AnalyzedStatement::Template(template) => Either::Left(template.expr_scopes.as_slice()),
            AnalyzedStatement::BuiltinCall(builtin_call) => {
                Either::Left(builtin_call.expr_scopes.as_slice())
            }
            AnalyzedStatement::DeclareArgs(_) | AnalyzedStatement::Import(_) => {
                Either::Left([].as_slice())
            }
        }
        .into_iter()
    }

    pub fn subscopes(&self) -> impl Iterator<Item = &AnalyzedBlock<'i, 'p>> {
        self.body_scope().into_iter().chain(self.expr_scopes())
    }
}

#[derive(Clone)]
pub struct AnalyzedAssignment<'i, 'p> {
    pub assignment: &'p Assignment<'i>,
    pub primary_variable: Span<'i>,
    pub comments: Comments<'i>,
    pub expr_scopes: Vec<AnalyzedBlock<'i, 'p>>,
}

#[derive(Clone)]
pub struct AnalyzedCondition<'i, 'p> {
    pub condition: &'p Condition<'i>,
    pub expr_scopes: Vec<AnalyzedBlock<'i, 'p>>,
    pub then_block: AnalyzedBlock<'i, 'p>,
    pub else_block: Option<Either<Box<AnalyzedCondition<'i, 'p>>, Box<AnalyzedBlock<'i, 'p>>>>,
}

#[derive(Clone)]
pub struct AnalyzedDeclareArgs<'i, 'p> {
    pub call: &'p Call<'i>,
    pub body_block: AnalyzedBlock<'i, 'p>,
}

#[derive(Clone)]
pub struct AnalyzedForeach<'i, 'p> {
    pub call: &'p Call<'i>,
    pub loop_variable: &'p Identifier<'i>,
    pub loop_items: &'p Expr<'i>,
    pub expr_scopes: Vec<AnalyzedBlock<'i, 'p>>,
    pub body_block: AnalyzedBlock<'i, 'p>,
}

#[derive(Clone)]
pub struct AnalyzedForwardVariablesFrom<'i, 'p> {
    pub call: &'p Call<'i>,
    pub includes: &'p Expr<'i>,
    pub expr_scopes: Vec<AnalyzedBlock<'i, 'p>>,
}

#[derive(Clone)]
pub struct AnalyzedImport<'i, 'p> {
    pub call: &'p Call<'i>,
    pub path: PathBuf,
}

#[derive(Clone)]
pub struct AnalyzedTarget<'i, 'p> {
    pub call: &'p Call<'i>,
    pub name: &'p Expr<'i>,
    pub expr_scopes: Vec<AnalyzedBlock<'i, 'p>>,
    pub body_block: AnalyzedBlock<'i, 'p>,
}

#[derive(Clone)]
pub struct AnalyzedTemplate<'i, 'p> {
    pub call: &'p Call<'i>,
    pub name: &'p Expr<'i>,
    pub comments: Comments<'i>,
    pub expr_scopes: Vec<AnalyzedBlock<'i, 'p>>,
    pub body_block: AnalyzedBlock<'i, 'p>,
}

#[derive(Clone)]
pub struct AnalyzedBuiltinCall<'i, 'p> {
    pub call: &'p Call<'i>,
    pub expr_scopes: Vec<AnalyzedBlock<'i, 'p>>,
    pub body_block: Option<AnalyzedBlock<'i, 'p>>,
}

#[derive(Clone)]
pub struct Target<'i, 'p> {
    pub document: &'i Document,
    pub call: &'p Call<'i>,
    pub name: &'i str,
}

impl<'i, 'p> AnalyzedTarget<'i, 'p> {
    pub fn as_target(&self, document: &'i Document) -> Option<Target<'i, 'p>> {
        let name = self.name.as_simple_string()?;
        Some(Target {
            document,
            call: self.call,
            name,
        })
    }
}

#[derive(Clone)]
pub struct Template<'i, 'p> {
    pub document: &'i Document,
    pub call: &'p Call<'i>,
    pub name: &'i str,
    pub comments: Comments<'i>,
}

impl<'i, 'p> AnalyzedTemplate<'i, 'p> {
    pub fn as_template(&self, document: &'i Document) -> Option<Template<'i, 'p>> {
        let name = self.name.as_simple_string()?;
        Some(Template {
            document,
            call: self.call,
            name,
            comments: self.comments.clone(),
        })
    }
}

#[derive(Clone, Debug)]
pub struct Variable<'i, 'p> {
    pub assignments: Vec<VariableAssignment<'i, 'p>>,
    pub is_args: bool,
}

impl Variable<'_, '_> {
    pub fn new(is_args: bool) -> Self {
        Self {
            assignments: Vec::new(),
            is_args,
        }
    }
}

#[derive(Clone, Debug)]
pub struct VariableAssignment<'i, 'p> {
    pub document: &'i Document,
    pub assignment_or_call: Either<&'p Assignment<'i>, &'p Call<'i>>,
    pub primary_variable: Span<'i>,
    pub comments: Comments<'i>,
}

impl<'i, 'p> AnalyzedAssignment<'i, 'p> {
    pub fn as_variable_assignment(&self, document: &'i Document) -> VariableAssignment<'i, 'p> {
        VariableAssignment {
            document,
            assignment_or_call: Either::Left(self.assignment),
            primary_variable: self.primary_variable,
            comments: self.comments.clone(),
        }
    }
}

impl<'i, 'p> AnalyzedForeach<'i, 'p> {
    pub fn as_variable_assignment(&self, document: &'i Document) -> VariableAssignment<'i, 'p> {
        VariableAssignment {
            document,
            assignment_or_call: Either::Right(self.call),
            primary_variable: self.loop_variable.span,
            comments: Default::default(),
        }
    }
}

impl<'i, 'p> AnalyzedForwardVariablesFrom<'i, 'p> {
    pub fn as_variable_assignment(
        &self,
        document: &'i Document,
    ) -> Vec<VariableAssignment<'i, 'p>> {
        // TODO: Handle excludes.
        let Some(strings) = self.includes.as_primary_list().map(|list| {
            list.values
                .iter()
                .filter_map(|expr| expr.as_primary_string())
                .collect::<Vec<_>>()
        }) else {
            return Vec::new();
        };
        strings
            .into_iter()
            .filter_map(|string| {
                parse_simple_literal(string.raw_value).map(|_| {
                    let primary_variable = Span::new(
                        string.span.get_input(),
                        string.span.start() + 1,
                        string.span.end() - 1,
                    )
                    .unwrap();
                    VariableAssignment {
                        document,
                        assignment_or_call: Either::Right(self.call),
                        primary_variable,
                        comments: Default::default(),
                    }
                })
            })
            .collect()
    }
}

#[derive(Clone, Eq, PartialEq)]
pub enum AnalyzedLink<'i> {
    /// Link to a file. No range is specified.
    File { path: PathBuf, span: Span<'i> },
    /// Link to a target defined in a BUILD.gn file.
    Target {
        path: PathBuf,
        name: &'i str,
        span: Span<'i>,
    },
}

impl<'i> AnalyzedLink<'i> {
    pub fn path(&self) -> &Path {
        match self {
            AnalyzedLink::File { path, .. } => path,
            AnalyzedLink::Target { path, .. } => path,
        }
    }

    pub fn span(&self) -> Span<'i> {
        match self {
            AnalyzedLink::File { span, .. } => *span,
            AnalyzedLink::Target { span, .. } => *span,
        }
    }
}
