# Git and GitHub Workflow Guidelines

## Git command conventions

Avoid using `git -C <path>` to run git commands from a different directory. Instead, rely on the shell's working directory or `cd` into the target directory first. Some AI-agent permission matchers only recognize commands run from the repository working directory.

## Branch naming convention

All branches must follow a descriptive, prefix-based naming convention. Use lowercase letters and hyphens for word separation.

Standard prefixes:

Features: feature/<short-description>

Bug Fixes: fix/<short-description>

Flexibility Note: You are encouraged to select alternative prefixes (e.g., docs/, refactor/, perf/, chore/, or test/) if the standard "feature" or "fix" categories do not accurately represent the scope of the work.

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
