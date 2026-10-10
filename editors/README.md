# Editor support

| Editor | Install | Notes |
| --- | --- | --- |
| VS Code / Cursor / Windsurf | Marketplace (`dolphlabs.slang-vscode`) or Open VSX, or attached `.vsix` on GitHub releases | Server binary auto-downloads (checksum-verified); `slang.serverPath` overrides |
| Neovim 0.11+ | Manual, see below | `slanganalyser` on `PATH` |
| Zed | Planned | Blocked on a tree-sitter grammar; see `editors/zed/README.md` |

All editors talk to the same `slanganalyser` binary over LSP (stdio).
A `slang.project` file marks the workspace root.
