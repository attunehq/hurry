# Control Flow Patterns

## Let-Else Pattern

Prefer `let Some(value) = option else { ... }` over checking `.is_none()` and using `.unwrap()`:

```rust
// ❌ Avoid
if value.is_none() {
    return handle_none();
}
let value = value.unwrap();

// ✅ Prefer
let Some(value) = value else {
    return handle_none();
};
```

This makes the control flow explicit and immune to bugs from forgetting to handle the None case.

## Benefits

- **Explicit control flow**: Early return is clear and visible
- **Immune to refactoring bugs**: Can't accidentally forget to handle None
- **More concise**: Less code to express the same logic
- **Type-safe**: Compiler enforces exhaustive handling

## When to Use

Use let-else when:
- You need to return early on None/Err
- The success case continues with the unwrapped value
- The error handling is simple (return, continue, break)

## Related Patterns

For multiple option checks:

```rust
// ✅ Chain let-else for multiple checks
let Some(config) = load_config() else {
    return Err("Config not found");
};

let Some(db) = connect_db(&config) else {
    return Err("DB connection failed");
};

// Continue with config and db
```
