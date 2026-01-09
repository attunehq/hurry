# Requirements Document

## Introduction

This document specifies the requirements for refactoring the shared CLI arguments between `hurry cargo build` and `hurry cross build` commands. Both commands currently have duplicated option structs with identical fields for Hurry-specific arguments. This refactoring will extract the common options into a shared struct that can be flattened into both command-specific option structs using clap's `#[clap(flatten)]` attribute.

## Glossary

- **Options_Struct**: A Rust struct decorated with clap attributes that defines CLI arguments for a command
- **Shared_Options**: A struct containing CLI arguments common to both cargo and cross build commands
- **Cargo_Build_Options**: The command-specific options struct for `hurry cargo build`
- **Cross_Build_Options**: The command-specific options struct for `hurry cross build`
- **Flatten**: A clap attribute that embeds all fields from one Args struct into another

## Requirements

### Requirement 1: Identify Shared Arguments

**User Story:** As a developer, I want shared CLI arguments to be defined in a single location, so that I can maintain consistency and reduce code duplication.

#### Acceptance Criteria

1. THE Shared_Options struct SHALL contain the `api_url` field with long flag `--hurry-api-url`, environment variable `HURRY_API_URL`, and default value `https://app.hurry.build`
2. THE Shared_Options struct SHALL contain the `api_token` field with long flag `--hurry-api-token` and environment variable `HURRY_API_TOKEN`
3. THE Shared_Options struct SHALL contain the `skip_backup` field with long flag `--hurry-skip-backup` and default value `false`
4. THE Shared_Options struct SHALL contain the `skip_build` field with long flag `--hurry-skip-build` and default value `false`
5. THE Shared_Options struct SHALL contain the `skip_restore` field with long flag `--hurry-skip-restore` and default value `false`
6. THE Shared_Options struct SHALL contain the `async_upload` field with long flag `--hurry-async-upload`, environment variable `HURRY_ASYNC_UPLOAD`, and default value `false`

### Requirement 2: Create Shared struct in main `cmd.rs` module

**User Story:** As a developer, I want the shared options to be in the main `cmd.rs` module, so that both cargo and cross commands can import and use them while avoiding a "utility module" that just contains types and nothing else.

#### Acceptance Criteria

1. WHEN the shared options struct is created, THE system SHALL place it at a location accessible to both cargo and cross build commands
2. THE Shared_Options struct SHALL derive `Clone`, `Args`, and `Debug`
3. THE Shared_Options struct SHALL preserve all existing documentation comments for each field

### Requirement 3: Flatten Shared Options into Command Structs

**User Story:** As a developer, I want to use clap's flatten attribute to embed shared options, so that the CLI interface remains unchanged for users.

#### Acceptance Criteria

1. WHEN Shared_Options is flattened into Cargo_Build_Options, THE system SHALL preserve all existing CLI argument names and behaviors
2. WHEN Shared_Options is flattened into Cross_Build_Options, THE system SHALL preserve all existing CLI argument names and behaviors
3. THE Cargo_Build_Options struct SHALL retain the `help` field as command-specific
4. THE Cross_Build_Options struct SHALL retain the `help` field as command-specific
5. THE Cargo_Build_Options struct SHALL retain the `argv` field as command-specific
6. THE Cross_Build_Options struct SHALL retain the `argv` field as command-specific

### Requirement 4: Maintain API Compatibility

**User Story:** As a user, I want the CLI interface to remain unchanged after the refactoring, so that my existing scripts and workflows continue to work.

#### Acceptance Criteria

1. WHEN a user runs `hurry cargo build --hurry-api-url <url>`, THE system SHALL accept the argument as before
2. WHEN a user runs `hurry cross build --hurry-api-token <token>`, THE system SHALL accept the argument as before
3. WHEN a user sets `HURRY_API_URL` environment variable, THE system SHALL use it for both cargo and cross build commands
4. WHEN a user sets `HURRY_ASYNC_UPLOAD` environment variable, THE system SHALL use it for both cargo and cross build commands

### Requirement 5: Update Command Implementations

**User Story:** As a developer, I want the exec functions to access shared options through the flattened struct, so that the implementation remains clean and maintainable.

#### Acceptance Criteria

1. WHEN the cargo build exec function accesses `api_url`, THE system SHALL access it through the flattened shared options
2. WHEN the cross build exec function accesses `api_token`, THE system SHALL access it through the flattened shared options
3. THE cargo build exec function SHALL continue to function identically after the refactoring
4. THE cross build exec function SHALL continue to function identically after the refactoring
