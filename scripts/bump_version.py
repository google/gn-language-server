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

import argparse
import json
import os
import subprocess
import sys


def main():
    os.chdir(os.path.dirname(os.path.dirname(__file__)))

    parser = argparse.ArgumentParser()
    parser.add_argument(
        '--update',
        action='store_true',
        help='Update manifests with the new version')
    options = parser.parse_args()

    with open('vscode-gn/package.json') as f:
        version = json.load(f)['version']
    components = [int(s) for s in version.split('.')]
    assert len(components) == 3, version
    if components[1] % 2 == 0:
        components[1] += 1
        components[2] = 0
    else:
        components[2] += 1
    new_version = '.'.join(str(c) for c in components)
    print(new_version)

    if options.update:
        subprocess.check_call(['cargo', 'set-version', new_version])
        subprocess.check_call(['npm', 'version', new_version], cwd='vscode-gn')
    else:
        print(
            'INFO: Not updating manifests as --update was not set',
            file=sys.stderr)


if __name__ == '__main__':
    main()
