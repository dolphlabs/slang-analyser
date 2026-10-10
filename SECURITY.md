# Security policy

## Supported versions

slanganalyser is pre-1.0. Only the latest release and `main` get security
fixes.

## Reporting a vulnerability

**Please do not open a public issue for a security problem.** Report it
privately through GitHub:

<https://github.com/dolphlabs/slang-analyser/security/advisories/new>

(or the repository's **Security** tab → **Report a vulnerability**). Only the
maintainers can see it.

If that form is not available, open a public issue titled
`security contact request` with **no details** in it, and a maintainer will
arrange a private way to talk.

A useful report has:

- the slanganalyser version (release tag or commit), the OS and architecture;
- the smallest `.sl` file, project layout, or LSP request sequence that shows it;
- what you expected, what happened, and what you think an attacker gains.

We will acknowledge the report, keep you informed while we work on it, and
credit you in the fix unless you would rather we did not. Please give us a
reasonable chance to fix a problem before you publish it.

## What counts as a vulnerability

In scope:

- **Release-asset integrity**: a checksum or signature check that can be
  bypassed, so the VSCode extension (or a user) runs a binary that did not
  come from this repository's release pipeline.
- **Extension download abuse**: the auto-download following redirects or
  hosts outside the release flow in a way an attacker on the network can
  exploit (note: downloads go to `github.com` over TLS; plain-HTTP or
  host-swapping behavior is a bug).
- **Memory safety in the server**: this is Rust, so the bar is a panic that
  kills the server on untrusted input (a malicious `.sl` file or a crafted
  LSP request) or, worse, any unsafe-code memory unsafety. A crash on a
  pathological file is still worth reporting: the server must never die on
  input, only report diagnostics.
- **Workspace escape**: the server reading or writing files outside the
  `slang.project` workspace it was opened on.

Not in scope:

- Soundness of the slang language itself (type rules, the collector, the
  runtime): report those to [dolphlabs/slang](https://github.com/dolphlabs/slang).
- Bugs in a user's own `.sl` program.
- Problems in Rust, Tokio, tower-lsp, or Node themselves; report those
  upstream. This repository vendors none of them (see
  [THIRD_PARTY.md](THIRD_PARTY.md)).
