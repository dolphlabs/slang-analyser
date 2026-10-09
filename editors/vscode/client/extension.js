// slang VSCode client: spawns `slanganalyser` and bridges LSP.
// Configure the binary with `slang.serverPath` (defaults to PATH lookup).
const path = require("path");
const vscode = require("vscode");
const { LanguageClient, TransportKind } = require("vscode-languageclient/node");

let client = null;

function activate(context) {
    const config = vscode.workspace.getConfiguration("slang");
    const serverPath = config.get("serverPath", "slanganalyser");
    const serverOptions = {
        command: serverPath,
        args: [],
        transport: TransportKind.stdio,
    };
    const clientOptions = {
        documentSelector: [{ scheme: "file", language: "slang" }],
        synchronize: {
            fileEvents: vscode.workspace.createFileSystemWatcher("**/slang.project"),
        },
        traceOutputChannel: vscode.window.createOutputChannel("slang trace"),
    };
    client = new LanguageClient("slang", "slang", serverOptions, clientOptions);
    context.subscriptions.push(client.start());
    context.subscriptions.push(
        vscode.commands.registerCommand("slang.restartServer", async () => {
            if (client) {
                await client.stop();
                await client.start();
            }
        })
    );
}

function deactivate() {
    return client ? client.stop() : undefined;
}

module.exports = { activate, deactivate };
