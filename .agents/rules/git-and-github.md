# Git and GitHub Workflow Guidelines

## Git command conventions

Always run git commands from the current working directory. Never use `git -C <path>` or `cd <path> && git ...` to target a different directory — both patterns trigger permission prompts and signal a workflow problem.

The right directory is always implied by context:

- When working in the main repository, it is the current directory.
- When working in a worktree, the worktree root is the current directory.

If you find yourself wanting to run a git command against a path that is not the current directory, treat that as a red flag: you are likely reaching across worktree boundaries. Stay in your working directory instead.

## Branch naming

All branches must follow a descriptive, prefix-based naming convention. Use lowercase letters and hyphens for word separation.

| Prefix | When to use |
|--------|-------------|
| `feature/` | New functionality |
| `fix/` | Bug fixes |
| `docs/`, `refactor/`, `perf/`, `chore/`, `test/`, … | When the above don't fit |

## Branch setup before committing

Before creating a commit, verify you are on the correct branch for the work at hand.

**Adding to an existing PR branch** — if the branch already exists and matches the work, just commit.

**Starting a new PR** — if the current branch belongs to unrelated work:

1. Inspect uncommitted changes (`git diff`, `git status`) and determine which files belong to the current branch and which to the new PR.
2. Stash selectively as needed — use `git stash push -- <paths>` to move only specific files into the stash, keeping unrelated changes on the current branch.
3. Switch to `main` and pull: `git switch main && git pull`.
4. Create the new branch: `git switch -c <prefix>/<description>`.
5. Restore the stashed changes: `git stash pop`.
6. Commit.

Never commit work intended for a new PR onto an existing feature branch or directly onto `main`.

## Commits

### Messages

Limit the subject line to 50 characters.

Use the body to explain what changed and why, not how it was changed. Wrap the body at 72 characters. When a body is long enough to need wrapping, use a single `-m` argument with embedded newlines rather than multiple body `-m` arguments.

### Attribution

AI contributions should include an `Assisted-by` trailer in the following format: `Assisted-by: AGENT_NAME:MODEL_VERSION`

Where:

- `AGENT_NAME` is the name of the AI tool or framework
- `MODEL_VERSION` is the specific model version used

Use the `--trailer` command-line option to specify the `Assisted-by` trailer when creating a commit.

### Example

```sh
git commit \
  -m "Document retry policy" \
  -m "Explain when background jobs should retry transient failures.
Keep permanent errors visible so operators can diagnose bad input
without reading task logs." \
  --trailer "Assisted-by: Claude:claude-3-opus"
```

## Pull requests

When opening a pull request, the description must serve as a concise technical summary for human reviewers.

Content Focus: Detail what was changed and why. Focus on architectural shifts, logic updates, or dependency changes.

No CI/CD info: Do not mention build statuses, test passes, or linting results.

No Redundant Attribution: Since the commits already contain the `Assisted-by` trailer, do not mention your AI identity in the PR description.

No TODOs: Unless the PR is explicitly marked as a [Draft], the description should not contain "to-do" items or "work in progress" notes.

Structure: Use bullet points for readability and clear headings.

## Pull request reviews

When the task is to review an opened pull request on GitHub, there is no need to run CI checks locally. Rely on GitHub CI to run those checks.
