# slanganalyser

rust-analyzer equivalent for the slang language (`.sl`).

Workspace root is marked by `slang.project` (like `Cargo.toml`).

## Crates

- `slang_syntax`: lossless lexer for `.sl` (mirrors `src/lexer.c` / `src/lexer.h`)
- `slang_project`: `slang.project` + `slang.lock` model (mirrors `src/project.c`)
- `slang_hir`: workspace index, lexical scopes, name resolution, type inference, completion
- `slang_fmt`: deterministic formatter
- `slanganalyser`: LSP server (hover, goto-def, completion, diagnostics, formatting, inlay hints, semantic tokens; references and rename next)

## Reference

Compiler: https://github.com/dolphlabs/slang
Docs: https://slang.dolphlabs.com
