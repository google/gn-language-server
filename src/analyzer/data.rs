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
    sync::Arc,
    time::Instant,
};

use either::Either;
use pest::Span;
use self_cell::self_cell;
use tower_lsp::lsp_types::{DocumentSymbol, Url};

use crate::{
    analyzer::{cache::CacheKey, toplevel::TopLevelStatementsExt, utils::resolve_path},
    common::{
        builtins::{FOREACH, FORWARD_VARIABLES_FROM},
        storage::{Document, DocumentVersion},
        utils::{format_path, parse_simple_literal},
        workspace::find_nearest_workspace_root,
    },
    parser::{
        Assignment, Call, Comments, Condition, ErrorStatement, Expr, Identifier, Node, OwnedBlock,
    },
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

pub type StrKeyedMap<'p, T> = HashMap<&'p str, T>;
pub type VariableMap<'p> = StrKeyedMap<'p, Variable<'p>>;
pub type TemplateMap<'p> = StrKeyedMap<'p, Template<'p>>;
pub type TargetMap<'p> = StrKeyedMap<'p, Target<'p>>;

#[derive(Default)]
pub struct FileExports<'p> {
    pub variables: VariableMap<'p>,
    pub templates: TemplateMap<'p>,
    pub targets: TargetMap<'p>,
    pub children: Vec<PathBuf>,
}

impl FileExports<'_> {
    pub fn new() -> Self {
        Default::default()
    }
}

self_cell!(
    struct FileExportsSelfCell {
        owner: OwnedBlock,
        #[covariant]
        dependent: FileExports,
    }
);

pub struct OwnedFileExports(FileExportsSelfCell);

impl OwnedFileExports {
    pub fn new(block: OwnedBlock, builder: impl FnOnce(&OwnedBlock) -> FileExports<'_>) -> Self {
        Self(FileExportsSelfCell::new(block, builder))
    }

    pub fn get(&self) -> &FileExports<'_> {
        self.0.borrow_dependent()
    }
}

#[derive(Default)]
pub struct Environment<'a> {
    pub variables: VariableMap<'a>,
    pub templates: TemplateMap<'a>,
}

impl Environment<'_> {
    pub fn new() -> Self {
        Default::default()
    }
}

self_cell!(
    struct EnvironmentSelfCell {
        owner: Vec<Arc<AnalyzedFile>>,
        #[covariant]
        dependent: Environment,
    }
);

pub struct OwnedEnvironment(EnvironmentSelfCell);

impl OwnedEnvironment {
    pub fn new(
        files: Vec<Arc<AnalyzedFile>>,
        builder: impl FnOnce(&Vec<Arc<AnalyzedFile>>) -> Environment<'_>,
    ) -> Self {
        Self(EnvironmentSelfCell::new(files, builder))
    }

    pub fn get(&self) -> &Environment<'_> {
        self.0.borrow_dependent()
    }
}

pub struct AnalyzedFile {
    pub document: Arc<Document>,
    pub workspace_root: PathBuf,
    pub ast: OwnedBlock,
    pub analyzed_root: OwnedAnalyzedBlock,
    pub exports: OwnedFileExports,
    pub link_index: OwnedLinkIndex,
    pub outline: Vec<DocumentSymbol>,
    pub external: bool,
    pub key: Arc<CacheKey>,
}

impl AnalyzedFile {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        document: Arc<Document>,
        workspace_root: PathBuf,
        ast: OwnedBlock,
        analyzed_root: OwnedAnalyzedBlock,
        exports: OwnedFileExports,
        link_index: OwnedLinkIndex,
        outline: Vec<DocumentSymbol>,
        request_time: Instant,
    ) -> Self {
        let external =
            find_nearest_workspace_root(&document.path).is_none_or(|path| path != workspace_root);
        let key = CacheKey::new(document.path.clone(), document.version, request_time);
        Self {
            document,
            workspace_root,
            ast,
            analyzed_root,
            exports,
            link_index,
            outline,
            external,
            key,
        }
    }

    pub fn local_variables_at(&self, pos: usize) -> VariableMap<'_> {
        self.analyzed_root.get().local_variables_at(pos)
    }

    pub fn local_templates_at(&self, pos: usize) -> TemplateMap<'_> {
        self.analyzed_root.get().local_templates_at(pos)
    }
}

self_cell!(
    struct AnalyzedBlockSelfCell {
        owner: OwnedBlock,
        #[covariant]
        dependent: AnalyzedBlock,
    }
);

#[derive(Clone)]
pub struct OwnedAnalyzedBlock(Arc<AnalyzedBlockSelfCell>);

impl OwnedAnalyzedBlock {
    pub fn new(block: OwnedBlock, builder: impl FnOnce(&OwnedBlock) -> AnalyzedBlock<'_>) -> Self {
        Self(Arc::new(AnalyzedBlockSelfCell::new(block, builder)))
    }

    pub fn get(&self) -> &AnalyzedBlock<'_> {
        self.0.borrow_dependent()
    }
}

#[derive(Clone)]
pub struct AnalyzedBlock<'p> {
    pub statements: Vec<AnalyzedStatement<'p>>,
    pub document: &'p Document,
    pub span: Span<'p>,
}

impl<'p> AnalyzedBlock<'p> {
    pub fn targets<'a>(&'a self) -> impl Iterator<Item = Target<'p>> + 'a {
        self.top_level_statements().filter_map(|event| match event {
            AnalyzedStatement::Target(target) => target.as_target(self.document),
            _ => None,
        })
    }

    pub fn local_variables_at(&self, pos: usize) -> VariableMap<'p> {
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
                        .or_insert_with(|| {
                            Variable::new(
                                assignment.primary_variable.as_str(),
                                !declare_args_stack.is_empty(),
                            )
                        })
                        .assignments
                        .push(assignment);
                }
                AnalyzedStatement::Foreach(foreach) => {
                    let assignment = foreach.as_variable_assignment(self.document);
                    variables
                        .entry(assignment.primary_variable.as_str())
                        .or_insert_with(|| {
                            Variable::new(
                                assignment.primary_variable.as_str(),
                                !declare_args_stack.is_empty(),
                            )
                        })
                        .assignments
                        .push(assignment);
                }
                AnalyzedStatement::ForwardVariablesFrom(forward_variables_from) => {
                    for assignment in forward_variables_from.as_variable_assignment(self.document) {
                        variables
                            .entry(assignment.primary_variable.as_str())
                            .or_insert_with(|| {
                                Variable::new(
                                    assignment.primary_variable.as_str(),
                                    !declare_args_stack.is_empty(),
                                )
                            })
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
                | AnalyzedStatement::BuiltinCall(_)
                | AnalyzedStatement::Error(_) => {}
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

    pub fn local_templates_at(&self, pos: usize) -> TemplateMap<'p> {
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
                | AnalyzedStatement::BuiltinCall(_)
                | AnalyzedStatement::Error(_) => {}
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
pub enum AnalyzedStatement<'p> {
    Assignment(Box<AnalyzedAssignment<'p>>),
    Conditions(Box<AnalyzedCondition<'p>>),
    DeclareArgs(Box<AnalyzedDeclareArgs<'p>>),
    Foreach(Box<AnalyzedForeach<'p>>),
    ForwardVariablesFrom(Box<AnalyzedForwardVariablesFrom<'p>>),
    Import(Box<AnalyzedImport<'p>>),
    Target(Box<AnalyzedTarget<'p>>),
    Template(Box<AnalyzedTemplate<'p>>),
    BuiltinCall(Box<AnalyzedBuiltinCall<'p>>),
    Error(&'p ErrorStatement<'p>),
}

impl<'p> AnalyzedStatement<'p> {
    pub fn span(&self) -> Span<'p> {
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
            AnalyzedStatement::Error(statement) => statement.span(),
        }
    }

    pub fn body_scope(&self) -> Option<&AnalyzedBlock<'p>> {
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
            | AnalyzedStatement::Error(_) => None,
        }
    }

    pub fn expr_scopes(&self) -> impl IntoIterator<Item = &AnalyzedBlock<'p>> {
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
            | AnalyzedStatement::Error(_) => Either::Left([].as_slice()),
        }
        .into_iter()
    }

    pub fn subscopes(&self) -> impl Iterator<Item = &AnalyzedBlock<'p>> {
        self.body_scope().into_iter().chain(self.expr_scopes())
    }
}

#[derive(Clone)]
pub struct AnalyzedAssignment<'p> {
    pub assignment: &'p Assignment<'p>,
    pub primary_variable: Span<'p>,
    pub comments: Comments<'p>,
    pub expr_scopes: Vec<AnalyzedBlock<'p>>,
}

#[derive(Clone)]
pub struct AnalyzedCondition<'p> {
    pub condition: &'p Condition<'p>,
    pub expr_scopes: Vec<AnalyzedBlock<'p>>,
    pub then_block: AnalyzedBlock<'p>,
    pub else_block: Option<Either<Box<AnalyzedCondition<'p>>, Box<AnalyzedBlock<'p>>>>,
}

#[derive(Clone)]
pub struct AnalyzedDeclareArgs<'p> {
    pub call: &'p Call<'p>,
    pub body_block: AnalyzedBlock<'p>,
}

#[derive(Clone)]
pub struct AnalyzedForeach<'p> {
    pub call: &'p Call<'p>,
    pub loop_variable: &'p Identifier<'p>,
    pub loop_items: &'p Expr<'p>,
    pub expr_scopes: Vec<AnalyzedBlock<'p>>,
    pub body_block: AnalyzedBlock<'p>,
}

#[derive(Clone)]
pub struct AnalyzedForwardVariablesFrom<'p> {
    pub call: &'p Call<'p>,
    pub includes: &'p Expr<'p>,
    pub expr_scopes: Vec<AnalyzedBlock<'p>>,
}

#[derive(Clone)]
pub struct AnalyzedImport<'p> {
    pub call: &'p Call<'p>,
    pub name: &'p str,
    pub path: PathBuf,
}

#[derive(Clone)]
pub struct AnalyzedTarget<'p> {
    pub call: &'p Call<'p>,
    pub name: &'p Expr<'p>,
    pub expr_scopes: Vec<AnalyzedBlock<'p>>,
    pub body_block: AnalyzedBlock<'p>,
}

#[derive(Clone)]
pub struct AnalyzedTemplate<'p> {
    pub call: &'p Call<'p>,
    pub name: &'p Expr<'p>,
    pub comments: Comments<'p>,
    pub expr_scopes: Vec<AnalyzedBlock<'p>>,
    pub body_block: AnalyzedBlock<'p>,
}

#[derive(Clone)]
pub struct AnalyzedBuiltinCall<'p> {
    pub call: &'p Call<'p>,
    pub expr_scopes: Vec<AnalyzedBlock<'p>>,
    pub body_block: Option<AnalyzedBlock<'p>>,
}

#[derive(Clone)]
pub struct Target<'p> {
    pub document: &'p Document,
    pub call: &'p Call<'p>,
    pub name: &'p str,
}

impl<'p> AnalyzedTarget<'p> {
    pub fn as_target(&self, document: &'p Document) -> Option<Target<'p>> {
        let name = self.name.as_simple_string()?;
        Some(Target {
            document,
            call: self.call,
            name,
        })
    }
}

#[derive(Clone)]
pub struct Template<'p> {
    pub document: &'p Document,
    pub call: &'p Call<'p>,
    pub name: &'p str,
    pub comments: Comments<'p>,
}

impl Template<'_> {
    pub fn format_help(&self, workspace_root: &Path) -> Vec<String> {
        let mut paragraphs = vec![format!("```gn\ntemplate(\"{}\") {{ ... }}\n```", self.name)];
        if !self.comments.is_empty() {
            paragraphs.push(format!(
                "```text\n{}\n```",
                self.comments.to_string().trim()
            ));
        };
        let position = self
            .document
            .line_index
            .position(self.call.function.span.start());
        paragraphs.push(format!(
            "Defined at [{}:{}:{}]({}#L{},{})",
            format_path(&self.document.path, workspace_root),
            position.line + 1,
            position.character + 1,
            Url::from_file_path(&self.document.path).unwrap(),
            position.line + 1,
            position.character + 1,
        ));

        paragraphs
    }
}

impl<'p> AnalyzedTemplate<'p> {
    pub fn as_template(&self, document: &'p Document) -> Option<Template<'p>> {
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
pub struct Variable<'p> {
    pub name: &'p str,
    pub assignments: Vec<VariableAssignment<'p>>,
    pub is_args: bool,
}

impl<'p> Variable<'p> {
    pub fn new(name: &'p str, is_args: bool) -> Self {
        Self {
            name,
            assignments: Vec::new(),
            is_args,
        }
    }

    pub fn format_help(&self, workspace_root: &Path) -> Vec<String> {
        let first_assignment = self.assignments.first().unwrap();
        let single_assignment = self.assignments.len() == 1;

        let snippet = if single_assignment {
            match first_assignment.assignment_or_call {
                Either::Left(assignment) => {
                    let raw_value = assignment.rvalue.span().as_str();
                    let display_value = if raw_value.lines().count() <= 5 {
                        raw_value
                    } else {
                        "..."
                    };
                    format!(
                        "{} {} {}",
                        assignment.lvalue.span().as_str(),
                        assignment.op,
                        display_value
                    )
                }
                Either::Right(call) => {
                    match call.function.name {
                        FORWARD_VARIABLES_FROM => call.span.as_str().to_string(),
                        // TODO: Include the entire foreach call (without block)
                        FOREACH => call.args[0].span().as_str().to_string(),
                        _ => panic!("Unexpected assignment: {}", call.function.name),
                    }
                }
            }
        } else {
            format!("{} = ...", first_assignment.primary_variable.as_str())
        };

        let mut paragraphs = vec![format!("```gn\n{snippet}\n```")];

        if single_assignment {
            paragraphs.push(format!(
                "```text\n{}\n```",
                first_assignment.comments.to_string().trim()
            ));
        }

        let span = match &first_assignment.assignment_or_call {
            Either::Left(assignment) => assignment.span,
            Either::Right(call) => call.span,
        };
        let position = first_assignment.document.line_index.position(span.start());
        paragraphs.push(if single_assignment {
            format!(
                "Defined at [{}:{}:{}]({}#L{},{})",
                format_path(&first_assignment.document.path, workspace_root),
                position.line + 1,
                position.character + 1,
                Url::from_file_path(&first_assignment.document.path).unwrap(),
                position.line + 1,
                position.character + 1,
            )
        } else {
            format!(
                "Defined and modified in {} locations",
                self.assignments.len()
            )
        });

        paragraphs
    }
}

#[derive(Clone, Debug)]
pub struct VariableAssignment<'p> {
    pub document: &'p Document,
    pub assignment_or_call: Either<&'p Assignment<'p>, &'p Call<'p>>,
    pub primary_variable: Span<'p>,
    pub comments: Comments<'p>,
}

impl<'p> AnalyzedAssignment<'p> {
    pub fn as_variable_assignment(&self, document: &'p Document) -> VariableAssignment<'p> {
        VariableAssignment {
            document,
            assignment_or_call: Either::Left(self.assignment),
            primary_variable: self.primary_variable,
            comments: self.comments.clone(),
        }
    }
}

impl<'p> AnalyzedForeach<'p> {
    pub fn as_variable_assignment(&self, document: &'p Document) -> VariableAssignment<'p> {
        VariableAssignment {
            document,
            assignment_or_call: Either::Right(self.call),
            primary_variable: self.loop_variable.span,
            comments: Default::default(),
        }
    }
}

impl<'p> AnalyzedForwardVariablesFrom<'p> {
    pub fn as_variable_assignment(&self, document: &'p Document) -> Vec<VariableAssignment<'p>> {
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
pub enum AnalyzedLink<'p> {
    /// Link to a file. No range is specified.
    File { path: PathBuf, span: Span<'p> },
    /// Link to a target defined in a BUILD.gn file.
    Target {
        path: PathBuf,
        name: &'p str,
        span: Span<'p>,
    },
}

impl<'p> AnalyzedLink<'p> {
    pub fn path(&self) -> &Path {
        match self {
            AnalyzedLink::File { path, .. } => path,
            AnalyzedLink::Target { path, .. } => path,
        }
    }

    pub fn span(&self) -> Span<'p> {
        match self {
            AnalyzedLink::File { span, .. } => *span,
            AnalyzedLink::Target { span, .. } => *span,
        }
    }
}

pub type LinkIndex<'p> = HashMap<PathBuf, Vec<AnalyzedLink<'p>>>;

self_cell!(
    struct LinkIndexSelfCell {
        owner: OwnedBlock,
        #[covariant]
        dependent: LinkIndex,
    }
);

pub struct OwnedLinkIndex(LinkIndexSelfCell);

impl OwnedLinkIndex {
    pub fn new(block: OwnedBlock, builder: impl FnOnce(&OwnedBlock) -> LinkIndex<'_>) -> Self {
        Self(LinkIndexSelfCell::new(block, builder))
    }

    pub fn get(&self) -> &LinkIndex<'_> {
        self.0.borrow_dependent()
    }
}
