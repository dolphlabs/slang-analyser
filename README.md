# slanganalyser

Language server (`LSP`) and formatter for [slang](https://github.com/dolphlabs/slang) (`.sl`), powered by the workspace root marker `slang.project`.

## Features

- Diagnostics, name resolution, and type-aware inference (`slang_hir`)
- Context-aware code completion on a shared lexical scope walker
- Go-to-definition, hover types/signatures, inlay hints, semantic tokens
- Deterministic formatting (`slang_fmt`) and workspace model (`slang_project` + `slang.lock`)

## Crates

- `slang_syntax`: lossless lexer for `.sl` (mirrors `src/lexer.c` / `src/lexer.h`)
- `slang_project`: `slang.project` + `slang.lock` model (mirrors `src/project.c`)
- `slang_hir`: workspace index, lexical scopes, name resolution, type inference, completion
- `slang_fmt`: deterministic formatter
- `slanganalyser`: LSP server binary over stdio

## Install & editors

- **VS Code / Cursor / Windsurf**: install `dolphlabs.slang-analyser` from the VS Code Marketplace or Open VSX (or a `.vsix` from GitHub Releases); the extension downloads a checksum-verified `slanganalyser` binary automatically (`slang.serverPath` overrides).
- **Neovim 0.11+ / other LSP clients**: install from source and configure per [`editors/README.md`](editors/README.md):

```sh
cargo install --path crates/slanganalyser
```

## The golden path: `slang` + `zokor` + `slanganalyser`

`slanganalyser` completes the unified backend workflow alongside
[`slang`](https://github.com/dolphlabs/slang) (`slangc`),
[`zokor`](https://github.com/dolphlabs/zokor), and
[`zokor_cli`](https://github.com/dolphlabs/zokor_cli):

```sh
zokor newcomp billing --db pg   # scaffold domain/service/store/server + slang.project
cd billing && slangc get        # resolve deps cleanly for both compiler and LSP
zokor watch                     # live fast rebuilds (SLANG_DEV=1) while coding in editor
zokor check [--json]            # enforce layer rules, config, and opt-in tenant/auth checks
slangc test .                   # run unit & integration tests
```

## Reference

- Compiler: <https://github.com/dolphlabs/slang>
- Docs: <https://slang.dolphlabs.com>
- CLI & Framework: <https://github.com/dolphlabs/zokor_cli> / <https://github.com/dolphlabs/zokor>
