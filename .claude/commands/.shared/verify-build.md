Run verification steps to ensure code quality:

1. **Build**: `cargo build` to ensure the code compiles
2. **Format**: `make format` to format code
3. **Clippy**: `cargo clippy` to check for warnings/errors
4. **Tests** (if feasible): `cargo nextest run -p <package>` for modified packages

Report pass/fail status for each step. If any step fails and you're unsure how to fix it, ask the user for guidance.
