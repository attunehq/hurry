#!/bin/bash
# Hook to catch left-hand side type annotations in variable declarations
# This pattern is FORBIDDEN per the style guide - use turbofish or inference instead

set -euo pipefail

# Only check Rust files
if [[ ! "$HOOK_FILE_PATH" =~ \.rs$ ]]; then
    exit 0
fi

# Check for `let name: Type = ...` or `let mut name: Type = ...` patterns
# We need to be careful not to match:
# - Function signatures: `fn foo(x: Type)` or `-> Type`
# - Struct/enum definitions: These are allowed
# - Pattern matching with type ascription in match arms
#
# Strategy: Look for lines that start with whitespace + `let` + optional `mut` + identifier + `:` + type
if echo "$HOOK_NEW_CONTENT" | grep -E '^\s*let\s+(mut\s+)?[a-zA-Z_][a-zA-Z0-9_]*\s*:\s*' | grep -v '^\s*//' > /dev/null; then
    violations=$(echo "$HOOK_NEW_CONTENT" | grep -nE '^\s*let\s+(mut\s+)?[a-zA-Z_][a-zA-Z0-9_]*\s*:\s*' | grep -v '^\s*//')
    line_numbers=$(echo "$violations" | cut -d: -f1 | tr '\n' ',' | sed 's/,$//')

    cat <<EOF
❌ Found left-hand side type annotations (lines: $line_numbers)

Per the style guide, this pattern is FORBIDDEN. Never use \`let foo: Type = ...\` syntax.

Examples from your code:
$(echo "$violations" | head -3)

Why this matters:
  - Type annotations on the left make code harder to scan
  - Turbofish syntax is more idiomatic and flexible in Rust
  - Type inference should be preferred when the type is obvious

What to do:
  - Use turbofish: \`let foo = items.collect::<Vec<_>>()\`
  - Use inference: \`let foo = parse(input)\` (compiler infers type)
  - Use helper methods: \`let foo = items.collect_vec()\` (with itertools)

The ONLY exceptions are function signatures and struct/enum definitions where
type annotations are syntactically required.

If you believe this is a false positive (e.g., inside a function signature),
you can proceed. Otherwise, please refactor to use turbofish or inference.
EOF
    exit 1
fi

exit 0
