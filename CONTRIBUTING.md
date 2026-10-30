# Contributing

btop-web is a web port of btop, so changes that touch collection are held to btop's own collection semantics.

## Before you start

- Open an issue for a behaviour change or a new dependency, so the scope is agreed before the code exists
- Read `INTENT.md` for the non-negotiables: collection parity, one collector module per btop collector, dual themes and no invented data

## Rules

- Explain why in the issue, the commit and the pull request. The diff shows what changed
- Keep unrelated changes in separate commits and pull requests
- Declare AI-generated code in the pull request and say which parts it produced
- Run `make ci` before you push; CI runs the same format, clippy and test commands and fails on any warning
- Add a test for behaviour you change. Collector tests read recorded trees under `crates/btop-web/tests/fixtures` through `BTOP_WEB_PROC_ROOT`; do not write a test that depends on the host kernel
- Do not replace a btop computation with a generic metric crate. Each collector module names the btop source file and function it mirrors, and must read the same source with the same formula, units and rounding

## Style

Rust is formatted by `cargo fmt` with `rustfmt.toml`, and clippy runs with warnings denied. Use Conventional Commits for commit messages.

## Dependencies

Prefixed crates come from crates.io and Renovate groups their updates. htmx and its SSE extension are vendored files, so Renovate does not see them: upgrade both together, keep the pinned version in the README accurate and run the tests.

## Licence

Contributions are accepted under the MIT licence in `LICENSE`; the third-party files listed in `NOTICE.md` keep their own licences.

## Reports

Security vulnerabilities go through the process in `SECURITY.md`, never a public issue. Conduct concerns go through `CODE_OF_CONDUCT.md`.
