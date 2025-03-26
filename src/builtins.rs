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

pub const IMPORT: &str = "import";
pub const TEMPLATE: &str = "template";
pub const DECLARE_ARGS: &str = "declare_args";
pub const FOREACH: &str = "foreach";
pub const SET_DEFAULTS: &str = "set_defaults";
pub const FORWARD_VARIABLES_FROM: &str = "forward_variables_from";

pub struct BuiltinSymbol {
    pub name: &'static str,
    pub doc: &'static str,
}

pub struct BuiltinSymbols {
    pub targets: &'static [BuiltinSymbol],
    pub functions: &'static [BuiltinSymbol],
    pub predefined_variables: &'static [BuiltinSymbol],
    pub target_variables: &'static [BuiltinSymbol],
}

impl BuiltinSymbols {
    pub fn all(&self) -> impl Iterator<Item = &'static BuiltinSymbol> {
        self.targets
            .iter()
            .chain(self.functions.iter())
            .chain(self.predefined_variables.iter())
            .chain(self.target_variables.iter())
    }
}

pub const BUILTINS: BuiltinSymbols = include!(concat!(env!("OUT_DIR"), "/builtins.gen.rsi"));
