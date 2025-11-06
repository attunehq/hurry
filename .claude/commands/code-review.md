---
description: Run formatters, linters, and perform code review on current branch changes
---

You are performing a self-review of code changes on the current branch.

# Step 1: Understand the Intention

{{> .shared/understand-intention}}

# Step 2: Run Formatters and Linters

{{> .shared/verify-build}}

Address any formatting issues or clippy warnings. If there are clippy warnings, fix them or explain why they should be ignored.

# Step 3: Review for Code Quality Issues

Using the understood intention as context, review the changes for:

## Vestigial Code
- Functions, methods, or types that were added during development but are no longer used
- Dead code that was part of earlier iterations but is now redundant
- Experimental code that didn't make it into the final design
- Check with tools like `cargo build --timings` or manual inspection

## DRY Violations
- Repeated code patterns that could be factored into shared functions
- Similar logic duplicated across multiple files
- Constants or configuration that appear in multiple places
- Look for opportunities to extract common patterns

## Comment Quality
**IMPORTANT**: Only write comments that explain WHY, not WHAT
- Remove comments that just restate what the code does (redundant/noisy comments)
- Remove comments that are obvious from reading the code
- Identify places where a comment explaining WHY would add value (e.g., non-obvious algorithmic choices, performance tradeoffs, workarounds for external constraints)
- Good comment: `// Use atomic rename to prevent partial reads during concurrent access`
- Bad comment: `// Rename the temp file to the target path`
- If you don't know why something is done (because the user hasn't explained), DO NOT add comments: let the user add them when they understand the context

## Type Annotations
**CRITICAL**: Left-hand-side type annotations are FORBIDDEN
- Flag ANY instance of `let foo: Type = ...` patterns
- Always prefer type inference: `let foo = ...`
- Use turbofish syntax when type hints are needed: `let foo = method::<Type>()`
- ❌ NEVER: `let mut hurry_json: serde_json::Value = ...`
- ✅ ALWAYS: `let mut hurry_json = ...` or `let hurry_json = parse::<serde_json::Value>()`

## Standard Code Review
- Does the code match the intention stated in Step 1?
- Are there any logic errors or edge cases not handled?
- Is error handling appropriate and consistent?
- Are there any performance concerns?
- Does the code follow the project's style guidelines (CLAUDE.md, AGENTS.md, codebase style conventions)?
- Are there missing tests for the changes?
- Is documentation needed or outdated?

# Step 4: Report Findings

{{> .shared/report-format}}

Use this structure:

## Code Review Results

### Summary
[Brief overview of what was reviewed]

### ✅ Formatters & Linters
[Results from running formatters/linters]

### 🧹 Vestigial Code
[List any unused functions, methods, or code that should be removed, or note "None found"]

### 🔄 DRY Opportunities
[List repeated patterns that could be factored out, or note "None found"]

### 💬 Comment Quality
[List redundant/noisy comments to remove and places where WHY comments would add value, or note "Comments look good"]

### 🚫 Type Annotations
[List any left-hand-side type annotations (let foo: Type = ...) that must be removed, or note "None found"]

### ⚠️ General Issues
[Other code review findings that need attention]

### Verification Status
- Build: [pass/fail]
- Format: [pass/fail]
- Clippy: [pass/fail]
- Tests: [pass/fail/not run]

### Next Steps
[If everything looks good, say so and suggest next command. If there are issues, offer to fix them or explain what should be changed.]

---

## Workflow Suggestions

After code review:
- If issues found: Fix them, then run `/code-review` again
- If everything looks good: Consider running `/pr-draft` to create a pull request
