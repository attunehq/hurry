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

Import with prefixes to avoid shadowing (see hurry/tests/it/passthrough.rs:7 or hurry/src/cargo/build_args.rs:650):

```rust
use pretty_assertions::assert_eq as pretty_assert_eq;
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

Use `simple_test_case` for tests with multiple variations (see hurry/tests/it/passthrough.rs:55-65 or hurry/src/cargo/build_args.rs:655-662):

```rust
use simple_test_case::test_case;

#[test_case("--release"; "long")]
#[test_case("-r"; "short")]
#[test]
fn parses_release_flag(flag: &str) {
    let args = CargoBuildArguments::from_iter(vec![flag]);
    assert!(args.is_release());
    pretty_assert_eq!(args.profile(), Some("release"));
}
```

Each case runs independently: `parses_release_flag::long`, `parses_release_flag::short`

**See** `references/parameterized-tests.md` for testing multiple input formats

## Running Tests

Use cargo nextest:
```bash
cargo nextest run -p {PACKAGE_NAME}
```

Available packages: `hurry`, `courier`, `clients`, `e2e`

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
