# Implementation Plan: Shared Build Options

## Overview

This plan refactors the shared CLI arguments between `hurry cargo build` and `hurry cross build` commands by extracting common Hurry-specific options into a shared struct in `cmd.rs` that can be flattened into both command-specific option structs.

## Tasks

- [x] 1. Create HurryOptions struct in cmd.rs
  - Add the `HurryOptions` struct with all 6 shared fields to `packages/hurry/src/bin/hurry/cmd.rs`
  - Include all clap attributes (long flags, env vars, defaults)
  - Preserve existing documentation comments
  - Derive `Clone`, `Args`, and `Debug`
  - _Requirements: 1.1, 1.2, 1.3, 1.4, 1.5, 1.6, 2.1, 2.2, 2.3_

- [ ] 2. Update cargo build Options struct
  - [x] 2.1 Refactor cargo/build.rs Options to use flattened HurryOptions
    - Replace the 6 individual fields with `#[clap(flatten)] pub hurry: super::super::HurryOptions`
    - Keep `help` and `argv` fields as command-specific
    - _Requirements: 3.1, 3.3, 3.5_
  - [x] 2.2 Update cargo build exec function to access fields through hurry
    - Change `options.api_url` to `options.hurry.api_url`
    - Change `options.api_token` to `options.hurry.api_token`
    - Change `options.skip_backup` to `options.hurry.skip_backup`
    - Change `options.skip_build` to `options.hurry.skip_build`
    - Change `options.skip_restore` to `options.hurry.skip_restore`
    - Change `options.async_upload` to `options.hurry.async_upload`
    - _Requirements: 5.1, 5.3_

- [x] 3. Update cross build Options struct
  - [x] 3.1 Refactor cross/build.rs Options to use flattened HurryOptions
    - Replace the 6 individual fields with `#[clap(flatten)] pub hurry: super::super::HurryOptions`
    - Keep `help` and `argv` fields as command-specific
    - _Requirements: 3.2, 3.4, 3.6_
  - [x] 3.2 Update cross build exec function to access fields through hurry
    - Change `options.api_url` to `options.hurry.api_url`
    - Change `options.api_token` to `options.hurry.api_token`
    - Change `options.skip_backup` to `options.hurry.skip_backup`
    - Change `options.skip_build` to `options.hurry.skip_build`
    - Change `options.skip_restore` to `options.hurry.skip_restore`
    - Change `options.async_upload` to `options.hurry.async_upload`
    - _Requirements: 5.2, 5.4_

- [x] 4. Checkpoint - Verify compilation and existing tests pass
  - Run `cargo check -p hurry` to verify compilation
  - Run `cargo nextest run -p hurry` to verify existing tests pass
  - Ensure all tests pass, ask the user if questions arise.

- [x] 5. post-implementation tweaks
  - fix `opts.opts.` stuttering
  - move `help` into the shared struct

- [ ]* 5. Write property test for argument parsing equivalence
  - **Property 1: Argument Parsing Equivalence**
  - **Validates: Requirements 1.1, 1.2, 1.3, 1.4, 1.5, 1.6, 3.1, 3.2**
  - Test that parsing various combinations of hurry flags produces correct field values

- [ ]* 6. Write property test for passthrough arguments preservation
  - **Property 2: Passthrough Arguments Preservation**
  - **Validates: Requirements 3.5, 3.6**
  - Test that argv sequences are captured correctly in both cargo and cross Options

- [ ]* 7. Write property test for environment variable consistency
  - **Property 3: Environment Variable Consistency**
  - **Validates: Requirements 4.3, 4.4**
  - Test that env vars are resolved identically for both cargo and cross commands

- [x] 8. Final checkpoint - Ensure all tests pass
  - Ensure all tests pass, ask the user if questions arise.

## Notes

- Tasks marked with `*` are optional and can be skipped for faster MVP
- Each task references specific requirements for traceability
- The refactoring is purely structural - no behavioral changes
- Field access pattern changes from `options.field` to `options.hurry.field`
