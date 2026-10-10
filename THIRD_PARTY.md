# Third-party software

slanganalyser itself is MIT-licensed (see [LICENSE](LICENSE)). This file says
what it touches that is not, and what that means for you.

**This repository contains no third-party source code.** There are no vendored
libraries. Dependencies are fetched by Cargo (Rust) and npm (the VSCode
extension) at build time, under their own licences.

## Rust dependencies (`Cargo.lock` pins the exact versions)

| Crate | Licence |
|---|---|
| `tower-lsp` | MIT |
| `tokio` | MIT |
| `serde`, `serde_json` | MIT OR Apache-2.0 |

All of their transitive dependencies are MIT and/or Apache-2.0 licensed.
Run `cargo tree` and `cargo metadata --format-version 1` to audit the full
closure for the pinned versions in `Cargo.lock`, which is tracked.

## Extension dependencies (`editors/vscode`)

| Package | Licence |
|---|---|
| `vscode-languageclient` | MIT |

The extension downloads the `slanganalyser` server binary from this
repository's own GitHub releases and verifies its SHA-256 checksum before
running it; the binary is covered by the MIT licence above.

## Things this repository refers to but does not ship

- **The Contributor Covenant**, version 2.1, in
  [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md), is used under its Creative Commons
  Attribution 4.0 licence; the attribution is in that file.
- **The slang language and compiler** ([dolphlabs/slang](https://github.com/dolphlabs/slang),
  MIT) whose lexer, parser, project model, and inference rules this project
  mirrors. Nothing of that repository is copied into this one beyond
  behavior.
