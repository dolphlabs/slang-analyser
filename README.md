# slanganalyser

rust-analyzer equivalent for the slang language (`.sl`).

Workspace root is marked by `slang.project` (like `Cargo.toml`).

## Crates

- `slang_syntax`: lossless lexer for `.sl` (mirrors `src/lexer.c` / `src/lexer.h`)
- `slang_project`: `slang.project` + `slang.lock` model (mirrors `src/project.c`)
- `slang_hir` (next): name resolution + type inference
- `slang_fmt` (next): deterministic formatter
- `slanganalyser` (next): LSP server (hover, goto-def, refs, rename, completion, diagnostics, inlay hints, semantic tokens)

## Reference

Compiler: https://github.com/dolphlabs/slang
Docs: https://slang.dolphlabs.com
