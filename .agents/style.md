# Rust Code Style Guide

## String Creation
- Use `String::from("...")` instead of `"...".to_string()`
- Use `String::new()` instead of `"".to_string()`

## Type Annotations
**CRITICAL**: Left-hand-side type annotations are FORBIDDEN. Never use `let foo: Type = ...` syntax.
- Always prefer type inference when possible
- Use turbofish syntax (postfix types) when explicit types are needed
- ❌ NEVER: `let foo: Vec<_> = items.collect()`
- ❌ NEVER: `let mut data: serde_json::Value = parse(input)`
- ✅ ALWAYS: `let foo = items.collect::<Vec<_>>()`
- ✅ ALWAYS: `let foo = items.collect_vec()` (with itertools)
- ✅ ALWAYS: `let mut data = parse(input)` (type inference)

This rule applies to ALL variable declarations including:
- Function bodies
- Match arms
- Closures
- Struct fields (use turbofish on constructor, not field types)
- Test code

Only exception: function signatures and struct/enum definitions where type annotations are syntactically required.

## Control Flow
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

## Functional Style Over Mutation
**CRITICAL**: Prefer functional/immutable patterns over mutation. Treat `mut` as essentially banned unless absolutely necessary.

Use functional iterator methods instead of mutable loops:
```rust
// ❌ Avoid: mutable accumulation
let mut results = Vec::new();
for item in items {
    if let Some(value) = process(item) {
        results.push(value);
    }
}

// ✅ Prefer: functional style with filter_map
let results = items
    .into_iter()
    .filter_map(process)
    .collect::<Vec<_>>();
```

Use `find_map` for early-exit searches instead of mutable state:
```rust
// ❌ Avoid: mutable search
let mut found = None;
for outer in items {
    for inner in outer.children {
        if matches_condition(&inner) {
            found = Some(inner);
            break;
        }
    }
    if found.is_some() {
        break;
    }
}

// ✅ Prefer: functional find_map
let found = items
    .iter()
    .find_map(|outer| {
        outer.children.iter().find(|inner| matches_condition(inner))
    });
```

Acceptable uses of `mut`:
- Streaming I/O with readers/writers (e.g., `std::io::copy`, `tokio::io::copy`)
- Performance-critical code where mutation measurably improves performance
- FFI or unsafe code where mutation is required
- Building complex data structures where functional style is genuinely awkward

In all other cases, use functional combinators like `map`, `filter`, `filter_map`, `fold`, `find`, `find_map`, etc.

## Array Indexing
Avoid array indexing when possible. Use iterator methods instead:

**Using indices in loops:** Use `.enumerate()` to get both index and value:
```rust
// ❌ Avoid
for i in 0..items.len() {
    let item = &items[i];
    process(i, item);
}

// ✅ Prefer
for (i, item) in items.iter().enumerate() {
    process(i, item);
}
```

**Accessing elements in maps:** Use `.iter().map()` with destructuring:
```rust
// ❌ Avoid
let keyed = (0..items.len()).map(|i| (items[i].name, compute_key(i)));

// ✅ Prefer
let keyed = items
    .iter()
    .enumerate()
    .map(|(i, item)| (&item.name, compute_key(i)));
```

**Building expected values from restored data:** Use `.iter().map()` to pair iteration with indices:
```rust
// ❌ Avoid
let expected = (0..count)
    .map(|i| ExpectedValue::new(restored[i].clone(), ...))
    .collect::<Vec<_>>();

// ✅ Prefer
let expected = restored
    .iter()
    .enumerate()
    .map(|(i, value)| ExpectedValue::new(value.clone(), ...))
    .collect::<Vec<_>>();
```

Benefits:
- Eliminates bounds-checking concerns
- More readable intent
- Reduces off-by-one errors
- More functional and idiomatic Rust

Exceptions:
- Macros requiring individual arguments (e.g., `tokio::join!`) may require indexing unavoidably

## Naming Conventions

### Type Names: Avoid Stuttering
When a type is already namespaced by its module, don't repeat context in the type name.

- ❌ `storage::CasStorage` — stutters "storage"
- ✅ `storage::Disk` — describes implementation

- ❌ `db::Database` — generic, stutters "db"
- ✅ `db::Postgres` — specific implementation

- ❌ `cache::KeyCache` — stutters "cache"
- ✅ `cache::Memory` — describes mechanism

- ❌ `auth::JwtManager` — "manager" adds no value
- ✅ `auth::Jwt` — concise, module provides context

### Enum Variant Names
For enums with multiple variants for the same logical concept, use a single canonical variant:
```rust
// ❌ Avoid separate variants for aliases
enum Arg {
    Package(String),
    PackageShort(String),  // -p
}

// ✅ Use alias function to normalize to canonical form
enum Arg {
    Package(String),  // both --package and -p parse to this
}

fn alias(s: &str) -> &str {
    match s {
        "-p" => "--package",
        _ => s,
    }
}
```

Benefits:
- Single representation ensures consistent behavior
- Simpler pattern matching (no need to handle multiple variants)
- Clear canonical form for serialization/rendering

### Function Names
- Don't prefix test functions with `test_` (avoid stuttering)
- ❌ `fn test_parses_config()`
- ✅ `fn parses_config()`

### Variable Names
- Don't use hungarian notation; prefer just shadowing
- ❌: `formats_str`
- ✅: `formats`

## Import Style
Prefer direct imports over fully qualified paths unless ambiguous:

```rust
// ✅ Prefer: Import and use directly
use client::courier::v1::{Key, cache::ArtifactFile};

let key = Key::from_hex(&hex_string)?;
let artifact = ArtifactFile::builder()
    .object_key(key)
    .build();

// ❌ Avoid: Fully qualified paths when unambiguous
let key = client::courier::v1::Key::from_hex(&hex_string)?;
let artifact = client::courier::v1::cache::ArtifactFile::builder()
    .object_key(key)
    .build();
```

**Exceptions where fully qualified paths are preferred:**
- When the function/type name is ambiguous or unclear on its own (e.g., `serde_json::to_string` is clearer than a freestanding `to_string`)
- When multiple types with the same name exist in scope (use one-level-up imports or aliases)
- When the import would create naming conflicts

## String Formatting
- Always inline rust variables in format-like strings if they can be inlined
- Plain variables can be inlined: `format!("Hello, {name}")`
- Expressions cannot be inlined: `format!("Hello, {}", user.name())`

## Module Structure
- Do not use `mod.rs`. Always prefer to create Rust modules using a `.rs` file, then put other files inside a directory with the same name
  - ✅ Good: `my_module.rs`, `my_module/other_file.rs`
  - ❌ Bad: `my_module/mod.rs`, `my_module/other_file.rs`

## Dependency Management
- When adding packages to `Cargo.toml`, use `cargo add` instead of adding the package manually
- After adding all packages, run `cargo autoinherit` to update workspace dependencies

## Code Quality
- After writing a batch of Rust changes, use `make format` to format code
- After writing a batch of Rust changes, run `cargo clippy` on the project
- Prefer operations like `Itertools::sorted` over `Vec::sort` if we're going to work with the collection as an iterator

## Error Handling
- Use `color-eyre` for error handling and reporting
- Only panic if the problem is an invariant violation that makes it impossible for the program to continue safely, or in test code
- Prefer returning `Result` types for recoverable errors

## Documentation and Comments
- Don't bold bullet points in markdown
  - ❌ `- **Hook**: message`
  - ✅ `- Hook: message`
- Avoid the "space dash space" pattern when writing prose/comments, use ":" instead
  - ❌: "All commands work the same way - do x then y"
  - ✅: "All commands work the same way: do x then y"

## Comments
- **IMPORTANT**: Only write comments that explain WHY, not WHAT
- Don't write comments that restate what the code does
- If you don't know why something is done (because the user hasn't explained), DO NOT add comments
- Let the user add comments when they understand the context
- Good comment: `// Use atomic rename to prevent partial reads during concurrent access`
- Bad comment: `// Rename the temp file to the target path`
