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

impl<'i, 'p> TopLevelStatementsExt for &'p Block<'i> {
    type Item = &'p Statement<'i>;
    type IntoIter = TopLevelStatements<'i, 'p>;

    fn top_level_statements(self) -> Self::IntoIter {
        TopLevelStatements::new(&self.statements)
    }
}

pub struct TopLevelStatements<'i, 'p> {
    stack: Vec<&'p Statement<'i>>,
}

impl<'i, 'p> TopLevelStatements<'i, 'p> {
    pub fn new<I>(events: impl IntoIterator<Item = &'p Statement<'i>, IntoIter = I>) -> Self
    where
        I: DoubleEndedIterator<Item = &'p Statement<'i>>,
    {
        TopLevelStatements {
            stack: events.into_iter().rev().collect(),
        }
    }
}

impl<'i, 'p> Iterator for TopLevelStatements<'i, 'p> {
    type Item = &'p Statement<'i>;

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

impl<'i, 'p, 'a> TopLevelStatementsExt for &'a AnalyzedBlock<'i, 'p> {
    type Item = &'a AnalyzedStatement<'i, 'p>;
    type IntoIter = AnalyzedTopLevelStatements<'i, 'p, 'a>;

    fn top_level_statements(self) -> Self::IntoIter {
        AnalyzedTopLevelStatements::new(&self.statements)
    }
}

pub struct AnalyzedTopLevelStatements<'i, 'p, 'a> {
    stack: Vec<&'a AnalyzedStatement<'i, 'p>>,
}

impl<'i, 'p, 'a> AnalyzedTopLevelStatements<'i, 'p, 'a> {
    pub fn new<I>(
        events: impl IntoIterator<Item = &'a AnalyzedStatement<'i, 'p>, IntoIter = I>,
    ) -> Self
    where
        I: DoubleEndedIterator<Item = &'a AnalyzedStatement<'i, 'p>>,
    {
        AnalyzedTopLevelStatements {
            stack: events.into_iter().rev().collect(),
        }
    }
}

impl<'i, 'p, 'a> Iterator for AnalyzedTopLevelStatements<'i, 'p, 'a> {
    type Item = &'a AnalyzedStatement<'i, 'p>;

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
            | AnalyzedStatement::BuiltinCall(_) => {}
        }
        Some(statement)
    }
}
