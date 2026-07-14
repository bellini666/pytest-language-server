import * as path from 'path';
import * as vscode from 'vscode';
import * as fs from 'fs';
import {
  LanguageClient,
  LanguageClientOptions,
  ServerOptions,
} from 'vscode-languageclient/node';

let client: LanguageClient | undefined;

/** Detect musl-based Linux (no glibc runtime in the process report). */
function isMusl(): boolean {
  try {
    const report = process.report?.getReport() as { header?: { glibcVersionRuntime?: string } };
    return report?.header?.glibcVersionRuntime === undefined;
  } catch {
    return false;
  }
}

export async function activate(context: vscode.ExtensionContext) {
  const config = vscode.workspace.getConfiguration('pytestLanguageServer');
  const customExecutable = config.get<string>('executable', '');

  let command: string;

  if (customExecutable) {
    // User specified a custom executable
    command = customExecutable;
  } else {
    // Use bundled binary
    const platform = process.platform;
    const arch = process.arch;

    let binaryName: string;
    if (platform === 'win32') {
      // Windows arm64 can run the x64 binary through emulation.
      binaryName = 'pytest-language-server.exe';
    } else if (platform === 'darwin') {
      // macOS
      if (arch === 'arm64') {
        binaryName = 'pytest-language-server-aarch64-apple-darwin';
      } else {
        binaryName = 'pytest-language-server-x86_64-apple-darwin';
      }
    } else if (platform === 'linux') {
      if (arch !== 'arm64' && arch !== 'x64') {
        vscode.window.showErrorMessage(
          `Unsupported architecture: linux/${arch}. Please install pytest-language-server manually ` +
          `(e.g. "pip install pytest-language-server") and set "pytestLanguageServer.executable".`
        );
        return;
      }
      if (isMusl()) {
        vscode.window.showErrorMessage(
          'The bundled pytest-language-server binary is built against glibc and will not run on ' +
          'musl-based systems (e.g. Alpine). Please install pytest-language-server manually ' +
          '(e.g. "pip install pytest-language-server") and set "pytestLanguageServer.executable".'
        );
        return;
      }
      if (arch === 'arm64') {
        binaryName = 'pytest-language-server-aarch64-unknown-linux-gnu';
      } else {
        binaryName = 'pytest-language-server-x86_64-unknown-linux-gnu';
      }
    } else {
      vscode.window.showErrorMessage(
        `Unsupported platform: ${platform}. Please install pytest-language-server manually and configure the executable path.`
      );
      return;
    }

    command = context.asAbsolutePath(path.join('bin', binaryName));
  }

  // Check if the binary exists and is executable
  try {
    if (!fs.existsSync(command)) {
      vscode.window.showErrorMessage(
        `pytest-language-server binary not found at: ${command}. Please install pytest-language-server or configure the executable path.`
      );
      return;
    }
    // Make sure it's executable on Unix-like systems
    if (process.platform !== 'win32') {
      try {
        fs.accessSync(command, fs.constants.X_OK);
      } catch {
        fs.chmodSync(command, 0o755);
      }
    }
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    vscode.window.showErrorMessage(
      `Failed to access pytest-language-server binary: ${message}. ` +
      `Ensure the file exists and you have permission to execute it.`
    );
    console.error('Binary access error:', error);
    return;
  }

  const serverOptions: ServerOptions = {
    command,
    args: [],
  };

  const clientOptions: LanguageClientOptions = {
    documentSelector: [{ scheme: 'file', language: 'python' }],
    synchronize: {
      fileEvents: vscode.workspace.createFileSystemWatcher('**/*.py'),
    },
  };

  client = new LanguageClient(
    'pytestLanguageServer',
    'pytest Language Server',
    serverOptions,
    clientOptions
  );

  // Register restart command
  context.subscriptions.push(
    vscode.commands.registerCommand('pytest-language-server.restart', async () => {
      if (client) {
        try {
          await client.restart();
          vscode.window.showInformationMessage('pytest Language Server restarted successfully');
        } catch (error) {
          const message = error instanceof Error ? error.message : String(error);
          vscode.window.showErrorMessage(
            `Failed to restart pytest Language Server: ${message}`
          );
        }
      }
    })
  );

  // Register the command emitted by the server's code lenses ("N usages"):
  // resolve references at the lens position and show them in the peek view.
  context.subscriptions.push(
    vscode.commands.registerCommand(
      'pytest-lsp.findReferences',
      async (uriString: string, line: number, character: number) => {
        const uri = vscode.Uri.parse(uriString);
        const position = new vscode.Position(line, character);
        const locations = await vscode.commands.executeCommand<vscode.Location[]>(
          'vscode.executeReferenceProvider',
          uri,
          position
        );
        await vscode.commands.executeCommand(
          'editor.action.showReferences',
          uri,
          position,
          locations ?? []
        );
      }
    )
  );

  // Start the language server with proper error handling
  try {
    await client.start();
    console.log('pytest-language-server started successfully');
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    vscode.window.showErrorMessage(
      `Failed to start pytest-language-server: ${message}. Please check the extension output for details.`
    );
    console.error('pytest-language-server activation error:', error);
    throw error; // Re-throw to prevent partial activation
  }
}

export function deactivate(): Thenable<void> | undefined {
  if (!client) {
    return undefined;
  }
  return client.stop();
}
