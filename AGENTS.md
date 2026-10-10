# AGENTS.md — slanganalyser

Read this whole file before changing anything. The global studio rules in
`/Users/utee/AGENTS.md` also apply; this file adds what is specific to this
repository. Where the two disagree, the global rules win.

## 1. What this is, and the bar

slanganalyser is the rust-analyzer equivalent for the slang language
(`.sl`): a Rust workspace (`slang_syntax`, `slang_project`, `slang_hir`,
`slang_fmt`, `slanganalyser`) plus editor clients in `editors/`. A
`slang.project` file marks the workspace root, like `Cargo.toml`.

The bar for every change:

- **Production quality.** No TODOs, stubs, or placeholder logic.
- **Fidelity to slangc first.** The lexer, parser, project model, and
  inference mirror `src/lexer.c`, `src/parser.c`, `src/project.c`, and
  `src/codegen/infer.c` in `dolphlabs/slang`. The compiler's behavior wins
  over this codebase's convenience. Note the slang commit you checked against.
- **The server never dies on input.** A malicious `.sl` file or crafted LSP
  request gets diagnostics, never a panic or a hang. See `SECURITY.md`.
- **Never execute what you did not verify.** The extension runs a downloaded
  binary only after its SHA-256 matches the release asset. Do not weaken that.

## 2. Repository map

| Path | What it is |
|---|---|
| `crates/slang_syntax/` | lossless lexer, error-recovery parser, AST |
| `crates/slang_project/` | `slang.project` model (mirrors `src/project.c`) |
| `crates/slang_hir/` | workspace index, name resolution, type inference |
| `crates/slang_fmt/` | canonical formatter, idempotent, keeps comments |
| `crates/slanganalyser/` | LSP server over stdio (`backend.rs`, `convert.rs`) |
| `editors/vscode/` | extension client, TextMate grammar, slang Dark/Light themes |
| `editors/neovim/`, `editors/zed/` | wiring docs and registry status |
| `.github/workflows/` | `ci.yml` (fmt, clippy, tests, vsix smoke), `release.yml` (tag → binaries + publish) |

## 3. Workflow (every task)

- Branch from an up-to-date `main`, one branch per task, PR against `main`.
- **Commits:** conventional commits (`feat(hir): …`), subject ≤ 72 chars,
  imperative, body saying what and why. One commit per feature; stage files
  by name, never `git add -A`.
- **End every task with a `## Git` section** (commit messages, PR title, PR
  description), per the global rules.
- Verify with `cargo test --workspace`, `cargo fmt --all --check`, and
  `cargo clippy --workspace --all-targets -- -D warnings` before committing.

## 4. Definition of done

- [ ] A test that failed on the old code and passes on the new one.
- [ ] Full suite green with the count reported, fmt and clippy clean.
- [ ] Fidelity changes cite the slang behavior or commit they mirror.
- [ ] User-visible changes update `README.md`, the relevant
      `editors/*/README.md`, or the extension `CHANGELOG.md`.
- [ ] Extension/client changes re-checked with `node --check` (and
      `vsce package` for manifest changes).

## 5. Hard constraints

- Parser changes must keep error recovery: broken files still produce an AST
  plus errors, because the IDE parses on every keystroke.
- Formatter output must stay idempotent: `format(format(x)) == format(x)`.
  Comments are never dropped.
- The extension never runs an unverified binary: keep the checksum check and
  the `slang.serverPath` escape hatch; report, don't bypass.
- Release asset names are a contract with `client/extension.js`
  (`slanganalyser-<target>[.exe]` + bare-hash `.sha256`). Renaming one side
  means changing the other in the same task.
