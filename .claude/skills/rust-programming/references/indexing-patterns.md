# Array Indexing Patterns

Avoid array indexing in Rust. Use iterator methods for safety and clarity.

## Pattern: Using Indices in Loops

Use `.enumerate()` to get both index and value:

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

## Pattern: Accessing Elements in Maps

Use `.iter().map()` with destructuring:

```rust
// ❌ Avoid
let keyed = (0..items.len()).map(|i| (items[i].name, compute_key(i)));

// ✅ Prefer
let keyed = items
    .iter()
    .enumerate()
    .map(|(i, item)| (&item.name, compute_key(i)));
```

## Pattern: Building Expected Values from Restored Data

Use `.iter().map()` to pair iteration with indices:

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

## Benefits

- Eliminates bounds-checking concerns
- More readable intent
- Reduces off-by-one errors
- More functional and idiomatic Rust

## Exceptions

Macros requiring individual arguments (e.g., `tokio::join!`) may require indexing unavoidably.
