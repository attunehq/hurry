#!/bin/bash
# Hook to catch unnecessary fully qualified paths
# Prefer direct imports over fully qualified paths unless ambiguous

set -euo pipefail

# Only check Rust files
if [[ ! "$HOOK_FILE_PATH" =~ \.rs$ ]]; then
    exit 0
fi

# Look for common patterns of overly-qualified paths:
# - Multiple :: separators (e.g., foo::bar::baz::Thing)
# - Common crates that should be imported (color_eyre::eyre::, serde_json::, etc.)
#
# Strategy: Find paths with 2+ :: separators (foo::bar::baz)
# Exclude common patterns that should stay qualified:
# - std::io::*, std::fs::* (often intentionally qualified for clarity)
# - Module declarations (mod foo::bar)
# - Use statements themselves

# Check for paths with multiple :: that might be over-qualified
if echo "$HOOK_NEW_CONTENT" | grep -E '[a-zA-Z_][a-zA-Z0-9_]*(::[a-zA-Z_][a-zA-Z0-9_]*){2,}' | \
   grep -v '^\s*use ' | \
   grep -v '^\s*mod ' | \
   grep -v '^\s*//' > /dev/null; then

    # Get some example violations
    violations=$(echo "$HOOK_NEW_CONTENT" | grep -nE '[a-zA-Z_][a-zA-Z0-9_]*(::[a-zA-Z_][a-zA-Z0-9_]*){2,}' | \
                 grep -v '^\s*use ' | \
                 grep -v '^\s*mod ' | \
                 grep -v '^\s*//' | \
                 head -5)

    line_numbers=$(echo "$violations" | cut -d: -f1 | tr '\n' ',' | sed 's/,$//')

    cat <<EOF
⚠️  Found potentially over-qualified paths (lines: $line_numbers)

Per the style guide: "Prefer direct imports over fully qualified paths unless ambiguous"

Examples from your code:
$(echo "$violations")

Common mistakes:
  ❌ color_eyre::eyre::eyre!("...")    → ✅ use eyre; eyre!("...")
  ❌ client::courier::v1::Key::new()   → ✅ use client::courier::v1::Key; Key::new()
  ❌ serde_json::json!({...})          → This one is actually OK (keeps clarity)

When fully qualified paths ARE preferred:
  - When the name is ambiguous or unclear on its own
  - When multiple types with the same name exist
  - When it improves clarity (serde_json::to_string is clearer than to_string)

Review the flagged lines. If you can add a \`use\` statement at the top and
simplify the path, please do so. If the qualified path improves clarity or
avoids ambiguity, proceed as-is.
EOF
    exit 1
fi

exit 0
