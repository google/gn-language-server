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
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    pin::Pin,
    sync::Arc,
};

use pest::Span;
use tower_lsp::lsp_types::DocumentSymbol;

use crate::{
    ast::{parse, Block, Comments, Statement},
    storage::{Document, DocumentStorage},
};

pub fn find_workspace_root(path: &Path) -> std::io::Result<&Path> {
    for dir in path.ancestors().skip(1) {
        if dir.join(".gn").try_exists()? {
            return Ok(dir);
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::NotFound,
        format!("Workspace not found for {}", path.to_string_lossy()),
    ))
}

pub fn resolve_path(name: &str, root_dir: &Path, current_dir: &Path) -> PathBuf {
    if let Some(rest) = name.strip_prefix("//") {
        root_dir.join(rest)
    } else {
        current_dir.join(name)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct WorkspaceContext {
    pub root: PathBuf,
    pub build_config: PathBuf,
}

impl WorkspaceContext {
    pub fn resolve_path(&self, name: &str, current_dir: &Path) -> PathBuf {
        resolve_path(name, &self.root, current_dir)
    }

    pub fn format_path(&self, path: &Path) -> String {
        if let Ok(relative_path) = path.strip_prefix(&self.root) {
            format!("//{}", relative_path.to_string_lossy())
        } else {
            path.to_string_lossy().to_string()
        }
    }
}

pub struct ShallowAnalyzedFile {
    pub document: Pin<Arc<Document>>,
    pub workspace: WorkspaceContext,
    #[allow(unused)] // Backing analyzed_root
    pub ast_root: Pin<Box<Block<'static>>>,
    pub analyzed_root: ShallowAnalyzedBlock<'static, 'static>,
    pub deps: Vec<Pin<Arc<ShallowAnalyzedFile>>>,
}

impl ShallowAnalyzedFile {
    pub fn empty(path: &Path, workspace: &WorkspaceContext) -> Pin<Arc<Self>> {
        let document = Arc::pin(Document::empty(path));
        let ast_root = Box::pin(parse(&document.data));
        let analyzed_root = ShallowAnalyzedBlock::new_top_level();
        // SAFETY: ast_root's contents are backed by pinned document.
        let ast_root = unsafe { std::mem::transmute::<Pin<Box<Block>>, Pin<Box<Block>>>(ast_root) };
        // SAFETY: analyzed_root's contents are backed by pinned document and pinned ast_root.
        let analyzed_root = unsafe {
            std::mem::transmute::<ShallowAnalyzedBlock, ShallowAnalyzedBlock>(analyzed_root)
        };
        Arc::pin(ShallowAnalyzedFile {
            document,
            workspace: workspace.clone(),
            ast_root,
            analyzed_root,
            deps: Vec::new(),
        })
    }

    pub fn is_fresh(&self, storage: &DocumentStorage) -> std::io::Result<bool> {
        if self.document.will_check() {
            let version = storage.read_version(&self.document.path)?;
            if version != self.document.version {
                return Ok(false);
            }
        }
        for dep in &self.deps {
            if !dep.is_fresh(storage)? {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

pub struct ShallowAnalyzedBlock<'i, 'p> {
    pub scope: AnalyzedScope<'i, 'p>,
    pub templates: HashSet<AnalyzedTemplate<'i>>,
    pub targets: HashSet<AnalyzedTarget<'i>>,
}

impl ShallowAnalyzedBlock<'_, '_> {
    pub fn new_top_level() -> Self {
        ShallowAnalyzedBlock {
            scope: AnalyzedScope::new(None),
            templates: HashSet::new(),
            targets: HashSet::new(),
        }
    }

    pub fn merge(&mut self, other: &Self) {
        self.scope.merge(&other.scope);
        self.templates.extend(other.templates.clone());
        self.targets.extend(other.targets.clone());
    }
}

pub struct AnalyzedFile {
    pub document: Pin<Arc<Document>>,
    pub workspace: WorkspaceContext,
    pub ast_root: Pin<Box<Block<'static>>>,
    pub analyzed_root: AnalyzedBlock<'static, 'static>,
    pub deps: Vec<Pin<Arc<ShallowAnalyzedFile>>>,
    pub links: Vec<Link<'static>>,
    pub symbols: Vec<DocumentSymbol>,
}

impl AnalyzedFile {
    pub fn is_fresh(&self, storage: &DocumentStorage) -> std::io::Result<bool> {
        if self.document.will_check() {
            let version = storage.read_version(&self.document.path)?;
            if version != self.document.version {
                return Ok(false);
            }
        }
        for dep in &self.deps {
            if !dep.is_fresh(storage)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    pub fn scope_at(&self, pos: usize) -> AnalyzedScope {
        self.analyzed_root.scope_at(pos, None)
    }

    pub fn templates_at(&self, pos: usize) -> HashSet<&AnalyzedTemplate> {
        self.analyzed_root.templates_at(pos)
    }

    pub fn targets_at(&self, pos: usize) -> HashSet<&AnalyzedTarget> {
        self.analyzed_root.targets_at(pos)
    }
}

pub struct AnalyzedBlock<'i, 'p> {
    pub events: Vec<AnalyzedEvent<'i, 'p>>,
    pub span: Span<'i>,
}

impl<'i, 'p> AnalyzedBlock<'i, 'p> {
    pub fn top_level_events<'a>(&'a self) -> TopLevelEvents<'i, 'p, 'a> {
        TopLevelEvents::new(&self.events)
    }

    pub fn scope_at(
        &self,
        pos: usize,
        parent: Option<Box<AnalyzedScope<'i, 'p>>>,
    ) -> AnalyzedScope<'i, 'p> {
        let mut scope = AnalyzedScope::new(parent);

        // First pass: Collect all variables in the scope.
        for event in self.top_level_events() {
            match event {
                AnalyzedEvent::Assignment(assignment) => {
                    scope.insert(assignment.clone());
                }
                AnalyzedEvent::Import(import) => {
                    scope.merge(&import.file.analyzed_root.scope);
                }
                _ => {}
            }
        }

        // Second pass: Find the subscope that contains the position.
        for event in self.top_level_events() {
            if let AnalyzedEvent::NewScope(block) = event {
                if block.span.start() < pos && pos < block.span.end() {
                    return block.scope_at(pos, Some(Box::new(scope)));
                }
            }
        }

        scope
    }

    pub fn templates_at(&'i self, pos: usize) -> HashSet<&'i AnalyzedTemplate<'i>> {
        let mut templates = HashSet::new();
        for event in &self.events {
            match event {
                AnalyzedEvent::Conditions(blocks) => {
                    if blocks.last().unwrap().span.end() <= pos {
                        for block in blocks {
                            templates.extend(block.templates_at(pos));
                        }
                    } else {
                        for block in blocks {
                            if block.span.start() <= pos && pos <= block.span.end() {
                                templates.extend(block.templates_at(pos));
                            }
                        }
                    }
                }
                AnalyzedEvent::Import(import) => {
                    if import.span.end() <= pos {
                        templates.extend(import.file.analyzed_root.templates.iter());
                    }
                }
                AnalyzedEvent::Template(template) => {
                    if template.span.end() <= pos {
                        templates.insert(template);
                    }
                }
                AnalyzedEvent::NewScope(block) => {
                    if block.span.start() <= pos && pos <= block.span.end() {
                        templates.extend(block.templates_at(pos));
                    }
                }
                AnalyzedEvent::Assignment(_)
                | AnalyzedEvent::DeclareArgs(_)
                | AnalyzedEvent::Target(_) => {}
            }
        }
        templates
    }

    pub fn targets_at(&'i self, pos: usize) -> HashSet<&'i AnalyzedTarget<'i>> {
        let mut targets = HashSet::new();
        for event in &self.events {
            match event {
                AnalyzedEvent::Conditions(blocks) => {
                    if blocks.last().unwrap().span.end() <= pos {
                        for block in blocks {
                            targets.extend(block.targets_at(pos));
                        }
                    } else {
                        for block in blocks {
                            if block.span.start() <= pos && pos <= block.span.end() {
                                targets.extend(block.targets_at(pos));
                            }
                        }
                    }
                }
                AnalyzedEvent::Import(import) => {
                    if import.span.end() <= pos {
                        targets.extend(import.file.analyzed_root.targets.iter());
                    }
                }
                AnalyzedEvent::Target(target) => {
                    if target.span.end() <= pos {
                        targets.insert(target);
                    }
                }
                AnalyzedEvent::NewScope(block) => {
                    if block.span.start() <= pos && pos <= block.span.end() {
                        targets.extend(block.targets_at(pos));
                    }
                }
                AnalyzedEvent::Assignment(_)
                | AnalyzedEvent::DeclareArgs(_)
                | AnalyzedEvent::Template(_) => {}
            }
        }
        targets
    }
}

pub struct TopLevelEvents<'i, 'p, 'a> {
    stack: Vec<&'a AnalyzedEvent<'i, 'p>>,
}

impl<'i, 'p, 'a> TopLevelEvents<'i, 'p, 'a> {
    pub fn new<I>(events: impl IntoIterator<Item = &'a AnalyzedEvent<'i, 'p>, IntoIter = I>) -> Self
    where
        I: DoubleEndedIterator<Item = &'a AnalyzedEvent<'i, 'p>>,
    {
        TopLevelEvents {
            stack: events.into_iter().rev().collect(),
        }
    }
}

impl<'i, 'p, 'a> Iterator for TopLevelEvents<'i, 'p, 'a> {
    type Item = &'a AnalyzedEvent<'i, 'p>;

    fn next(&mut self) -> Option<Self::Item> {
        while let Some(event) = self.stack.pop() {
            match event {
                AnalyzedEvent::Conditions(blocks) => {
                    self.stack
                        .extend(blocks.iter().flat_map(|block| &block.events).rev());
                }
                AnalyzedEvent::DeclareArgs(block) => {
                    self.stack.extend(block.events.iter().rev());
                }
                AnalyzedEvent::Import(_)
                | AnalyzedEvent::Assignment(_)
                | AnalyzedEvent::Template(_)
                | AnalyzedEvent::Target(_)
                | AnalyzedEvent::NewScope(_) => {
                    return Some(event);
                }
            }
        }
        None
    }
}

pub enum AnalyzedEvent<'i, 'p> {
    Conditions(Vec<AnalyzedBlock<'i, 'p>>),
    Import(AnalyzedImport<'i>),
    DeclareArgs(AnalyzedBlock<'i, 'p>),
    Assignment(AnalyzedAssignment<'i, 'p>),
    Template(AnalyzedTemplate<'i>),
    Target(AnalyzedTarget<'i>),
    NewScope(AnalyzedBlock<'i, 'p>),
}

pub struct AnalyzedImport<'i> {
    pub file: Pin<Arc<ShallowAnalyzedFile>>,
    pub span: Span<'i>,
}

pub struct AnalyzedScope<'i, 'p> {
    pub parent: Option<Box<AnalyzedScope<'i, 'p>>>,
    pub variables: HashMap<&'i str, AnalyzedVariable<'i, 'p>>,
}

impl<'i, 'p> AnalyzedScope<'i, 'p> {
    pub fn new(parent: Option<Box<AnalyzedScope<'i, 'p>>>) -> Self {
        AnalyzedScope {
            parent,
            variables: HashMap::new(),
        }
    }

    pub fn get(&self, name: &str) -> Option<&AnalyzedVariable<'i, 'p>> {
        self.variables
            .get(name)
            .or_else(|| self.parent.as_ref().and_then(|p| p.get(name)))
    }

    pub fn insert(&mut self, assignment: AnalyzedAssignment<'i, 'p>) {
        self.variables
            .entry(assignment.name)
            .or_insert_with(|| AnalyzedVariable {
                assignments: HashSet::new(),
            })
            .assignments
            .insert(assignment);
    }

    pub fn merge(&mut self, other: &Self) {
        for (name, other_variable) in &other.variables {
            self.variables
                .entry(name)
                .or_insert_with(|| AnalyzedVariable {
                    assignments: HashSet::new(),
                })
                .assignments
                .extend(other_variable.assignments.clone());
        }
    }

    pub fn all_variables(&self) -> HashMap<&'i str, &AnalyzedVariable<'i, 'p>> {
        let mut variables = if let Some(parent) = &self.parent {
            parent.all_variables()
        } else {
            HashMap::new()
        };
        for (name, variable) in &self.variables {
            variables.insert(name, variable);
        }
        variables
    }
}

#[derive(Clone)]
pub struct AnalyzedVariable<'i, 'p> {
    pub assignments: HashSet<AnalyzedAssignment<'i, 'p>>,
}

#[derive(Clone, Eq, Hash, PartialEq)]
pub struct AnalyzedAssignment<'i, 'p> {
    pub name: &'i str,
    pub comments: Comments<'i>,
    pub statement: &'p Statement<'i>,
    pub document: &'i Document,
    pub variable_span: Span<'i>,
}

#[derive(Clone, Eq, Hash, PartialEq)]
pub struct AnalyzedTemplate<'i> {
    pub name: &'i str,
    pub comments: Comments<'i>,
    pub document: &'i Document,
    pub header: Span<'i>,
    pub span: Span<'i>,
}

#[derive(Clone, Eq, Hash, PartialEq)]
pub struct AnalyzedTarget<'i> {
    pub name: &'i str,
    pub document: &'i Document,
    pub header: Span<'i>,
    pub span: Span<'i>,
}

pub enum Link<'i> {
    /// Link to a file. No range is specified.
    File { path: PathBuf, span: Span<'i> },
    /// Link to a target defined in a BUILD.gn file.
    Target {
        path: PathBuf,
        name: &'i str,
        span: Span<'i>,
    },
}

impl<'i> Link<'i> {
    pub fn span(&self) -> Span<'i> {
        match self {
            Link::File { span, .. } => *span,
            Link::Target { span, .. } => *span,
        }
    }
}
