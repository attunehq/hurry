# Parameterized Tests

Use `simple_test_case` to test multiple variations of the same logic.

## Basic Pattern

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

Each case runs independently with clear naming:
- `parses_flag::long`
- `parses_flag::short`

## Testing Multiple Input Formats

### Flags with Values

Test all variations:

```rust
#[test_case("--flag", "value"; "long_space")]
#[test_case("--flag=value"; "long_equals")]
#[test_case("-f", "value"; "short_space")]
#[test_case("-f=value"; "short_equals")]
#[test]
fn parses_flag_with_value(/* ... */) {
    // Test implementation
}
```

Variations to test:
- Long form with space: `--flag value`
- Long form with equals: `--flag=value`
- Short form with space: `-f value`
- Short form with equals: `-f=value`

### List/Collection Inputs

```rust
#[test_case(vec!["a", "b", "c"]; "comma_separated")]
#[test_case(vec!["a b c"]; "space_separated")]
#[test_case(vec!["a", "b", "c"]; "multiple_invocations")]
#[test]
fn parses_list(input: Vec<&str>) {
    // Test implementation
}
```

Test variations:
- Different delimiters: comma vs space
- Multiple invocations: `--flag a --flag b`
- Combined: `--flag a,b --flag c`

## Benefits

- **Independent execution**: Each case runs separately
- **Clear failure reporting**: Know exactly which case failed
- **Non-monotonic test data**: Use distinct names (`foo`, `bar`, `baz`) instead of numbers
- **Maintainability**: Add/remove cases without affecting others

## Test Data Naming

Use descriptive, non-numeric names:

```rust
// ✅ Good
#[test_case("foo"; "simple")]
#[test_case("bar"; "with_special_chars")]
#[test_case("baz"; "unicode")]

// ❌ Avoid
#[test_case("value1"; "test1")]
#[test_case("value2"; "test2")]
#[test_case("value3"; "test3")]
```
