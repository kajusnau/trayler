# trayler — instructions for coding agents

trayler is a small Rust CLI that shows a tray icon (StatusNotifierItem) with a menu of user-given commands and an
Exit entry, optionally following a process given with `--pid`. All code is in `src/main.rs`; the Nix build is in
`package.nix` and `flake.nix`.

## Before you commit

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
nix build                 # flakes only see tracked files: `git add -N` new files first
```

Run them inside `nix develop` if the toolchain is not installed.

## Commits

Follow the "Commit messages" section of `CONTRIBUTING.md`. In short:

- `<type>(<optional scope>): <description>`, type one of `feat`, `fix`, `refactor`, `test`, `chore`, `ci`,
  `revert`, `build`; lowercase scope without spaces.
- The summary is a command, starts lowercase, has no period, and describes the change, not the problem.
- Wrap every line, summary included, at 75 characters.
- The body says what and why, never how, and mentions any change to flags, output or exit codes.
- Sign off every commit (`git commit -s`).
- Keep the message short: no file lists, no restating the diff.

One change per commit: the code builds and the tests pass at every commit, and reverting one does not break its
neighbours. Fold review and typo fixes into the commit they fix, but keep separate changes in separate commits.

Do not commit or push unless asked to.

## Code changes

**Think first.** State your assumptions and ask when unsure. If a request can be read several ways, say so instead
of picking one. If a simpler approach exists, say that too.

**Keep it simple.** Write the least code that solves the problem. No speculative features, no abstractions used
once, no options nobody asked for, no handling of errors that cannot happen.

**Change only what you must.** Every changed line should trace back to the request. Do not tidy nearby code,
comments or formatting. Match the existing style. Remove only what your own change made unused.

**Make it checkable.** Turn the task into something you can verify ("fix the bug" → "write a test that shows the
bug, then make it pass"), and say how each step is checked.

## Code comments

Default to none.

- Explain only *why*: a hidden constraint, a workaround and its cause, something the reader cannot work out. Never
  *what* the code does.
- One line, two at most. If it needs a paragraph, it belongs in the commit message, or the code needs reworking.
- No banners, separators, or notes about the edit itself ("changed X to Y").
- Doc comments only where clap turns them into `--help` text, one line each.

When code exists only because a fix is waiting upstream, mark it with a `TODO` that links the upstream issue or
pull request:

```rust
// TODO: drop once https://github.com/example/crate/pull/123 is released.
```
