# Changelog

## Unreleased

- Code completion: locals and parameters in scope (with inferred types),
  struct fields and methods after `.` (including `self.` in methods),
  enum variants after `Type.`, exported members after `pkg.` (native
  packages included), package items, builtins, and keywords. Nothing is
  offered inside strings or comments, or where a new name is declared.
- Hover and goto-definition now see `let`s in nested blocks (`if`, `for`,
  `while`, `select`, `switch` bodies) and method parameters such as
  `self`, and show doc comments on `impl` methods.
- Fix: a self-referential `let` (`let x = x.next;`) no longer crashes the
  language server with a stack overflow.
- Imports resolve like the compiler: local dirs, native packages (158
  signatures with hover + return-type inference), stdlib sources, and
  `slang.project` pins from the package cache — so `zokor.*`, `pg.*`,
  `json.*` and friends now hover, complete chains, and infer.
- `slang.project` gains `exclude` dirs and `slang.lock` hashes, mirroring
  the compiler's manifest handling.
- Parser: generic/dotted struct literals (`ScheduledJob[S] { ... }`,
  `httpc.Request { ... }`), pointer dereference (`*p`), and `peer`/`wire`/
  `until`/`fault`/`trip` lex as identifiers exactly like upstream.
- Faster rebuilds on every keystroke (docs are cached per file).

## 0.1.7

- Hover now infers call return types, struct field walks (`c.config.database`),
  `guard`/`if let` bindings with `opt`/`result` unwrapping, `err_of` bindings,
  package constants, and multi-level chains; goto-definition follows all of them.
- Parser: `pub gc struct`, `extern fn`, `map[K]V`, open slices, assigning
  `if`/`while` bodies, `link`/`wire`/`until` types (verified zero errors on
  real-world packages).
- Workspace loading is lenient: one unparsable file no longer blinds hover
  for the whole project.

## 0.1.0

- Initial release: hover with inferred types and docs, goto-definition,
  diagnostics, formatting, inlay hints, semantic tokens.
- TextMate grammar for `.sl`, language configuration, slang Dark/Light themes.
- Automatic `slanganalyser` binary download from GitHub releases with
  checksum verification and `slang.serverPath` override.
