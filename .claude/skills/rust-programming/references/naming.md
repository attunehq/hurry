# Naming Conventions

## Type Names: Avoid Stuttering

When a type is already namespaced by its module, don't repeat context in the type name.

### Examples

**Storage Types:**
- ❌ `storage::CasStorage` — stutters "storage"
- ✅ `storage::Disk` — describes implementation

**Database Types:**
- ❌ `db::Database` — generic, stutters "db"
- ✅ `db::Postgres` — specific implementation

**Cache Types:**
- ❌ `cache::KeyCache` — stutters "cache"
- ✅ `cache::Memory` — describes mechanism

**Auth Types:**
- ❌ `auth::JwtManager` — "manager" adds no value
- ✅ `auth::Jwt` — concise, module provides context

## Enum Variant Names

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

### Benefits

- Single representation ensures consistent behavior
- Simpler pattern matching (no need to handle multiple variants)
- Clear canonical form for serialization/rendering

## Function Names

Don't prefix test functions with `test_` (avoid stuttering):

- ❌ `fn test_parses_config()`
- ✅ `fn parses_config()`

The `#[test]` attribute already indicates it's a test.

## Variable Names

Don't use hungarian notation; prefer shadowing:

- ❌: `formats_str`
- ✅: `formats`

Rust's type system and compiler make type suffixes unnecessary.
