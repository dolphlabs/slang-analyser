# Contributing to slanganalyser

Thank you for wanting to help. This file is what you need to build the
language server, run its tests, and get a change merged.

By contributing you agree that your contribution is licensed under the
[MIT licence](LICENSE), like the rest of the project. Please sign your commits
off (`git commit -s`), which adds a `Signed-off-by:` line certifying the
[Developer Certificate of Origin](https://developercertificate.org/). Nothing
checks this automatically yet; we ask that you do it anyway.

Everyone taking part is expected to follow the [Code of
Conduct](CODE_OF_CONDUCT.md). To report a security problem, read
[SECURITY.md](SECURITY.md) and do not open a public issue.

## Build and test

You need a Rust toolchain (`rustup`) and, for the VSCode extension, Node 20.

```sh
cargo build -p slanganalyser   # the language server binary
cargo test --workspace         # the whole suite (28+ tests)
cargo fmt --all --check        # canonical formatting (CI enforces this)
cargo clippy --workspace --all-targets -- -D warnings
```

The extension lives in `editors/vscode/`:

```sh
cd editors/vscode
npm install --no-audit --no-fund
node --check client/extension.js
npx @vscode/vsce package       # builds slang-<version>.vsix
```

To try the server against a slang project before wiring an editor:

```sh
cargo run -p slanganalyser   # speaks LSP over stdio; point your client at it
```

## What a change needs

- **A test.** Unit tests live next to the code (`#[cfg(test)]`) or in
  `crates/*/tests/`. A bug fix needs a test that fails without the fix:
  show it by running the test against the code before your change.
- **No new warnings.** `cargo clippy` with `-D warnings` and `cargo fmt
  --check` must both pass; CI runs them.
- **Fidelity to slangc.** The lexer, parser, project model, and inference
  mirror `src/lexer.c`, `src/parser.c`, `src/project.c`, and
  `src/codegen/infer.c` in [dolphlabs/slang](https://github.com/dolphlabs/slang).
  When in doubt, the compiler's behavior wins over this codebase's
  convenience. Pin or note the slang commit you checked against.
- **Docs.** Update `README.md`, the relevant `editors/*/README.md`, or the
  extension `CHANGELOG.md` when behavior a user can see changes.

## Workflow

- `main` is the release branch. Branch each change from an up-to-date `main`,
  one branch per task, and open the pull request against `main`.
- **Commits:** conventional commits (`feat(hir): …`, `fix(lsp): …`), subject
  ≤ 72 characters, imperative mood, with a body saying what and why. One
  commit per feature; stage files by name, not `git add -A`.
- Write the pull request description for someone who has not seen the
  conversation: what changed and why, the evidence (what failed before, what
  passes now), and anything you did not do.

## Releasing

A pushed version tag is the release; `.github/workflows/release.yml` does
the rest. It builds the server for five targets, attaches the binaries,
checksums, and the `.vsix` to the GitHub release, and publishes the
extension to the VS Code Marketplace and Open VSX when the `VSCE_PAT` and
`OPEN_VSX_TOKEN` secrets exist.

```sh
git tag v0.1.0
git push origin v0.1.0
```

Only tag from a `main` where `ci.yml` is green. If a release is broken,
delete and re-push the tag, or push a new one; the extension version is
taken from the tag.
