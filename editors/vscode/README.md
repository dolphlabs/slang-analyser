# slang for VS Code

Language support for [slang](https://slang.dolphlabs.com) (`.sl`), powered by the
`slanganalyser` language server.

## Features

- Hover shows inferred types plus documentation
- Go to definition (functions, structs, methods, enum variants)
- Diagnostics as you type
- Format document (`slang_fmt` canonical style)
- Inlay hints for unannotated `let` bindings
- Semantic highlighting (works with any theme) plus bundled **slang Dark** and **slang Light** themes

## Requirements

On activation the extension uses `slanganalyser` from your `PATH` when present.
Otherwise it downloads the matching binary from the
[slang-analyser releases](https://github.com/dolphlabs/slang-analyser/releases)
into extension storage (checksum-verified) and uses that. Set
`slang.serverPath` to pin a specific binary and skip downloads entirely:

```json
{
    "slang.serverPath": "/usr/local/bin/slanganalyser"
}
```

A `slang.project` file marks the workspace root, the same file `slangc` uses.

## Settings

| Setting | Default | Description |
| --- | --- | --- |
| `slang.serverPath` | `""` (auto) | Path to the `slanganalyser` binary. Empty means PATH lookup, then auto-download. |
| `slang.trace.server` | `"off"` | LSP traffic tracing for debugging. |

## Commands

- `slang: Restart Server` (`slang.restartServer`)

## Building from source

```sh
cargo build --release -p slanganalyser
# binary at target/release/slanganalyser
```
