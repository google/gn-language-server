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
    collections::{BTreeSet, HashMap},
    path::{Path, PathBuf},
    pin::Pin,
    sync::Arc,
    time::Instant,
};

use either::Either;
use pest::Span;
use tower_lsp::lsp_types::DocumentSymbol;

use crate::{
    analyzer::{cache::CacheNode, toplevel::TopLevelStatementsExt, utils::resolve_path},
    common::{
        storage::{Document, DocumentVersion},
        utils::parse_simple_literal,
    },
    parser::{parse, Assignment, Block, Call, Comments, Condition, Expr, Identifier},
};

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct PathSpan<'i> {
    pub path: &'i Path,
    pub span: Span<'i>,
}

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

#[derive(Clone)]
pub struct Environment<'i, T> {
    imports: Vec<Arc<Environment<'i, T>>>,
    locals: HashMap<&'i str, T>,
}

impl<T> Default for Environment<'_, T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'i, T> Environment<'i, T> {
    pub fn new() -> Self {
        Self {
            imports: Vec::new(),
            locals: HashMap::new(),
        }
    }

    pub fn locals(&self) -> &HashMap<&'i str, T> {
        &self.locals
    }

    pub fn get(&self, name: &str) -> Option<&T> {
        self.locals
            .get(name)
            .or_else(|| self.imports.iter().find_map(|import| import.get(name)))
    }

    pub fn contains(&self, name: &str) -> bool {
        self.locals.contains_key(name) || self.imports.iter().any(|import| import.contains(name))
    }

    pub fn import(&mut self, other: &Arc<Environment<'i, T>>) {
        self.imports.push(Arc::clone(other));
    }

    pub fn insert(&mut self, name: &'i str, item: T) {
        self.locals.insert(name, item);
    }

    pub fn ensure(&mut self, name: &'i str, f: impl FnOnce() -> T) -> &mut T {
        self.locals.entry(name).or_insert_with(f)
    }

    pub fn merge(&mut self, other: Environment<'i, T>) {
        for (name, item) in other.locals {
            self.insert(name, item);
        }
        self.imports.extend(other.imports);
    }
}

impl<'i, T> Environment<'i, T>
where
    T: Clone,
{
    pub fn all_items(&self) -> HashMap<&'i str, T> {
        let mut items = HashMap::new();
        self.collect_items(&mut items, &mut Default::default());
        items
    }

    fn collect_items<'e>(
        &'e self,
        items: &mut HashMap<&'i str, T>,
        visited: &mut BTreeSet<*const Self>,
    ) {
        if !visited.insert(self as *const Self) {
            return;
        }
        for import in &self.imports {
            import.collect_items(items, visited);
        }
        for (name, item) in &self.locals {
            items.insert(name, item.clone());
        }
    }
}

pub struct ShallowAnalyzedFile {
    pub document: Pin<Arc<Document>>,
    #[allow(unused)] // Backing environment
    pub ast: Pin<Box<Block<'static>>>,
    pub environment: FileEnvironment<'static, 'static>,
    pub links: Vec<AnalyzedLink<'static>>,
    pub node: Arc<CacheNode>,
}

impl ShallowAnalyzedFile {
    pub fn new(
        document: Pin<Arc<Document>>,
        ast: Pin<Box<Block<'static>>>,
        environment: FileEnvironment<'static, 'static>,
        links: Vec<AnalyzedLink<'static>>,
        deps: Vec<Arc<CacheNode>>,
        request_time: Instant,
    ) -> Pin<Arc<Self>> {
        let node = CacheNode::new(document.path.clone(), document.version, deps, request_time);
        Arc::pin(Self {
            document,
            ast,
            environment,
            links,
            node,
        })
    }

    pub fn error(path: &Path, request_time: Instant) -> Pin<Arc<Self>> {
        let document = Arc::pin(Document::analysis_error(path));
        let ast = Box::pin(parse(&document.data));
        let environment = FileEnvironment::new();
        // SAFETY: ast's contents are backed by pinned document.
        let ast = unsafe { std::mem::transmute::<Pin<Box<Block>>, Pin<Box<Block>>>(ast) };
        // SAFETY: environment's contents are backed by pinned document and pinned ast.
        let environment =
            unsafe { std::mem::transmute::<FileEnvironment, FileEnvironment>(environment) };
        Self::new(
            document,
            ast,
            environment,
            Vec::new(),
            Vec::new(),
            request_time,
        )
    }
}

#[derive(Default)]
pub struct MutableFileEnvironment<'i, 'p> {
    pub variables: VariableScope<'i, 'p>,
    pub templates: TemplateScope<'i, 'p>,
    pub targets: TargetScope<'i, 'p>,
}

impl MutableFileEnvironment<'_, '_> {
    pub fn new() -> Self {
        Default::default()
    }
}

impl<'i, 'p> MutableFileEnvironment<'i, 'p> {
    pub fn import(&mut self, env: &FileEnvironment<'i, 'p>) {
        self.variables.import(&env.variables);
        self.templates.import(&env.templates);
        self.targets.import(&env.targets);
    }

    pub fn finalize(self) -> FileEnvironment<'i, 'p> {
        FileEnvironment {
            variables: Arc::new(self.variables),
            templates: Arc::new(self.templates),
            targets: Arc::new(self.targets),
        }
    }
}

#[derive(Default)]
pub struct FileEnvironment<'i, 'p> {
    pub variables: Arc<VariableScope<'i, 'p>>,
    pub templates: Arc<TemplateScope<'i, 'p>>,
    pub targets: Arc<TargetScope<'i, 'p>>,
}

impl FileEnvironment<'_, '_> {
    pub fn new() -> Self {
        Default::default()
    }
}

pub struct AnalyzedFile {
    pub document: Pin<Arc<Document>>,
    pub workspace_root: PathBuf,
    pub ast: Pin<Box<Block<'static>>>,
    pub analyzed_root: AnalyzedBlock<'static, 'static>,
    pub links: Vec<AnalyzedLink<'static>>,
    pub symbols: Vec<DocumentSymbol>,
    pub node: Arc<CacheNode>,
}

impl AnalyzedFile {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        document: Pin<Arc<Document>>,
        workspace_root: PathBuf,
        ast: Pin<Box<Block<'static>>>,
        analyzed_root: AnalyzedBlock<'static, 'static>,
        links: Vec<AnalyzedLink<'static>>,
        symbols: Vec<DocumentSymbol>,
        deps: Vec<Arc<CacheNode>>,
        request_time: Instant,
    ) -> Pin<Arc<Self>> {
        let node = CacheNode::new(document.path.clone(), document.version, deps, request_time);
        Arc::pin(Self {
            document,
            workspace_root,
            ast,
            analyzed_root,
            links,
            symbols,
            node,
        })
    }

    pub fn variables_at(&self, pos: usize) -> VariableScope<'_, '_> {
        self.analyzed_root.variables_at(pos)
    }

    pub fn templates_at(&self, pos: usize) -> TemplateScope<'_, '_> {
        self.analyzed_root.templates_at(pos)
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

    pub fn variables_at(&self, pos: usize) -> VariableScope<'i, 'p> {
        let mut variables = VariableScope::new();

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
                        .ensure(assignment.primary_variable.as_str(), || {
                            Variable::new(!declare_args_stack.is_empty())
                        })
                        .assignments
                        .insert(
                            PathSpan {
                                path: &assignment.document.path,
                                span: assignment.primary_variable,
                            },
                            assignment,
                        );
                }
                AnalyzedStatement::Foreach(foreach) => {
                    let assignment = foreach.as_variable_assignment(self.document);
                    variables
                        .ensure(assignment.primary_variable.as_str(), || {
                            Variable::new(!declare_args_stack.is_empty())
                        })
                        .assignments
                        .insert(
                            PathSpan {
                                path: &assignment.document.path,
                                span: assignment.primary_variable,
                            },
                            assignment,
                        );
                }
                AnalyzedStatement::ForwardVariablesFrom(forward_variables_from) => {
                    for assignment in forward_variables_from.as_variable_assignment(self.document) {
                        variables
                            .ensure(assignment.primary_variable.as_str(), || {
                                Variable::new(!declare_args_stack.is_empty())
                            })
                            .assignments
                            .insert(
                                PathSpan {
                                    path: &assignment.document.path,
                                    span: assignment.primary_variable,
                                },
                                assignment,
                            );
                    }
                }
                AnalyzedStatement::Import(import) => {
                    // TODO: Handle import() within declare_args.
                    variables.import(&import.file.environment.variables);
                }
                AnalyzedStatement::SyntheticImport(import) => {
                    variables.import(&import.file.environment.variables);
                }
                AnalyzedStatement::DeclareArgs(declare_args) => {
                    declare_args_stack.push(declare_args);
                }
                AnalyzedStatement::Conditions(_)
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
                    variables.merge(scope.variables_at(pos));
                }
            }
        }

        variables
    }

    pub fn templates_at(&self, pos: usize) -> TemplateScope<'i, 'p> {
        let mut templates = TemplateScope::new();

        // First pass: Collect all templates in the scope.
        for statement in self.top_level_statements() {
            match statement {
                AnalyzedStatement::Template(template) => {
                    if let Some(template) = template.as_template(self.document) {
                        templates.insert(template.name, template);
                    }
                }
                AnalyzedStatement::Import(import) => {
                    templates.import(&import.file.environment.templates);
                }
                AnalyzedStatement::SyntheticImport(import) => {
                    templates.import(&import.file.environment.templates);
                }
                AnalyzedStatement::Assignment(_)
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
                    templates.merge(scope.templates_at(pos));
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
    SyntheticImport(Box<SyntheticImport<'i>>),
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
            AnalyzedStatement::SyntheticImport(synthetic_import) => synthetic_import.span,
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
            | AnalyzedStatement::Import(_)
            | AnalyzedStatement::SyntheticImport(_) => None,
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
            AnalyzedStatement::DeclareArgs(_)
            | AnalyzedStatement::Import(_)
            | AnalyzedStatement::SyntheticImport(_) => Either::Left([].as_slice()),
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
    pub file: Pin<Arc<ShallowAnalyzedFile>>,
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
pub struct SyntheticImport<'i> {
    pub file: Pin<Arc<ShallowAnalyzedFile>>,
    pub span: Span<'i>,
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
    pub assignments: HashMap<PathSpan<'i>, VariableAssignment<'i, 'p>>,
    pub is_args: bool,
}

impl Variable<'_, '_> {
    pub fn new(is_args: bool) -> Self {
        Self {
            assignments: HashMap::new(),
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

pub type TargetScope<'i, 'p> = Environment<'i, Target<'i, 'p>>;
pub type TemplateScope<'i, 'p> = Environment<'i, Template<'i, 'p>>;
pub type VariableScope<'i, 'p> = Environment<'i, Variable<'i, 'p>>;

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
    pub fn span(&self) -> Span<'i> {
        match self {
            AnalyzedLink::File { span, .. } => *span,
            AnalyzedLink::Target { span, .. } => *span,
        }
    }
}
