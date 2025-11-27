---
description: Resolve merge conflicts by analyzing PR context and merging base branch
---

You are resolving merge conflicts for the current branch with its base branch.

**CRITICAL**: If you are unsure about how to resolve any conflict, ASK the user rather than making assumptions. It is better to ask clarifying questions than to incorrectly resolve conflicts.

# Step 1: Gather PR Information

{{> .shared/gather-pr-info}}

# Step 2: Analyze Current Branch Context

Examine the PR diff on GitHub to understand the intent of changes on this branch:
- Use `gh pr diff <pr-number>` to view the diff in terminal
- Read the PR description and any comments for additional context
- Identify the key changes and their purpose
- Note the state and intent of the code when it was written

**Important**: Focus on understanding WHY these changes were made, not just WHAT changed.

# Step 3: Identify Conflicting Changes

Before attempting to merge, identify what's causing conflicts:
- Run `git fetch origin` to update remote tracking branches
- Run `git merge-base HEAD origin/<base-branch>` to find the common ancestor
- Use `git log --oneline HEAD..origin/<base-branch>` to see commits in base that aren't in current branch
- Identify which commits likely introduce conflicts (look for changes to same files)

For potentially conflicting PRs:
- Use `gh pr list --base <base-branch> --state merged --search "merged:>$(git log -1 --format=%cd --date=short $(git merge-base HEAD origin/<base-branch>))"` to find recently merged PRs
- For each relevant PR, examine with `gh pr view <pr-number>` or `gh pr diff <pr-number>`
- Understand the state and intent of those changes

# Step 4: Plan Conflict Resolution

Before merging, develop a resolution strategy:
- Identify files that will likely conflict
- For each potential conflict, determine:
  - What this branch was trying to accomplish in that area
  - What the merged PR(s) accomplished in that area
  - How both changes can coexist or which should take precedence

**If you cannot determine the correct resolution strategy, STOP and ask the user:**
- "I see that both your branch and PR #123 modified `file.rs:45-67`. Your branch does X, while PR #123 does Y. How should these be reconciled?"
- "There's a conflict in the approach to Z. Should we preserve your implementation, adopt the new one from the base branch, or merge both?"

# Step 5: Attempt Merge

Once you have a resolution strategy:
1. Ensure working directory is clean (stash if needed)
2. Run `git merge origin/<base-branch>`
3. If conflicts occur, `git status` will show conflicting files
4. Make sure to annotate the merge commit message with context about what you changed and why.

# Step 6: Resolve Conflicts

For each conflicting file:
1. Read the entire file to see conflict markers
2. Understand what each side is trying to accomplish
3. Apply your resolution strategy from Step 4
4. If the correct resolution is unclear for any conflict:
   - **STOP and ask the user** with specific details about the conflict
   - Show relevant code sections from both sides
   - Explain what you understand about each side's intent
   - Ask how they should be combined

When resolving:
- Remove conflict markers (`<<<<<<<`, `=======`, `>>>>>>>`)
- Ensure the resolved code is syntactically correct
- Preserve the intent of both changes where possible
- Follow the codebase's existing patterns

After resolving each file:
- Run `git add <file>` to mark as resolved

# Step 7: Verify Resolution

{{> .shared/verify-build}}

**If any verification step fails and you're not sure how to fix it, ask the user.**

# Step 8: Complete Merge

Once everything is resolved and verified:
1. Run `git status` to confirm all conflicts are resolved
2. Complete the merge with `git commit` (it will use the default merge message)
3. Report results using the format below

# Step 9: Report Results

{{> .shared/report-format}}

Use this structure:

## Conflict Resolution Results

### Summary
[How many files had conflicts, what PRs were merged in]

### ✅ Successfully Resolved
[List of files that were resolved and how]

### ⚠️ Notable Decisions
[Any important resolution choices made, especially where both sides' changes were preserved]

### Verification Status
- Build: [pass/fail]
- Format: [pass/fail]
- Clippy: [pass/fail]
- Tests: [pass/fail/not run]

### Next Steps
[Recommendation on what to do next]

---

## Workflow Suggestions

After resolving conflicts:
- Run `/resolution-verify` with fresh context to double-check the merge preserved the PR's intent
- Run `/code-review` if you made significant changes during resolution
- Review the merge commit: `git show HEAD`
- Push when ready: `git push`

**Remember**: It's always better to ask for clarification than to incorrectly resolve a conflict. The user has the full context of their intent, and you should leverage that rather than guessing.
