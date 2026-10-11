# Changelog

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
