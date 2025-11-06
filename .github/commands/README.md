# Claude Code Slash Commands

This directory contains custom slash commands that can be used when interacting with Claude via GitHub Actions.

## Available Commands

### Development Workflow

- **/code-review** - Run formatters, linters, and perform comprehensive code review on current branch changes
- **/changes-commit** - Create a git commit with detailed context about the conversation
- **/pr-draft** - Create a draft pull request with context from conversation and branch changes

### Merge & Conflict Resolution

- **/conflicts-resolve** - Resolve merge conflicts by analyzing PR context and merging base branch
- **/resolution-verify** - Verify PR intent was preserved after conflict resolution with base branch

### Code Analysis

- **/code-trace** - Trace and map code architecture for a specific feature or flow (terminal output)
- **/diff-trace** - Trace code differences between base and current branch for code review (terminal output)
- **/pr-comment-trace** - Generate streamlined PR comment with diff trace (auto-posts to GitHub)

## Usage

### In GitHub Comments

When Claude is mentioned in a PR or issue comment, you can invoke slash commands:

```
@claude /code-review
```

```
@claude /pr-draft
```

```
@claude run /diff-trace and then /pr-comment-trace
```

### In Automated Workflows

The `claude-code-review.yml` workflow automatically runs `/code-review` on new PRs.

## Command Structure

Each command is defined in a markdown file with:
- **Frontmatter**: Metadata including command description
- **Body**: Detailed instructions for Claude on how to execute the command
- **Shared Partials**: Common instruction blocks in `.shared/` that are reused across commands

## Customization

To modify command behavior:
1. Edit the corresponding `.md` file
2. Commit and push changes
3. The updated commands will be available in the next workflow run

## Notes

- These commands are designed specifically for the Hurry/Courier codebase
- They follow the project's coding conventions defined in `CLAUDE.md` and `AGENTS.md`
- Commands integrate with the project's tooling: cargo, make, gh CLI, etc.
