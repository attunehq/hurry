# Architectural Patterns

## Async Patterns

### Runtime and Concurrency
- Use Tokio runtime: `#[tokio::main]` for binaries
- `tokio::spawn_blocking()` for CPU-bound work to avoid blocking the async executor
- `tokio::join!()` for concurrent operations that need to complete together
- `tokio::select!()` for async signal handling and cancellation
- `tokio::task::JoinSet` for managing multiple concurrent tasks

### Graceful Shutdown
- Use `.with_graceful_shutdown()` on web servers (Axum)
- Implement signal handlers with `tokio::signal::ctrl_c()`
- Ensure cleanup happens before process exits

### Buffer Sizes
- Define buffer size constants at module level
- Use large buffers (64KB) for network I/O and file operations
- Example: `const DEFAULT_BUF_SIZE: usize = 64 * 1024;`

## Type Design Patterns

### Phantom Types for Compile-Time Safety
Use phantom types to enforce state machines and constraints at compile time:

```rust
// Example: State machine for a resource
pub struct Idle;
pub struct Active;

pub struct Resource<State> {
    data: SomeData,
    _state: PhantomData<State>,
}

impl Resource<Idle> {
    pub fn activate(self) -> Result<Resource<Active>> {
        // transition to active state
    }
}

impl Resource<Active> {
    pub fn deactivate(self) -> Result<Resource<Idle>> {
        // transition back to idle state
    }
}
```

This prevents calling methods on incorrect states at compile time.

### Newtypes for Domain Modeling
- Wrap primitives in newtypes for type safety: `struct UserId(u64)`, `struct Email(String)`
- Keep fields private and provide smart constructors
- **DO NOT implement `Deref`**: newtypes should be distinct types, not transparent wrappers
- Implement domain-specific validation in constructors (parse, don't validate)
- Use factory methods for different construction paths: `Email::from_str()`, `UserId::new()`
- See: https://lexi-lambda.github.io/blog/2019/11/05/parse-don-t-validate/

### Exhaustive Field Destructuring for Future-Proofing

When writing methods that must process **all fields** of a struct (like hashing, serialization, or validation), use exhaustive destructuring to get compile-time errors when fields are added:

```rust
pub struct CacheKey {
    pub unit_hash: String,
    // Future: will add libc_version, etc.
}

impl CacheKey {
    pub fn compute_hash(&self) -> String {
        // This pattern forces you to handle new fields when they're added
        let Self { unit_hash } = self;

        let mut hasher = blake3::Hasher::new();
        hasher.update(unit_hash.as_bytes());
        hasher.finalize().to_hex().to_string()
    }
}
```

**When to use this pattern:**
- Methods that must include **all fields** in their computation
- Hashing functions (where missing a field breaks equality)
- Serialization/deserialization that must be exhaustive
- Validation that checks every field

**Why this works:**
When you add a new field like `libc_version`, the destructuring pattern will fail to compile with "pattern requires `..` due to inaccessible fields". This forces you to update the method intentionally.

**What to include in the method:**
- Add a comment explaining why exhaustive destructuring is used
- Mention what should be done when the compile error occurs

```rust
// When we add new fields, this will show a compile time error; if you got here
// due to a compilation error please handle the new field(s) appropriately.
let Self { unit_hash } = self;
```

**When NOT to use this pattern:**
- Methods that only need a subset of fields (just access them directly)
- Hot path code where the compiler might not optimize away the destructuring
- When the struct has many fields but the method only needs a few

**Real example:** See `SavedUnitCacheKey::stable_hash()` in `packages/clients/src/courier/v1/cache.rs`

### Type Aliases for Complex Generics
Define type aliases to simplify complex generic types:

```rust
pub type Result<T> = std::result::Result<T, Error>;
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
pub type HandlerResult = Result<Response>;
```

### Derive Macros
- Use `bon` for builder pattern: `#[derive(Builder)]`
- Use `derive_more` for common traits: `#[derive(Display)]` with `#[display("{path}")]`
- Use `duplicate` to generate lots of duplicate code (see the `TypedPath` code)
- Prefer to co-locate simple functionality with the type declaration using macros
  - `derive_more::Display` instead of `impl Display {...}`
  - `enum_assoc` for simple functions on enum variants

## Builder Patterns

### Prefer Derive Macros
- Use `bon` from workspace dependencies
- Add `#[derive(Builder)]` to structs that need builder pattern
- Generates type-safe builders automatically

### Manual Builders with Typestate
Only use manual builders when you need compile-time state transitions:
- Phantom type parameters track builder state
- Each state has its own `impl` block with allowed methods
- See phantom type pattern above for state machine examples

## HTTP/API Patterns

### Route Organization
- Nest routes by version and feature: `Router::new().nest("/v1/users", ...).nest("/v1/posts", ...)`
- One handler function per file in feature directories
- Group related handlers in modules (e.g., `api/v1/users/create.rs`, `api/v1/users/update.rs`)

### Response Types
Create custom response enums that implement `IntoResponse`:

```rust
pub enum ApiResponse {
    Success { data: Value },
    NotFound,
    Error(Report),
}

impl IntoResponse for ApiResponse {
    fn into_response(self) -> Response {
        match self {
            Self::Success { data } => (StatusCode::OK, Json(data)).into_response(),
            Self::NotFound => StatusCode::NOT_FOUND.into_response(),
            Self::Error(err) => (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response(),
        }
    }
}
```

Benefits:
- Type-safe response construction
- Clear documentation of possible responses
- Centralized status code mapping
- Can automate OpenAPI specs using this pattern with `utoipa`

### Dependency Injection with Aerosol
- Define state type: `pub type State = Aero![Database, Cache, Config];`
- **IMPORTANT**: Dependencies are listed in reverse order of how they'll be extracted in handlers
- Extract in handlers: `Dep(db): Dep<Database>, Dep(cache): Dep<Cache>`
- Each dependency is automatically provided by Axum's state system

### Middleware Stack
Order matters—apply in this sequence:
1. Request tracing (with UUID generation)
2. Decompression
3. Compression
4. Body size limits
5. Timeouts

Define appropriate constants for your use case:
```rust
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_BODY_SIZE: usize = 10 * 1024 * 1024; // 10MB
```

### Request Tracing
- Generate request ID using UUID for each request
- Use `#[instrument]` on handler functions
- Add request ID to tracing span for correlation
- Log method, URL, status, and duration

## Database Patterns

### Connection and Pooling
- Use `sqlx::PgPool` (or `MySqlPool`, `SqlitePool`) for connection pooling
- Wrap pool in domain type (e.g., `struct Database { pool: PgPool }`)
- Use lazy connection: pool creation is async
- Store migration runner as associated const for test compatibility

### Query Patterns
- Use `sqlx::query!()` macro for compile-time checked queries
- Prefer prepared statements over dynamic SQL
- Use transactions for multi-statement operations: `pool.begin().await?`

### Upsert Pattern
```rust
sqlx::query!(
    "INSERT INTO table (id, data) VALUES ($1, $2) ON CONFLICT (id) DO NOTHING",
    id,
    data
)
.execute(&pool)
.await?
```

### Migration Management
- Define schema in canonical source file (hand-maintained)
- Generate migrations as needed with migration tools
- Store migrations in version-controlled directory structure
- Never auto-apply on startup (prevents accidental production changes)
- Apply manually via migration tool before deployments

## File I/O Patterns

### Custom Filesystem Abstraction
- Route filesystem operations through custom abstraction layer when needed
- Can enforce via Clippy configuration in project
- Provides consistent error handling, async interface, and tracing
- Centralizes platform-specific behavior

### Atomic Writes
Use write-then-rename pattern for atomic file creation:
```rust
// Write to temporary file
let temp_path = target.with_extension(".tmp");
let mut file = File::create(&temp_path).await?;
file.write_all(&data).await?;
file.sync_all().await?;

// Atomic rename
tokio::fs::rename(temp_path, target).await?;
```

### Concurrency Control
- Define concurrency limits as constants: `const MAX_CONCURRENT_OPS: usize = 10;`
- Use semaphores, `JoinSet`, or stream buffering to limit concurrent operations
- Prevents overwhelming filesystem or network resources

### Typed Paths
- Use phantom types to distinguish path properties at compile time (absolute/relative, file/directory)
- Define type aliases for common path types
- Use compile-time macros for path validation when applicable
- Custom serde serialization for path types

## Error Handling Patterns

### Context Extension for Async
Create extension traits for cleaner async error handling using the `extfn` macro:

```rust
use extfn::extfn;

#[extfn]
pub async fn then_context<F, T>(future: F, msg: &str) -> Result<T>
where
    F: Future<Output = Result<T>>,
{
    future.await.context(msg)
}

// Usage
let result = async_operation()
    .then_context("operation failed")
    .await?;
```

Benefits:
- Reads more naturally with async chains
- No intermediate `await` needed before adding context
- `extfn` macro generates the extension trait boilerplate automatically

### Structured Error Reports
Use `Section` and `SectionExt` from color-eyre for multi-component errors:

```rust
bail!(
    "operation failed"
        .section(format!("Component A: {status_a}").header("A Status:"))
        .section(format!("Component B: {status_b}").header("B Status:"))
);
```

This creates well-formatted error reports with multiple sections of context.

## Logging and Tracing

### Instrumentation
- Add `#[instrument]` to all public async functions
- Skip large arguments: `#[instrument(skip(data))]`
- Add fields to spans: `#[instrument(fields(key = %key))]`
- Use `info!`, `debug!`, `warn!`, `error!` macros with structured fields

### Environment Configuration
- Use custom environment variables for log filtering
- Common patterns: `APP_LOG` or use standard `RUST_LOG`
- Default to INFO level with ability to override
- Support hierarchical filtering: `myapp::module=debug,myapp=info`

### Custom Formatters
- Implement custom timer formatters for specialized output
- Example: `Uptime` timer for relative timing instead of timestamps
- Useful for CLI tools where absolute times are less relevant

## Serialization Patterns

### Separation of Domain and DTO
- Domain types should NOT implement `Serialize`/`Deserialize`
- Create separate DTO types for serialization
- Convert between domain and DTO at API boundaries
- Prevents tight coupling between domain model and wire format

### Custom Serialization
Implement custom serialization for domain types when needed:

```rust
impl Serialize for DomainType {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for DomainType {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Self::from_str(&s).map_err(serde::de::Error::custom)
    }
}
```

### Format Preferences
- JSON for API communication (`serde_json`)
- TOML for configuration files
- Derive-based serialization for internal types

## Configuration and Constants

### Constants
Define constants when they're used in multiple places or when the value itself is worth documenting:

```rust
pub const NETWORK_BUFFER_SIZE: usize = 1024 * 1024; // 1MB
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
```

**Don't create constants if there's only one logical instance of the value** unless the constant name adds important documentation. If a value appears only once, prefer using it inline.

### Environment Variable Naming
- Use uppercase with underscores
- Prefix with app/crate name: `MYAPP_LOG`, `MYAPP_DATABASE_URL`
- Document in CLI help and README
- Support via Clap: `#[arg(long, env = "MYAPP_CONFIG")]`

### Lazy Static Initialization
Use `std::sync::LazyLock` for lazy initialization of expensive constants:

```rust
static CONFIG: LazyLock<Config> = LazyLock::new(|| {
    Config::load().expect("valid config")
});
```

## Workspace Organization

### Shared Dependencies
- Define all versions in workspace `Cargo.toml` under `[workspace.dependencies]`
- Packages reference with: `dependency = { workspace = true }`
- After `cargo add`, run `cargo autoinherit` to move version to workspace level
- Ensures version consistency across packages

### Cross-Package Imports
- Use workspace dependencies: `mypackage = { path = "../mypackage" }`
- Re-export common types for convenience: `pub type Client = internal::v1::Client;`
- Organize shared code in dedicated packages (e.g., `common`, `shared`, `core`)

### Feature Flags
- Use features to gate optional functionality
- Document features in package `Cargo.toml`
- Use `#[cfg(feature = "...")]` to conditionally compile code
- Example: `#[cfg(feature = "client")]` for HTTP client code

## CLI Patterns

### Argument Parsing
- Use Clap with derive macros: `#[derive(Parser)]`
- Support both short and long flags
- Normalize to canonical form early in parsing
- Support both space-separated and `=`-separated flag values

### Argument Roundtripping
Design parsers to support roundtripping: parsed args should convert back to argv:

```rust
let args = parse_args(argv);
let new_argv = args.to_argv();
// new_argv should be equivalent to original argv
```

Benefits:
- Enables argument forwarding to wrapped commands
- Testable behavior
- Clear canonical representation

### Command Structure
- Nested subcommands via `#[clap(subcommand)]`
- Each command in separate module: `cmd/cache.rs`, `cmd/debug.rs`
- Each command has `Options` struct and `exec()` function
- Options include both CLI flags and dependencies

## Utility Patterns

### Pipe and Tap
Use `tap` crate for functional composition:

```rust
use tap::{Pipe, Tap};

// Transform values inline
let result = calculate()
    .pipe(|x| x * 2)
    .tap(|x| println!("Debug: {x}"));
```

Benefits:
- Cleaner than intermediate variables
- Method-call syntax for better readability
- Useful for debugging in chains

### Iterator Preferences
- Use `Itertools::sorted()` over `Vec::sort()` when working with iterator chains
- Prefer `collect_vec()` (from itertools) over `collect::<Vec<_>>()`
- Use `.enumerate()` instead of manual indexing

## I/O Operations
- **Prefer streaming operations over buffered ones by default**
- Use `AsyncRead`, `AsyncWrite`, `Read`, `Write` traits directly
- Only buffer when there's a specific performance reason to do so
