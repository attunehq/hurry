# Assertion Patterns

## Core Principle: One Complete Assertion

Construct the full expected value, then assert once.

### Good Pattern

```rust
let expected = serde_json::json!({
    "written": [key1, key2, key3],
    "skipped": [],
    "errors": [],
});
let body = response.json::<Value>();
pretty_assert_eq!(body, expected);
```

### Anti-Pattern: Property-by-Property

```rust
// ❌ Avoid this
let body = response.json::<Value>();
pretty_assert_eq!(body["written"].len(), 3);
pretty_assert_eq!(body["skipped"], serde_json::json!([]));
assert!(body["written"].contains(&key1));
```

## Anti-Pattern: Using matches! for Full Values

```rust
// ❌ Avoid when you can construct the full value
assert!(matches!(args.0[0], CargoBuildArgument::GenericFlag(ref flag) if flag == "--flag"));

// ✅ Prefer full value construction
let expected = vec![CargoBuildArgument::GenericFlag(String::from("--flag"))];
pretty_assert_eq!(args.0, expected);
```

## Handling Non-Deterministic Values

When error messages or timestamps are unpredictable:

```rust
// ✅ Good: Separate structure checks from content checks
pretty_assert_eq!(body["written"], serde_json::json!([]));
pretty_assert_eq!(body["errors"].as_array().unwrap().len(), 1);
assert!(body["errors"][0]["error"].as_str().unwrap().contains("expected substring"));
```

### Anti-Pattern: Copying from Response

```rust
// ❌ Bad: Don't copy from the response you're testing
let expected = serde_json::json!({
    "errors": [body["errors"][0].clone()],  // Circular!
});
```

## Benefits

- **Clear intent**: See exactly what's expected
- **Better failure messages**: pretty_assertions shows full diff
- **Single source of truth**: Expected value is explicit
- **Easier maintenance**: Change expected in one place
