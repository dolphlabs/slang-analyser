// slang VSCode client: resolves the `slanganalyser` binary, then bridges LSP.
//
// Resolution order:
//   1. `slang.serverPath` when set (pinned build, no network).
//   2. `slanganalyser` on PATH.
//   3. Checksum-verified download from the slang-analyser GitHub releases,
//      cached in extension global storage keyed by extension version.
// A `slang.project` watcher restarts relevance on project changes.
const crypto = require("crypto");
const fs = require("fs");
const https = require("https");
const os = require("os");
const path = require("path");
const vscode = require("vscode");
const { LanguageClient, TransportKind } = require("vscode-languageclient/node");

const REPO = "dolphlabs/slang-analyser";
const EXT_ID = "dolphlabs.slang-vscode";

let client = null;

function targetTriple() {
    const plat = os.platform();
    const arch = os.arch();
    if (plat === "darwin") {
        return arch === "arm64" ? "aarch64-apple-darwin" : "x86_64-apple-darwin";
    }
    if (plat === "linux") {
        return arch === "arm64"
            ? "aarch64-unknown-linux-gnu"
            : "x86_64-unknown-linux-gnu";
    }
    if (plat === "win32") {
        if (arch !== "x64") {
            throw new Error(`slang: unsupported Windows architecture ${arch}`);
        }
        return "x86_64-pc-windows-msvc";
    }
    throw new Error(`slang: unsupported platform ${plat}/${arch}`);
}

function assetName(triple) {
    return triple.endsWith("-msvc")
        ? `slanganalyser-${triple}.exe`
        : `slanganalyser-${triple}`;
}

function fetchBuffer(url, redirects = 5) {
    return new Promise((resolve, reject) => {
        https
            .get(url, (res) => {
                if (res.statusCode >= 300 && res.statusCode < 400 && res.headers.location) {
                    if (redirects === 0) {
                        reject(new Error(`slang: too many redirects fetching ${url}`));
                        return;
                    }
                    resolve(fetchBuffer(res.headers.location, redirects - 1));
                    return;
                }
                if (res.statusCode !== 200) {
                    reject(new Error(`slang: download failed (${res.statusCode}) for ${url}`));
                    return;
                }
                const chunks = [];
                res.on("data", (c) => chunks.push(c));
                res.on("end", () => resolve(Buffer.concat(chunks)));
                res.on("error", reject);
            })
            .on("error", reject);
    });
}

async function pathOnPath(output) {
    const cmd = os.platform() === "win32" ? "where slanganalyser" : "command -v slanganalyser";
    const { execFile } = require("child_process");
    return new Promise((resolve) => {
        execFile(cmd, { shell: true }, (err, stdout) => {
            if (err) {
                resolve(null);
                return;
            }
            const found = stdout.split(/\r?\n/).find((l) => l.trim() !== "");
            output.appendLine(`slang: found server on PATH: ${found}`);
            resolve(found ? found.trim() : null);
        });
    });
}

async function downloadedServer(context, output) {
    const triple = targetTriple();
    const asset = assetName(triple);
    const ext = vscode.extensions.getExtension(EXT_ID);
    const version = (ext && ext.packageJSON.version) || "0.0.0";
    const tag = `v${version}`;
    const dir = context.globalStorageUri.fsPath;
    await vscode.workspace.fs.createDirectory(context.globalStorageUri);
    const dest = path.join(dir, asset);
    const verFile = `${dest}.version`;
    try {
        if (
            fs.existsSync(dest) &&
            fs.existsSync(verFile) &&
            fs.readFileSync(verFile, "utf8").trim() === version
        ) {
            if (os.platform() !== "win32") {
                fs.chmodSync(dest, 0o755);
            }
            output.appendLine(`slang: using cached server ${dest}`);
            return dest;
        }
    } catch (e) {
        output.appendLine(`slang: cache check failed: ${e.message}`);
    }
    const base = `https://github.com/${REPO}/releases/download/${tag}/${asset}`;
    output.appendLine(`slang: downloading ${base}`);
    const [bin, sum] = await Promise.all([
        fetchBuffer(base),
        fetchBuffer(`${base}.sha256`).then((b) => b.toString("utf8").trim()),
    ]);
    const actual = crypto.createHash("sha256").update(bin).digest("hex");
    if (actual !== sum) {
        throw new Error(
            `slang: checksum mismatch for ${asset} (release ${tag}); refusing to run it`
        );
    }
    fs.writeFileSync(dest, bin);
    if (os.platform() !== "win32") {
        fs.chmodSync(dest, 0o755);
    }
    fs.writeFileSync(verFile, `${version}\n`);
    output.appendLine(`slang: installed server ${dest}`);
    return dest;
}

async function resolveServer(context, output) {
    const pinned = vscode.workspace.getConfiguration("slang").get("serverPath", "");
    if (pinned) {
        return pinned;
    }
    const found = await pathOnPath(output);
    if (found) {
        return found;
    }
    return downloadedServer(context, output);
}

async function activate(context) {
    const output = vscode.window.createOutputChannel("slang");
    let command;
    try {
        command = await resolveServer(context, output);
    } catch (e) {
        const open = "Open Settings";
        vscode.window
            .showErrorMessage(
                `slang: cannot start language server (${e.message}). Set slang.serverPath or install slanganalyser manually.`,
                open
            )
            .then((choice) => {
                if (choice === open) {
                    vscode.commands.executeCommand(
                        "workbench.action.openSettings",
                        "slang.serverPath"
                    );
                }
            });
        return;
    }
    const serverOptions = { command, args: [], transport: TransportKind.stdio };
    const clientOptions = {
        documentSelector: [{ scheme: "file", language: "slang" }],
        synchronize: {
            fileEvents: vscode.workspace.createFileSystemWatcher("**/slang.project"),
        },
        traceOutputChannel: vscode.window.createOutputChannel("slang trace"),
        outputChannel: output,
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
