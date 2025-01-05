#!/usr/bin/env python3
#
# Copyright 2025 Google LLC
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.


import os
import re
import subprocess


_SECTION_RE = re.compile(r'^## <a name="([^"]+)">[^\n]*\n\n(.*?)(?=^## |\Z)', re.DOTALL | re.MULTILINE)
_ITEM_RE = re.compile(r'^### <a name="[^"]+"></a>(\*+([^*]+)\*+[^\n]*?)&nbsp;[^\n]*\n(.*?)(?=^### |\Z  )', re.DOTALL | re.MULTILINE)
_VERBATIM_RE = re.compile(r'^```$\n(.*?)^```$', re.DOTALL | re.MULTILINE)


def _ensure_gn():
    os.makedirs('build', exist_ok=True)
    if not os.path.exists('build/gn'):
        subprocess.check_call(['git', 'clone', 'https://gn.googlesource.com/gn'], cwd='build')
    subprocess.check_call(['git', 'checkout', '--quiet', 'c97a86a72105f3328a540f5a5ab17d11989ab7dd'], cwd='build/gn')


def _generate_builtins():
    with open('build/gn/docs/reference.md') as f:
        reference = f.read()
    with open('src/builtins.gen.rsi', 'w') as out:
        print('// This file was generated by generate_builtins.py. DO NOT EDIT.', file=out)
        print('', file=out)
        print('BuiltinSymbols {', file=out)
        for m in _SECTION_RE.finditer(reference):
            category = m.group(1)
            if category not in ('targets', 'functions', 'predefined_variables', 'target_variables'):
                continue
            print(f'    {category}: &[', file=out)
            for m in _ITEM_RE.finditer(m.group(2)):
                name = m.group(2)
                doc = m.group(1) + '\n' + m.group(3)
                doc = _VERBATIM_RE.sub(r'```text\n\1```', doc)
                print(f'        BuiltinSymbol {{ name: "{name}", doc: r#"{doc}"# }},', file=out)
            print(f'    ],', file=out)
        print('}', file=out)


def main():
    os.chdir(os.path.dirname(os.path.dirname(__file__)))
    _ensure_gn()
    _generate_builtins()


if __name__ == '__main__':
    main()
