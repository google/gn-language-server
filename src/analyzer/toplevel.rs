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

use crate::{
    analyzer::{AnalyzedBlock, AnalyzedStatement},
    common::builtins::{DECLARE_ARGS, FOREACH},
    parser::{Block, Statement},
};

pub trait TopLevelStatementsExt {
    type Item;
    type IntoIter: IntoIterator<Item = Self::Item>;

    fn top_level_statements(self) -> Self::IntoIter;
}

impl<'p> TopLevelStatementsExt for &'p Block<'p> {
    type Item = &'p Statement<'p>;
    type IntoIter = TopLevelStatements<'p>;

    fn top_level_statements(self) -> Self::IntoIter {
        TopLevelStatements::new(&self.statements)
    }
}

pub struct TopLevelStatements<'p> {
    stack: Vec<&'p Statement<'p>>,
}

impl<'p> TopLevelStatements<'p> {
    pub fn new<I>(events: impl IntoIterator<Item = &'p Statement<'p>, IntoIter = I>) -> Self
    where
        I: DoubleEndedIterator<Item = &'p Statement<'p>>,
    {
        TopLevelStatements {
            stack: events.into_iter().rev().collect(),
        }
    }
}

impl<'p> Iterator for TopLevelStatements<'p> {
    type Item = &'p Statement<'p>;

    fn next(&mut self) -> Option<Self::Item> {
        let statement = self.stack.pop()?;
        match statement {
            Statement::Condition(condition) => {
                let mut blocks = Vec::new();
                let mut current_condition = condition;
                loop {
                    blocks.push(&current_condition.then_block);
                    match &current_condition.else_block {
                        Some(Either::Left(next_condition)) => {
                            current_condition = next_condition;
                        }
                        Some(Either::Right(last_block)) => {
                            blocks.push(last_block);
                            break;
                        }
                        None => break,
                    }
                }
                self.stack
                    .extend(blocks.into_iter().flat_map(|block| &block.statements).rev());
            }
            Statement::Call(call) if [DECLARE_ARGS, FOREACH].contains(&call.function.name) => {
                if let Some(block) = &call.block {
                    self.stack.extend(block.statements.iter().rev());
                }
            }
            _ => {}
        }
        Some(statement)
    }
}

impl<'p, 'a> TopLevelStatementsExt for &'a AnalyzedBlock<'p> {
    type Item = &'a AnalyzedStatement<'p>;
    type IntoIter = AnalyzedTopLevelStatements<'p, 'a>;

    fn top_level_statements(self) -> Self::IntoIter {
        AnalyzedTopLevelStatements::new(&self.statements)
    }
}

pub struct AnalyzedTopLevelStatements<'p, 'a> {
    stack: Vec<&'a AnalyzedStatement<'p>>,
}

impl<'p, 'a> AnalyzedTopLevelStatements<'p, 'a> {
    pub fn new<I>(events: impl IntoIterator<Item = &'a AnalyzedStatement<'p>, IntoIter = I>) -> Self
    where
        I: DoubleEndedIterator<Item = &'a AnalyzedStatement<'p>>,
    {
        AnalyzedTopLevelStatements {
            stack: events.into_iter().rev().collect(),
        }
    }
}

impl<'p, 'a> Iterator for AnalyzedTopLevelStatements<'p, 'a> {
    type Item = &'a AnalyzedStatement<'p>;

    fn next(&mut self) -> Option<Self::Item> {
        let statement = self.stack.pop()?;
        match statement {
            AnalyzedStatement::Conditions(condition) => {
                let mut blocks = Vec::new();
                let mut current_condition = condition;
                loop {
                    blocks.push(&current_condition.then_block);
                    match &current_condition.else_block {
                        Some(Either::Left(next_condition)) => {
                            current_condition = next_condition;
                        }
                        Some(Either::Right(last_block)) => {
                            blocks.push(last_block);
                            break;
                        }
                        None => break,
                    }
                }
                self.stack
                    .extend(blocks.into_iter().flat_map(|block| &block.statements).rev());
            }
            AnalyzedStatement::DeclareArgs(declare_args) => {
                self.stack
                    .extend(declare_args.body_block.statements.iter().rev());
            }
            AnalyzedStatement::Foreach(foreach) => {
                self.stack
                    .extend(foreach.body_block.statements.iter().rev());
            }
            AnalyzedStatement::Assignment(_)
            | AnalyzedStatement::Import(_)
            | AnalyzedStatement::ForwardVariablesFrom(_)
            | AnalyzedStatement::Template(_)
            | AnalyzedStatement::Target(_)
            | AnalyzedStatement::BuiltinCall(_)
            | AnalyzedStatement::Error(_) => {}
        }
        Some(statement)
    }
}
