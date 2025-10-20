.PHONY: help format check check-fix autoinherit machete machete-fix precommit dev release sqlx-prepare

.DEFAULT_GOAL := help

help:
	@echo "Available commands:"
	@echo "  make format       - Format code with cargo +nightly fmt"
	@echo "  make check        - Run clippy linter"
	@echo "  make check-fix    - Run clippy with automatic fixes"
	@echo "  make precommit    - Run checks and automated fixes before committing"
	@echo "  make dev          - Build in debug mode"
	@echo "  make release      - Build in release mode"
	@echo "  make sqlx-prepare - Prepare sqlx metadata for courier and hurry"

format:
	cargo +nightly fmt

check:
	cargo clippy

check-fix:
	cargo clippy --fix --allow-dirty --allow-staged

autoinherit:
	cargo autoinherit

machete:
	cargo machete

machete-fix:
	cargo machete --fix

precommit: machete-fix autoinherit check-fix format sqlx-prepare

dev:
	cargo build

release:
	cargo build --release

sqlx-prepare:
	cd packages/courier && cargo sqlx prepare --database-url $(COURIER_DATABASE_URL)
