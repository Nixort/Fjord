# Contributing to Fjord

Thanks for your interest in Fjord. This is a security-critical OS; the bar for
changes — especially inside the TCB (`anchor`, `keel`, crypto) — is high.

## Ground rules

- **Discuss first.** Open an issue describing the design before large PRs.
- **Small, reviewable commits.** Follow Conventional Commits
  (`feat:`, `fix:`, `docs:`, `refactor:`, `test:`, `chore:`, `build:`).
- **No `unsafe` without justification.** Every `unsafe` block needs a
  `// SAFETY:` comment proving the invariants it relies on.
- **TCB changes need two reviewers** and, where applicable, updated proofs.

## Workflow

```sh
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo shipwright -- build
cargo shipwright -- test      # runs no_std tests under QEMU
```

All of the above must pass before review. CI enforces them.

## Patch series & history

Fjord values a *readable history*. Prefer a clean series of focused commits
over one large drop. Generate a shareable series with:

```sh
git format-patch origin/main      # one .patch file per commit
```

The initial skeleton itself was built as a patch series; see `patches/`.

## Code style

- Document every public item (`missing_docs` is denied).
- Mark unfinished work with `TODO(owner): ...` or `FIXME(owner): ...`.
- Keep modules small and capability-scoped; no ambient globals.

## Licensing of contributions

By contributing you agree your work is licensed under GPL-3.0-or-later.
