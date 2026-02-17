/**
 * Copyright 2025 Google LLC
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

import * as path from 'path';
import * as vscode from 'vscode';
import * as p2c from 'vscode-languageclient/lib/common/protocolConverter';
import {
  LanguageClient,
  LanguageClientOptions,
  Location,
  MessageSignature,
  Position,
  ResponseError,
  ServerOptions,
  TransportKind,
  WorkspaceEdit,
} from 'vscode-languageclient/node';

const EXECUTABLE_SUFFIX: string = process.platform === 'win32' ? '.exe' : '';

interface InitializationOptions {
  vscode_extension: boolean;
}

function reportAsyncError(
  output: vscode.OutputChannel,
  result: Promise<void>
): void {
  void result.catch(err => {
    if (err instanceof Error) {
      output.appendLine(err.stack ?? err.message);
      void vscode.window.showErrorMessage(`Error: ${err.message}`);
    } else {
      output.appendLine(String(err));
      void vscode.window.showErrorMessage(`Error: ${err}`);
    }
  });
}

function ancestors(uri: vscode.Uri): vscode.Uri[] {
  const ancestors = [];
  let current = uri;
  for (;;) {
    ancestors.push(current);
    const dir = path.posix.dirname(current.path);
    if (dir === current.path) {
      break;
    }
    current = current.with({path: dir});
  }
  return ancestors;
}

async function statNoThrow(
  uri: vscode.Uri
): Promise<vscode.FileStat | undefined> {
  try {
    return await vscode.workspace.fs.stat(uri);
  } catch {
    return undefined;
  }
}

async function isInGnWorkspace(uri: vscode.Uri): Promise<boolean> {
  for (const dirUri of ancestors(uri).slice(1)) {
    for (const name of ['.gn', 'BUILD.gn']) {
      const candidateUri = dirUri.with({
        path: path.posix.join(dirUri.path, name),
      });
      if (await statNoThrow(candidateUri)) {
        return true;
      }
    }
  }
  return false;
}

async function updateActiveEditorContext(): Promise<void> {
  const uri = vscode.window.activeTextEditor?.document?.uri;
  const inGnWorkspace = uri ? await isInGnWorkspace(uri) : false;
  vscode.commands.executeCommand(
    'setContext',
    'gn.inGnWorkspace',
    inGnWorkspace
  );
}

async function openBuildFile(): Promise<void> {
  const startUri = vscode.window.activeTextEditor?.document?.uri;
  if (!startUri) {
    void vscode.window.showErrorMessage('No open editor.');
    return;
  }

  const isGnFile =
    startUri.path.endsWith('.gn') || startUri.path.endsWith('.gni');

  if (isGnFile) {
    const dotGnUri = startUri.with({
      path: path.posix.join(path.posix.dirname(startUri.path), '.gn'),
    });
    if (await statNoThrow(dotGnUri)) {
      void vscode.window.showInformationMessage(
        'This file is in the top-level directory.'
      );
      return;
    }
  }

  for (const dirUri of ancestors(startUri).slice(isGnFile ? 2 : 1)) {
    const buildUri = dirUri.with({
      path: path.posix.join(dirUri.path, 'BUILD.gn'),
    });
    if (await statNoThrow(buildUri)) {
      vscode.window.showTextDocument(buildUri);
      return;
    }
    if (
      await statNoThrow(
        dirUri.with({path: path.posix.join(dirUri.path, '.gn')})
      )
    ) {
      break;
    }
  }

  void vscode.window.showErrorMessage(
    'BUILD.gn not found in the ancestor directories.'
  );
}

async function showTargetReferences(
  position: Position,
  locations: Location[],
  converter: p2c.Converter
): Promise<void> {
  const documentUri = vscode.window.activeTextEditor?.document?.uri;
  if (!documentUri) {
    void vscode.window.showErrorMessage('No open editor.');
    return;
  }

  await vscode.commands.executeCommand(
    'editor.action.showReferences',
    documentUri,
    converter.asPosition(position),
    locations.map(converter.asLocation)
  );
}

async function copyTargetLabel(label: string): Promise<void> {
  await vscode.env.clipboard.writeText(label);
  void vscode.window.showInformationMessage(`Copied: ${label}`);
}

interface ChooseImportCandidatesData {
  candidates: ImportCandidate[];
}

interface ImportCandidate {
  import: string;
  edit: WorkspaceEdit;
}

async function chooseImportCandidates(
  data: ChooseImportCandidatesData,
  converter: p2c.Converter
): Promise<void> {
  const items = data.candidates.map(candidate => ({
    label: `Import \`${candidate.import}\``,
    edit: candidate.edit,
  }));
  const selectedItem = await vscode.window.showQuickPick(items);
  if (selectedItem && vscode.window.activeTextEditor) {
    const workspaceEdit = await converter.asWorkspaceEdit(selectedItem.edit);
    await vscode.workspace.applyEdit(workspaceEdit);
  }
}

class GnLanguageClient extends LanguageClient {
  constructor(context: vscode.ExtensionContext, output: vscode.OutputChannel) {
    const clientOptions: LanguageClientOptions = {
      initializationOptions: {
        vscode_extension: true,
      } as InitializationOptions,
      documentSelector: [
        {scheme: 'file', pattern: '**/*.gn'},
        {scheme: 'file', pattern: '**/*.gni'},
      ],
      synchronize: {
        configurationSection: 'gn',
        fileEvents: [
          vscode.workspace.createFileSystemWatcher('**/*.gn'),
          vscode.workspace.createFileSystemWatcher('**/*.gni'),
        ],
      },
      outputChannel: output,
    };

    const extensionDir = context.extensionPath;
    const serverOptions: ServerOptions = {
      transport: TransportKind.stdio,
      command: path.join(
        extensionDir,
        'dist/gn-language-server' + EXECUTABLE_SUFFIX
      ),
      options: {
        cwd: extensionDir,
        env: {
          RUST_BACKTRACE: '1',
        },
      },
    };

    super('gn', 'GN', serverOptions, clientOptions);
  }

  handleFailedRequest<T>(
    type: MessageSignature,
    token: vscode.CancellationToken | undefined,
    error: unknown,
    defaultValue: T,
    showNotification?: boolean
  ): T {
    if (error instanceof ResponseError && error.code === 1) {
      this.error(`${type.method}: ${error.message}`, true);
      throw error;
    }
    return super.handleFailedRequest(
      type,
      token,
      error,
      defaultValue,
      showNotification
    );
  }
}

async function startLanguageServer(
  context: vscode.ExtensionContext,
  output: vscode.OutputChannel
): Promise<void> {
  const client = new GnLanguageClient(context, output);
  context.subscriptions.push(client);
  await client.start();

  context.subscriptions.push(
    vscode.commands.registerCommand(
      'gn.showTargetReferences',
      (position, locations) =>
        showTargetReferences(position, locations, client.protocol2CodeConverter)
    ),
    vscode.commands.registerCommand('gn.chooseImportCandidates', data =>
      chooseImportCandidates(data, client.protocol2CodeConverter)
    )
  );
}

export function activate(context: vscode.ExtensionContext): void {
  const output = vscode.window.createOutputChannel('GN');
  context.subscriptions.push(output);

  context.subscriptions.push(
    vscode.window.onDidChangeActiveTextEditor(() => {
      reportAsyncError(output, updateActiveEditorContext());
    })
  );
  reportAsyncError(output, updateActiveEditorContext());

  context.subscriptions.push(
    vscode.commands.registerCommand('gn.openBuildFile', openBuildFile),
    vscode.commands.registerCommand('gn.copyTargetLabel', copyTargetLabel)
  );

  reportAsyncError(output, startLanguageServer(context, output));
}
