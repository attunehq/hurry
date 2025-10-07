fn main() {
    println!("cargo:rerun-if-changed=schema/migrations");

    #[cfg(debug_assertions)]
    sqlx_env_var_tests();
}

/// sqlx's #[sqlx::test] macro doesn't respect the database_url_var setting in
/// sqlx.toml yet. The test runtime code hardcodes DATABASE_URL. See:
/// vendor/sqlx/sqlx-postgres/src/testing/mod.rs:42 and :93
///
/// This is a known limitation in sqlx 0.9 pre-release. Once fixed upstream,
/// this workaround can be removed.
///
/// We only run this in debug builds so that they affect tests but not
/// production; in release builds, the DATABASE_URL environment variable needs
/// to be set by the user.
#[cfg(debug_assertions)]
fn sqlx_env_var_tests() {
    println!("cargo:rerun-if-env-changed=HURRY_DATABASE_URL");
    if let Ok(url) = std::env::var("HURRY_DATABASE_URL") {
        println!("cargo:rustc-env=DATABASE_URL={url}");
    }
}
