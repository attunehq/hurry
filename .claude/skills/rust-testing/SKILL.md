---
name: rust-testing
description: Rust testing patterns and best practices. Use this skill when writing, reviewing, or modifying Rust tests. Covers test organization, assertions with pretty_assertions, parameterized tests, and testing multiple input formats.
---

# Rust Testing Guide

Project-specific testing patterns for consistent, readable tests.

## Test Organization

- Colocate tests with code in `#[cfg(test)]` modules (not separate `tests/` directories)
- Write tests integration-style (test public APIs) not unit-style (test internals)

## Assertions with pretty_assertions

Import with prefixes to avoid shadowing:

```rust
use pretty_assertions::{
    assert_eq as pretty_assert_eq,
    assert_ne as pretty_assert_ne,
    assert_matches as pretty_assert_matches,
};
```

### Key Pattern: Construct Full Expected Value First

Always construct the ENTIRE expected value upfront and compare in ONE operation:

```rust
// ✅ Prefer: Declare expected value first, single assertion
let expected = serde_json::json!({
    "written": [key1, key2, key3],
    "skipped": [],
    "errors": [],
});
let body = response.json::<Value>();
pretty_assert_eq!(body, expected);

// ❌ Avoid: Property-by-property assertions
let body = response.json::<Value>();
pretty_assert_eq!(body["written"].len(), 3);
pretty_assert_eq!(body["skipped"], serde_json::json!([]));
assert!(body["written"].contains(&key1));
```

### When Values Are Non-Deterministic

For unpredictable values (like error messages), keep property checks minimal:

```rust
// ✅ Good: Check structure separately
pretty_assert_eq!(body["written"], serde_json::json!([]));
pretty_assert_eq!(body["errors"].as_array().unwrap().len(), 1);
assert!(body["errors"][0]["error"].as_str().unwrap().contains("expected substring"));
```

**See** `references/assertion-patterns.md` for more examples

## Parameterized Tests

Use `simple_test_case` for tests with multiple variations:

```rust
use simple_test_case::test_case;

#[test_case("--flag"; "long")]
#[test_case("-f"; "short")]
#[test]
fn parses_flag(flag: &str) {
    let args = parse(vec![flag]);
    let expected = vec![Flag];
    pretty_assert_eq!(args, expected);
}
```

Each case runs independently: `parses_flag::long`, `parses_flag::short`

**See** `references/parameterized-tests.md` for testing multiple input formats

## Running Tests

Use cargo nextest:
```bash
cargo nextest run -p {PACKAGE_NAME}
```

### Workflow

1. Write tests
2. Run tests for the package
3. If successful, commit
4. If tests fail, fix issues before committing

## When to Use This Skill

Invoke when:
- Writing new tests
- Reviewing test code
- Debugging test failures
- Setting up test patterns for new modules
