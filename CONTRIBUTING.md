# Contributing

Ideas and fixes are welcome. Open an issue to discuss a change, or a pull request from a fork to propose one.

## Workflow

- Branch off `main` and rebase on it instead of merging it in. Force-push your branch after a rebase.
- Every change must pass the checks in [AGENTS.md](AGENTS.md#before-you-commit).

## Commits

Keep each commit to one change that stands on its own:

1. The code builds and the tests pass at that commit.
2. Reverting it does not break the commits around it.
3. It does one thing. If you need the word "and" to describe it, it is probably two commits.

Fold fixes for review comments and typos into the commit they fix: use `git commit --fixup <sha>` while you work
and `git rebase --autosquash` before asking for review. Keep refactoring in its own commit, apart from behaviour
changes.

## Commit messages

Use [Conventional Commits](https://www.conventionalcommits.org/): a summary line, an optional body after a blank
line, and a sign-off (`git commit -s`).

**Summary**

- `<type>(<optional scope>): <description>`, with a type from the table below and a lowercase scope without spaces.
- Describe the change, not the problem behind it; the reason goes in the body.
- Write it as a command, starting lowercase, with no period at the end: "fix: keep the icon after a panel
  restart", not "fix: Kept the icon after a panel restart."
- Keep the line within 75 characters.

**Body**

- Say what changed and why, in plain sentences. Leave out how; the diff shows that.
- Mention any change to the command-line flags, output or exit codes.
- Wrap lines at 75 characters and keep it short. Leave the body out only when the summary says it all.

| Type       | Use for                                  |
| ---------- | ---------------------------------------- |
| `feat`     | New feature                              |
| `fix`      | Bug fix                                  |
| `refactor` | Code change that is not a fix or feature |
| `test`     | Adding or updating tests                 |
| `chore`    | Maintenance                              |
| `ci`       | CI changes                               |
| `revert`   | Reverting a previous commit              |
| `build`    | Build, packaging or dependencies         |

Example:

```
fix(menu): keep underscores in option labels

Labels such as "save_as_mp4" lost their underscores, because the
menu protocol reads a single underscore as an access-key marker.
Escape them before handing labels to the tray. No flags change.

Signed-off-by: Your Name <you@example.com>
```

Pull request titles use the same format as the summary line.
