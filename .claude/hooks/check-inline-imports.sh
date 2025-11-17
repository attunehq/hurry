#!/bin/bash
# Hook to catch `use` statements inside function bodies
# This is a common mistake made by AI assistants that violates Rust conventions
# and the project's style guide.

set -euo pipefail

# Only check Rust files
if [[ ! "$HOOK_FILE_PATH" =~ \.rs$ ]]; then
    exit 0
fi

# Check for indented `use` statements in the new content
# Pattern: lines starting with whitespace followed by `use `
if echo "$HOOK_NEW_CONTENT" | grep -q '^[[:space:]]\+use '; then
    line_numbers=$(echo "$HOOK_NEW_CONTENT" | grep -n '^[[:space:]]\+use ' | cut -d: -f1 | tr '\n' ',' | sed 's/,$//')

    cat <<EOF
❌ Found indented 'use' statements in new content (lines: $line_numbers)

This usually means you've placed import statements inside a function body.
Per the project style guide (CLAUDE.md):

  "Never put import statements inside functions (unless the function is
   feature/cfg gated): always put them at file level"

Why this matters:
  - Rust convention: imports belong at the top of the file or module
  - Readability: dependencies should be visible at a glance
  - Consistency: all code in this project follows this pattern

What to do:
  - Move the 'use' statement to the top of the file with other imports
  - If this is legitimately needed (e.g., inside a #[cfg(test)] function),
    you can proceed anyway and I trust your judgment

Take a moment to review the code you're about to write. If this is a mistake,
fix it now. If it's intentional and correct, proceed with confidence.
EOF
    exit 1
fi

exit 0
