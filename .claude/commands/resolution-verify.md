---
description: Verify PR intent was preserved after conflict resolution with base branch
---

You are verifying that a PR's original intention was preserved after conflict resolution has been applied from the base branch.

**Context**: The current branch has had conflicts resolved from its base branch (typically `main`). Before pushing these changes, we need to ensure that the original purpose and functionality of the PR wasn't inadvertently changed or broken during conflict resolution.

**Important**: This command is meant to be run with "fresh eyes" after `/conflicts-resolve`. If you just resolved conflicts, recommend the user clear context and run this command fresh, OR proceed with heightened skepticism about your previous resolution decisions.

# Step 1: Gather PR Information

{{> .shared/gather-pr-info}}

# Step 2: Understand Original PR Intent

Examine the PR on GitHub to understand what it was trying to accomplish:
- Use `gh pr view <pr-number>` to see the PR description and metadata
- Use `gh pr diff <pr-number>` to view the original diff as it appears on GitHub
- Read the PR description carefully to understand:
  - What problem was being solved
  - What new functionality was being added
  - What refactoring or changes were being made
  - Any important implementation details or constraints

**Important**: Focus on understanding the PURPOSE and INTENT, not just the mechanical changes. Ask yourself: "What was this PR trying to achieve?"

# Step 3: Identify Conflict Resolution Changes

Determine what changed during conflict resolution:
- Find the merge commit: `git log --oneline --merges -n 1`
- Examine what was merged in: `git log --oneline <merge-commit>^2..origin/<base-branch>`
- Look at the merge commit's changes: `git show <merge-commit>`
- Identify which files had conflicts (they'll appear in the merge commit diff)

# Step 4: Compare Current State to Original Intent

For each file that was touched during conflict resolution:
1. Read the current version of the file
2. Compare it to what the PR originally intended for that file (from the GitHub diff)
3. Check if the core changes from the PR are still present and functional
4. Verify that the conflict resolution didn't:
   - Remove functionality the PR was adding
   - Break the logic the PR was implementing
   - Change the behavior in ways that conflict with the PR's goals
   - Introduce syntax errors or type errors

**Key areas to verify**:
- New functions/methods added by the PR still exist and have correct signatures
- Modified logic still implements the intended behavior
- New imports/dependencies are still present
- Test changes are still appropriate for the modified code
- Documentation changes still make sense with current code

# Step 5: Verify Compilation and Tests

{{> .shared/verify-build}}

**If any of these fail, investigate whether the failure is due to improper conflict resolution.**

# Step 6: Check for Semantic Issues

Beyond compilation, look for semantic problems:
- Logic errors introduced by merging incompatible changes
- Changed assumptions (e.g., function signatures that changed in base branch)
- Missing edge cases that the PR originally handled
- Behavioral changes that don't align with PR intent

**Examples of semantic issues to watch for**:
- The PR added error handling, but it was removed during conflict resolution
- The PR refactored a pattern, but only some instances were updated due to conflicts
- The PR changed an API, but conflict resolution reverted some call sites
- The PR fixed a bug, but the fix was lost in conflict resolution

# Step 7: Report Findings

{{> .shared/report-format}}

Use this structure:

## Resolution Verification Results

### Summary
[Brief summary of what the PR was trying to accomplish and verification outcome]

### Original PR Intent
[What the PR was meant to achieve]

### Files Modified During Conflict Resolution
- `file1.rs`: [brief note on what conflicts occurred]
- `file2.rs`: [brief note on what conflicts occurred]

### ✅ Preserved Functionality
[List of key features/changes that are still intact and working as intended]

### ⚠️ Areas of Concern
[List any potential issues or areas that need attention - could be minor inconsistencies, unclear resolutions, or things that look suspicious]

### ❌ Issues Found
[List specific problems that need fixing - these are clear bugs or incorrect resolutions]

### Verification Status
- Build: [pass/fail]
- Format: [pass/fail]
- Clippy: [pass/fail]
- Tests: [pass/fail/not run]

### Next Steps
[Clear recommendation: ready to push, needs fixes, or needs manual review]

---

## Workflow Suggestions

**If ready to push**:
- Run `/code-review` if you haven't already reviewed code quality
- Push the changes: `git push`
- Monitor CI for any issues

**If issues found**:
- Fix the issues (offer to help)
- Run `/resolution-verify` again after fixes
- Consider running `/code-review` if significant changes were made

**Remember**: The goal is to catch issues BEFORE pushing, not after CI fails or reviewers find problems. Be thorough but efficient in your analysis.
