# slang in Zed — status

A Zed registry extension is planned but **not shipped yet**: Zed languages
need a tree-sitter grammar for parsing/highlighting, and `.sl` does not have
one. The TextMate grammar in `editors/vscode/syntaxes/` is the starting point
for that port.

What works today: `slanganalyser` itself is editor-agnostic LSP over stdio,
so the moment the grammar lands, the extension is a thin manifest
(`extension.toml` + `languages/slang/config.toml`) pointing at the same
release binaries this repo already publishes per tag.

If you want it sooner, the pieces to build are:

1. `tree-sitter-slang` grammar covering the syntax in `crates/slang_syntax`
   (the lexer and parser there mirror `src/lexer.c` and are the spec).
2. Zed extension with `languages/slang/` (config + `highlights.scm`) and an
   LSP section downloading `slanganalyser-<target>` from this repo's releases,
   same URL scheme as `editors/vscode/client/extension.js`.
3. Submit to the [Zed extension registry](https://github.com/zed-industries/extensions).
