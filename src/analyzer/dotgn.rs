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

use std::path::{Path, PathBuf};

use crate::{
    analyzer::utils::resolve_path,
    common::{
        error::{Error, Result},
        utils::LineIndex,
    },
    parser::{parse, AssignOp, LValue, Statement},
};

pub fn evaluate_dot_gn(workspace_root: &Path, input: &str) -> Result<PathBuf> {
    let line_index = LineIndex::new(input);
    let parsed_root = parse(input);

    let mut build_config_path: Option<PathBuf> = None;

    for statement in &parsed_root.statements {
        let Statement::Assignment(assignment) = statement else {
            continue;
        };
        if !matches!(&assignment.lvalue, LValue::Identifier(identifier) if identifier.name == "buildconfig")
        {
            continue;
        }

        let position = line_index.position(assignment.span.start());

        if assignment.op != AssignOp::Assign {
            return Err(Error::General(format!(
                "{}:{}:{}: buildconfig must be assigned exactly once",
                workspace_root.join(".gn").to_string_lossy(),
                position.line + 1,
                position.character + 1
            )));
        }
        let Some(name) = assignment.rvalue.as_simple_string() else {
            return Err(Error::General(format!(
                "{}:{}:{}: buildconfig is not a simple string",
                workspace_root.join(".gn").to_string_lossy(),
                position.line + 1,
                position.character + 1
            )));
        };

        if build_config_path
            .replace(resolve_path(name, workspace_root, workspace_root))
            .is_some()
        {
            return Err(Error::General(format!(
                "{}:{}:{}: buildconfig is assigned multiple times",
                workspace_root.join(".gn").to_string_lossy(),
                position.line + 1,
                position.character + 1
            )));
        }
    }

    let Some(build_config_path) = build_config_path else {
        return Err(Error::General(format!(
            "{}: buildconfig is not assigned directly",
            workspace_root.join(".gn").to_string_lossy()
        )));
    };

    Ok(build_config_path)
}
