# Testing Strategy

## General Testing Principles
- Tests are colocated with code: Tests are written in `#[cfg(test)]` modules within source files, not in separate `tests/` directories
- Integration-style tests: Even though tests are colocated, write them integration-style (testing through public APIs) rather than unit-style (testing internal implementation details)
- Running tests: Use `cargo nextest run -p {PACKAGE_NAME}` to run tests for a specific package

## Assertions
Use `pretty_assertions` with prefixed imports:
```rust
use pretty_assertions::{
    assert_eq as pretty_assert_eq,
    assert_ne as pretty_assert_ne,
    assert_matches as pretty_assert_matches,
};
```

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

// ❌ Avoid: Using matches! when you can construct the full value
assert!(matches!(args.0[0], CargoBuildArgument::GenericFlag(ref flag) if flag == "--flag"));

// ✅ Prefer: Construct full expected value
let expected = vec![CargoBuildArgument::GenericFlag(String::from("--flag"))];
pretty_assert_eq!(args.0, expected);
```

For non-deterministic values (like error messages), keep property checks minimal and don't copy values from response bodies:
```rust
// ✅ Good: Check structure separately for unpredictable values
pretty_assert_eq!(body["written"], serde_json::json!([]));
pretty_assert_eq!(body["errors"].as_array().unwrap().len(), 1);
assert!(body["errors"][0]["error"].as_str().unwrap().contains("expected substring"));

// ❌ Bad: Copying from response body
let expected = serde_json::json!({
    "errors": [body["errors"][0].clone()],  // Don't do this!
});
```

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

Benefits:
- Each test case runs independently with clear naming (e.g., `parses_flag::long`, `parses_flag::short`)
- Test data is not monotonically increasing (use distinct names like `foo`, `bar`, `baz` instead of `value1`, `value2`, `value3`)
- Failures show which specific case failed

## Parsing with Multiple Input Formats
When implementing parsers that accept multiple input formats, test all variations:

For flags with values:
- Long form with space: `--flag value`
- Long form with equals: `--flag=value`
- Short form with space: `-f value`
- Short form with equals: `-f=value`

For list/collection inputs:
- Different delimiters: comma-separated vs space-separated
- Multiple invocations: `--flag a --flag b`
- Combined: `--flag a,b --flag c`

## Hurry Testing
- End-to-end tests: Full workflow validation in `packages/e2e/`
- Manual validation: Use `scripts/diff-*.sh` to verify cache restore accuracy
- Benchmarks: Performance regression testing via `cargo bench`

## Courier Testing
- API tests: Use `#[sqlx::test]` macro for automatic database setup with migrations
- Test isolation: Each test gets its own PostgreSQL database instance and temporary storage directory
- Test helpers: Use `test_server()` to create isolated test server, `write_cas()` for storage operations

## Database Testing
- Use `#[sqlx::test(migrator = "MIGRATOR")]` attribute for automatic test database setup
- Each test gets isolated database instance
- Migrations run automatically before tests
- No manual cleanup needed

## Test Workflow
After adding tests to a file:
1. Run tests for the package: `cargo nextest run -p {PACKAGE_NAME}`
2. If successful, commit the changes
3. If tests fail, fix the issues before committing

## Cache Correctness
hurry's core value proposition depends on cache correctness. When making changes:
1. Run `scripts/diff-mtime.sh` to verify mtime preservation
2. Run `scripts/diff-tree.sh` to verify directory structure consistency
3. Ensure end-to-end tests pass for various git scenarios
4. Test across different cargo profiles and dependency changes

**IMPORTANT**: Do NOT use mtime comparisons when deciding whether to restore from cache. mtimes are preserved but not used as cache invalidation criteria.
