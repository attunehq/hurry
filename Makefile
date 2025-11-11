.PHONY: help format check check-fix autoinherit machete machete-fix precommit dev release sqlx-prepare install install-dev reset-local-cache

.DEFAULT_GOAL := help

help:
	@echo "Available commands:"
	@echo "  make format            - Format code with cargo +nightly fmt"
	@echo "  make check             - Run clippy linter"
	@echo "  make check-fix         - Run clippy with automatic fixes"
	@echo "  make precommit         - Run checks and automated fixes before committing"
	@echo "  make dev               - Build in debug mode"
	@echo "  make release           - Build in release mode"
	@echo "  make sqlx-prepare      - Prepare sqlx metadata for courier and hurry"
	@echo "  make install           - Install hurry locally"
	@echo "  make install-dev       - Install hurry locally, renaming to 'hurry-dev'"
	@echo "  make reset-local-cache - Reset local courier instance (docker down, clear data, migrate)"

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
	cargo machete --fix || true

precommit: machete-fix autoinherit check-fix format sqlx-prepare

dev:
	cargo build

release:
	cargo build --release

sqlx-prepare:
	cargo sqlx prepare --database-url $(COURIER_DATABASE_URL) --workspace

install:
	@CARGO_HOME=$${CARGO_HOME:-$$HOME/.cargo} && \
		EXISTING_HURRY=$$(which hurry 2>/dev/null || echo "") && \
		if [ -n "$$EXISTING_HURRY" ] && [ "$$EXISTING_HURRY" != "$$CARGO_HOME/bin/hurry" ]; then \
			EXISTING_VERSION=$$($$EXISTING_HURRY --version 2>/dev/null || echo "unknown version"); \
			echo "WARNING: Found existing '$$EXISTING_VERSION' at $$EXISTING_HURRY"; \
			echo "This may conflict with the cargo-installed version at $$CARGO_HOME/bin/hurry"; \
			echo "Consider using 'make install-dev' instead to install as hurry-dev"; \
			echo ""; \
		fi
	@cargo install --path packages/hurry --locked --force
	@CARGO_HOME=$${CARGO_HOME:-$$HOME/.cargo} && \
		VERSION=$$($$CARGO_HOME/bin/hurry --version) && \
		echo "Installed '$$VERSION' to $$CARGO_HOME/bin/hurry"

install-dev:
	@cargo install --path packages/hurry --locked --force
	@CARGO_HOME=$${CARGO_HOME:-$$HOME/.cargo} && \
		mv "$$CARGO_HOME/bin/hurry" "$$CARGO_HOME/bin/hurry-dev" && \
		VERSION=$$($$CARGO_HOME/bin/hurry-dev --version) && \
		echo "Installed '$$VERSION' to $$CARGO_HOME/bin/hurry-dev"

reset-local-cache:
	docker compose down
	rm -rf .hurrydata
	docker compose up -d postgres
	cargo sqlx migrate run --source packages/courier/schema/migrations --database-url $(COURIER_DATABASE_URL)
