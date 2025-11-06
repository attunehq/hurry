# Claude Code Slash Commands

This directory contains custom slash commands for working with Claude Code in this repository. These commands provide structured workflows for common development tasks like code review, conflict resolution, and PR management.

## Quick Reference

| Command | Purpose | When to Use |
|---------|---------|-------------|
| `/code-review` | Run formatters, linters, and comprehensive code review | Before committing changes or creating a PR |
| `/changes-commit` | Create a detailed commit with conversation context | When ready to commit changes |
| `/conflicts-resolve` | Resolve merge conflicts intelligently | When merging base branch into your feature branch |
| `/resolution-verify` | Verify PR intent after conflict resolution | After resolving conflicts (with fresh context) |
| `/pr-draft` | Create a draft PR with detailed context | When ready to open a pull request |
| `/code-trace` | Trace and map code architecture for a feature | When exploring or documenting code flows |
| `/diff-trace` | Trace code differences between branches | When reviewing large changes or understanding a PR |
| `/pr-comment-trace` | Generate and post a technical analysis comment | When you want detailed technical analysis on a PR |

> [!TIP]
> The commands are generally organized in `noun-verb` format.

## Detailed Command Documentation

### `/code-review`

**Purpose**: Comprehensive code quality review including formatters, linters, and best practices checks.

**What it does**:
- Runs `make format` and `make check` (which includes clippy)
- Reviews code for common issues:
  - Vestigial/unused code
  - DRY violations (code duplication)
  - Comment quality (removes "what" comments, suggests "why" comments)
  - Type annotations (flags forbidden left-hand-side type annotations like `let foo: Type = ...`)
  - General code quality issues
- Provides structured feedback with actionable next steps

**When to use**:
- Before committing changes
- Before creating a PR
- After making significant code changes
- After resolving conflicts to ensure quality

**Example usage**:
```
/code-review
```

**Typical workflow**:
1. Make your code changes
2. Run `/code-review` to check quality
3. Fix any issues found
4. Run `/code-review` again to verify
5. Proceed to commit or PR creation

---

### `/changes-commit`

**Purpose**: Create a git commit with rich context about the conversation and decisions made.

**What it does**:
- Creates a structured commit message including:
  - Type and brief summary
  - Context about what problem was being solved
  - Key technical decisions and why they were made
  - Effective prompts/instructions that led to good outcomes
- Adds Claude Code attribution

**When to use**:
- After completing a feature or fix
- When you want to preserve conversation context in git history
- To document how to effectively collaborate with Claude

**Example usage**:
```
/changes-commit
```

**Commit message format**:
```
<type>: <brief summary>

## Context
What problem we were solving or what feature we were building

## Key Decisions
- Specific instructions or constraints provided
- Alternative approaches considered and why we chose this one
- Patterns, style preferences, or architectural decisions
- High-value context about why things are done a certain way

## Prompts & Instructions
Direct quotes or paraphrases of key prompts that led to specific outcomes

🤖 Generated with [Claude Code](https://claude.com/claude-code)

Co-Authored-By: Claude <noreply@anthropic.com>
```

**Typical workflow**:
1. Complete your changes
2. Run `/code-review` if you haven't already
3. Run `/changes-commit` to create a detailed commit
4. Push your changes

---

### `/conflicts-resolve`

**Purpose**: Intelligently resolve merge conflicts by understanding both branches' intent.

**What it does**:
- Gathers PR information and context
- Analyzes the current branch's intent
- Identifies what changed in the base branch
- Develops a resolution strategy
- Resolves conflicts while preserving both sides' intent
- Verifies the build still works
- Creates a merge commit with context

**When to use**:
- When you need to merge the base branch (e.g., `main`) into your feature branch
- When you have merge conflicts after pulling latest changes
- Before pushing a long-running branch

**Critical behavior**:
- **Will ask questions** if resolution strategy is unclear
- Better to ask than to make incorrect assumptions
- Focuses on preserving intent of both sides

**Example usage**:
```
/conflicts-resolve
```

**Typical workflow** (conflict resolution):
1. Run `/conflicts-resolve` to merge base branch and resolve conflicts
2. Review the changes and verify they look correct
3. Run `/changes-commit` if additional changes were needed during resolution
4. **Open a NEW Claude window** (fresh context)
5. Run `/resolution-verify` to double-check everything is correct
6. Fix any issues found
7. Push when satisfied

---

### `/resolution-verify`

**Purpose**: Verify that a PR's original intent was preserved after conflict resolution.

**What it does**:
- Examines the PR description and original diff
- Identifies what changed during conflict resolution
- Compares current state to original intent
- Checks for:
  - Missing functionality
  - Broken logic
  - Changed behavior
  - Build/test failures
- Provides detailed report of findings

**When to use**:
- **After running `/conflicts-resolve`** (with fresh context!)
- Before pushing a branch that had conflicts
- When you want to ensure merge didn't break anything
- After any significant merge from base branch

**Critical behavior**:
- **Meant to be run with "fresh eyes"** - use a new Claude window after resolving conflicts
- Skeptical review of previous resolution decisions
- Catches issues before CI or reviewers find them

**Example usage**:
```
/resolution-verify
```

**Typical workflow** (see `/conflicts-resolve` workflow above)

---

### `/pr-draft`

**Purpose**: Create a comprehensive draft pull request with detailed context.

**What it does**:
- Gathers branch and commit information
- Analyzes changes and conversation history
- **Asks clarifying questions** about unclear aspects
- Creates structured PR with:
  - Summary of changes
  - Areas of interest for reviewers
  - Testing instructions (if applicable)
  - Code map with GitHub permalinks
- Posts as draft PR on GitHub

**When to use**:
- When you're ready to open a pull request
- After code review and testing are complete
- When you want detailed PR documentation

**Critical behavior**:
- **Will ask questions** to ensure accuracy
- Prefers asking over guessing or speculation
- Only includes information it's confident about

**Example usage**:
```
/pr-draft
```

**Typical workflow**:
1. Complete your feature/fix
2. Run `/code-review` to ensure quality
3. Commit your changes (optionally with `/changes-commit`)
4. Run `/pr-draft` to create the PR
5. Review the PR on GitHub
6. Mark as ready for review when satisfied

---

### `/code-trace`

**Purpose**: Trace and document code architecture for a specific feature or flow.

**What it does**:
- Creates a structured code map showing:
  - Entry points (CLI commands, API endpoints)
  - Execution flow through the codebase
  - Key components and data structures
  - Architecture rationale
- Uses ASCII diagrams for visualization
- Outputs to terminal for easy viewing

**When to use**:
- When exploring unfamiliar parts of the codebase
- When documenting a feature for others
- When onboarding new team members
- When planning changes to existing flows

**Critical behavior**:
- **Only references code that actually exists**
- Verifies all functions/files before mentioning them
- Will say "I couldn't find X" rather than hallucinating

**Example usage**:
```
/code-trace how does cargo command passthrough work?
/code-trace cache restore flow
/code-trace daemon communication
```

**Output format**:
```
=== CODEMAP: [topic] ===

[ENTRY POINT(S)]
[EXECUTION FLOW]
[KEY DATA STRUCTURES]
[HELPER FUNCTIONS]
[CROSS-CUTTING CONCERNS]
[ARCHITECTURE NOTES]
```

---

### `/diff-trace`

**Purpose**: Create a detailed comparison of changes between base branch and current branch.

**What it does**:
- Identifies all changed files
- Analyzes what changed and why
- Creates structured diff trace with:
  - Overview of changes
  - Key architectural changes
  - Execution flow changes
  - ASCII flow diagrams (before/after)
  - Data structure changes
  - Breaking changes (public API only)
  - Cross-cutting concerns
- Outputs to terminal for review

**When to use**:
- When reviewing large changesets
- Before code review to understand impact
- When preparing to review someone else's PR
- To understand what a branch accomplishes

**Critical behavior**:
- Only reports **public API** breaking changes (CLI commands, API endpoints)
- Does **not** report internal refactoring as breaking changes
- Verifies all mentioned code exists

**Example usage**:
```
/diff-trace
```

**Typical workflow**:
1. Checkout a branch you want to understand
2. Run `/diff-trace` to see comprehensive analysis
3. Use the trace to guide your code review
4. Reference specific sections when asking questions

---

### `/pr-comment-trace`

**Purpose**: Generate a technical analysis comment and post it to a PR on GitHub.

**What it does**:
- Similar to `/diff-trace` but formatted for GitHub
- Creates focused markdown with:
  - Implementation details
  - Mermaid flow diagrams (if flow changed)
  - Breaking changes (if any)
  - Impact assessment
  - Expandable technical details
- Uses GitHub permalinks for all file references
- Posts directly as a PR comment

**When to use**:
- When you want to provide detailed technical analysis on a PR
- After reviewing a PR and want to document findings
- To help reviewers understand complex changes
- To provide implementation context on your own PR

**Critical behavior**:
- Only includes relevant sections (omits empty sections)
- Uses GitHub permalinks for clickable file references
- Only includes flow diagrams if flow actually restructured
- Only includes breaking changes section if public API changed

**Example usage**:
```
/pr-comment-trace
```

**Typical workflow**:
1. Checkout the PR branch locally
2. Run `/pr-comment-trace`
3. Comment is automatically posted to GitHub
4. Reviewers can use the technical analysis alongside code review

## Common Workflows

### Workflow 1: Regular Feature Development

```
1. Write your code
2. /code-review          # Check quality
3. Fix any issues
4. /changes-commit       # Commit with context
5. /pr-draft            # Create PR
6. Mark PR as ready for review
```

### Workflow 2: Handling Merge Conflicts

```
1. /conflicts-resolve    # Merge base branch and resolve conflicts
2. Review changes
3. /changes-commit       # Commit any additional changes (if needed)
4. Open NEW Claude window (fresh context)
5. /resolution-verify    # Verify intent preserved
6. Fix any issues found
7. Push when satisfied
```

### Workflow 3: Large Branch Sync

```
1. /conflicts-resolve    # Merge latest base branch
2. /changes-commit       # Commit the merge
3. Open NEW Claude window
4. /resolution-verify    # Verify everything works
5. /code-review         # Check for any quality issues
6. Push when satisfied
```

### Workflow 4: Understanding Existing Code

```
1. /code-trace [feature]  # Understand how something works
2. Read the mapped files
3. Ask follow-up questions
4. /diff-trace           # See what changed recently (if on a branch)
```

### Workflow 5: Comprehensive PR Review

```
1. Checkout PR branch
2. /diff-trace           # Understand full scope of changes
3. Review specific files based on trace
4. /pr-comment-trace     # Post detailed analysis
5. Provide review feedback on GitHub
```

## Command Structure

Each command is defined in a markdown file with:

- **Frontmatter**: Metadata including command description
  ```yaml
  ---
  description: Brief description shown in command list
  ---
  ```

- **Body**: Detailed instructions for Claude on how to execute the command
  - Step-by-step process
  - Critical behaviors and guidelines
  - Output format specifications
  - Verification requirements

- **Shared Partials**: Common instruction blocks in `.shared/` that are reused across commands
  - `gather-pr-info.md`: Get PR number and metadata
  - `understand-intention.md`: Understand what changes were trying to accomplish
  - `verify-build.md`: Run formatters, linters, and tests
  - `report-format.md`: Standard reporting format

## Customization

To modify command behavior:

1. Edit the corresponding `.md` file in `.claude/commands/`
2. Modify shared partials in `.claude/commands/.shared/` if needed
3. Commit and push changes
4. Updated commands are immediately available locally
5. Remote GitHub Actions workflows will use updated commands after merge

## Usage in GitHub Actions

These commands are automatically available when Claude Code runs in GitHub Actions (`.github/workflows/claude.yml`). Tag Claude in a PR or issue comment:

```
@claude /code-review
@claude /pr-comment-trace
```

### Commands Suitable for GitHub App Use

**Fully compatible** (read-only or report-only):
- `/code-review` - Reviews code and reports findings (doesn't auto-commit fixes)
- `/code-trace` - Generates code architecture maps
- `/diff-trace` - Analyzes branch differences
- `/pr-comment-trace` - Posts technical analysis to PR
- `/resolution-verify` - Verifies conflict resolution (read-only analysis)

**Use with caution in GitHub app** (creates commits/PRs):
- `/changes-commit` - Creates git commits (requires write permissions)
- `/conflicts-resolve` - Merges and commits conflict resolution (requires write permissions)
- `/pr-draft` - Creates draft PRs (requires PR creation permissions)

**Note**: The GitHub workflows have been configured with `contents: write` and `pull-requests: write` permissions to support all commands. However, commands that create commits or PRs should be used intentionally when invoked via `@claude`.

## Project-Specific Context

These commands are designed specifically for the Hurry/Courier codebase and:
- Follow project coding conventions defined in `CLAUDE.md` and `AGENTS.md`
- Integrate with project tooling: cargo, make, gh CLI, git
- Understand the monorepo structure (packages/hurry, packages/courier, etc.)
- Follow Rust-specific best practices

## Tips for Effective Use

1. **Ask questions during commands**: Many commands will ask clarifying questions - answer them for better results

2. **Use fresh context for verification**: After resolving conflicts, open a new Claude window before running `/resolution-verify`

3. **Chain commands logically**: Follow the recommended workflows for best results

4. **Trust but verify**: Commands do their best to be accurate, but always review the output

5. **Iterate**: If a command finds issues, fix them and run again

6. **Read the command files**: Each `.md` file contains detailed instructions - read them to understand what Claude will do

## Troubleshooting

**Command not found**:
- Ensure you're in the repository directory
- Check that `.claude/commands/` exists and contains the command file

**Command behavior unexpected**:
- Read the command's `.md` file to understand its instructions
- Check if shared partials (`.shared/`) were modified
- Verify you're providing necessary context in your prompts

**Accuracy issues**:
- Commands are designed to verify code existence before referencing
- If you notice inaccuracies, report them so commands can be improved
- Always review command output before acting on it

## Contributing

When adding new commands:

1. Create a new `.md` file in `.claude/commands/`
2. Add frontmatter with description
3. Write clear, step-by-step instructions
4. Include verification requirements
5. Specify output format
6. Consider extracting common patterns to `.shared/`
7. Test locally before committing
8. Update this README with command documentation
