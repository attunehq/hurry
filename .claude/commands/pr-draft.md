---
description: Create a draft pull request with context from conversation and branch changes
---

You are creating a draft pull request for the current branch.

**CRITICAL**: If you are unsure about any aspect of the changes, ASK the user rather than making assumptions. It is better to ask multiple clarifying questions than to include inaccurate or speculative information in the PR description.

# Step 1: Understand the Intention

{{> .shared/understand-intention}}

# Step 2: Gather Branch Information

{{> .shared/gather-pr-info}}

Additionally collect:
- Git diff summary and commit history
- Key files changed and their purpose
- Repository info for GitHub permalinks: `gh repo view --json owner,name` and `git rev-parse HEAD`

# Step 3: Draft PR Content

Using the understood intention and branch information, analyze what you can confidently include in the PR.

**Before drafting, identify any gaps where you need clarification:**
- Are there trade-offs or design decisions that aren't documented in comments or conversation history?
- Are there specific edge cases or considerations that aren't obvious from the code?
- Are there performance implications you're uncertain about?
- Are there breaking changes or migration steps that need explanation?
- Is there context about why certain approaches were chosen over alternatives?
- Are there specific areas of the code that need special reviewer attention, but you're not sure why?

**ASK the user about ANY gaps before proceeding.** Frame questions specifically:
- "I see you chose approach X in `file.rs:123`. What trade-offs or considerations led to that choice?"
- "Are there specific edge cases in the new functionality that reviewers should be aware of?"
- "Should reviewers pay special attention to any particular areas? If so, why?"

Once you have clarity, create a draft PR that includes:

## Title
- Concise, descriptive title following conventional commit style
- Detect commit type from branch name pattern:
  - Branch patterns like `*/TYPE/*` or `*-TYPE-*` where TYPE is one of: fix, feat, chore, docs, refactor, test, perf. Use the corresponding conventional commit prefix (e.g., `fix:`, `feat:`, etc.).
- If the branch name doesn't contain a recognizable commit type pattern, **ask the user** what conventional commit categorization to use (feat, fix, chore, docs, refactor, test, perf, etc.)
- Example: `feat: Add cargo command passthrough support`

## Body Structure

### Summary
- 2-4 sentences explaining the intention and what this PR accomplishes
- Focus on the "why" and high-level "what"
- **Only include information you're confident about or that the user has confirmed**

### Areas of Interest
**Only include sections where you have concrete information:**
- Specific code sections, patterns, or decisions that reviewers should pay attention to
- Non-obvious implementation choices (with context from user if needed)
- Trade-offs made (only if documented or explained by user)
- Performance considerations (only if verified)
- Breaking changes or deprecations (only if confirmed)

**If you're unsure whether something should be included here, ask the user.**

### Testing
**Only include if applicable and you have clear information** (skip if not relevant or unclear):
- Manual testing steps for reviewers to verify functionality
- Specific scenarios to test
- Expected behavior
- Any setup required

**If testing steps exist but you're not sure what they should be, ask the user to describe them.**

### Code Map
- Brief overview of major changes organized by area/package
- Use GitHub permalinks for all file references so they're clickable in the PR
- Don't list every single change: focus on key structural changes
- Example:
  - **CLI**: [`hurry/cmd/cargo.rs:45`](link) - Added passthrough argument parsing
  - **Tests**: [`hurry/tests/it/passthrough.rs`](link) - Comprehensive test coverage for argument variations
  - **Docs**: [`AGENTS.md:67`](link) - Updated guidelines for passthrough pattern

**GitHub Permalink Format:**
```
https://github.com/OWNER/REPO/blob/COMMIT_SHA/path/to/file.rs#L123
```

How to construct:
1. Get commit SHA: `git rev-parse HEAD`
2. Get repo info: `gh repo view --json owner,name`
3. Format: `https://github.com/{owner}/{name}/blob/{sha}/{path}#L{line}`

**Only describe changes you understand. If you're uncertain about the purpose or significance of a change, ask before including it.**

# Step 4: Create Draft PR

Use the `gh pr create` command with:
- `--draft` flag to mark as draft
- `--title` with the crafted title
- `--body` with the structured content using a HEREDOC for proper formatting
- Target the appropriate base branch

Example format:
```bash
gh pr create --draft --title "feat: description" --body "$(cat <<'EOF'
## Summary
[summary content]

## Areas of Interest
[areas content]

## Testing
[testing steps if applicable]

## Code Map
[code map content]

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

# Step 5: Report Success

{{> .shared/report-format}}

Use this structure:

## PR Creation Results

### Summary
[Brief description of what was included in the PR]

### PR Details
- URL: [PR URL]
- Title: [PR title]
- Base branch: [base branch]
- Status: Draft

### Next Steps
- Review the PR description on GitHub to ensure accuracy
- Make any manual adjustments if needed
- Mark as ready for review when satisfied

---

## Workflow Suggestions

After creating the PR:
- Review the PR on GitHub to ensure all information is accurate
- If you merged from base recently, consider running `/resolution-verify` to ensure merge correctness
- Mark the PR as ready for review when you're confident in the changes

**Remember**: Accuracy is more important than completeness. A shorter PR description with only confirmed information is better than a longer one with speculative or incorrect details. When in doubt, ask!
