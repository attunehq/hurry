# File Structure Examples

## Overview File: `.scratch/tasks.md`

```markdown
# Task Overview

## Global Context
- Using Rust 2024 edition
- Must maintain backward compatibility with existing API
- Target: Complete by end of sprint

## Current Task
In progress: `01-refactor-storage.md`

## Tasks
- [x] 00-research-architecture.md
- [ ] 01-refactor-storage.md (IN PROGRESS)
- [ ] 02-add-caching-layer.md
- [ ] 03-update-tests.md

## Cross-task Notes
- Storage refactor blocks caching implementation
- Need to update API docs after all changes complete
- Performance benchmarks required before closing
```

## Task File: `.scratch/tasks/01-refactor-storage.md`

```markdown
# Task: Refactor Storage Layer

## Goal
Refactor the storage layer to use a trait-based design for easier testing and future implementations.

## Context
- Current implementation in `src/storage.rs` is tightly coupled to disk operations
- Need to support in-memory storage for testing
- Future: may need S3 or other cloud storage backends
- Decision: Use async traits for consistency with API layer

## Files to Modify
- `src/storage.rs`: Extract trait definition
- `src/storage/disk.rs`: Move disk implementation here
- `src/storage/memory.rs`: New in-memory implementation
- `src/api/v1/cas.rs`: Update to use trait instead of concrete type
- `src/api/v1/cas_test.rs`: Switch to in-memory storage
- `src/db.rs`: Storage trait integration

## Implementation Plan
1. Define `Storage` trait with async methods: `read`, `write`, `exists`
2. Move current implementation to `storage::Disk`
3. Create `storage::Memory` for testing
4. Update API layer to use trait
5. Update all tests to use in-memory storage
6. Add integration test with both implementations

## Progress Notes
- [x] Defined Storage trait in src/storage.rs
- [x] Created disk implementation in src/storage/disk.rs
- [x] Created memory implementation in src/storage/memory.rs
- [ ] Currently updating API layer (src/api/v1/cas.rs)
- [ ] Need to update tests
- [ ] Blocker: Unsure how to handle lifetime in trait return types

## Testing
- Run `cargo nextest run -p courier`
- Verify all existing tests pass
- Add new test: `memory_storage_roundtrip`
- Add new test: `disk_storage_persistence`
- Benchmark: Compare performance of implementations
```

## Benefits of This Structure

**Atomic Information:**
- Each section serves a specific purpose
- Easy to scan and update

**Recovery-Friendly:**
- Progress Notes show exactly what's done
- Blockers are explicit
- Files to Modify gives scope

**Context Continuity:**
- Context section preserves decisions
- Implementation Plan maintains direction
- Global Context in overview provides constraints
