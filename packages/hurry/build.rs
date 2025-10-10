fn main() {
    println!("cargo:rerun-if-changed=schema/migrations");

    #[cfg(debug_assertions)]
    sqlx_env_var_tests();
}

/// sqlx's compile-time macros and #[sqlx::test] macro use DATABASE_URL. This
/// workaround allows us to use HURRY_DATABASE_URL in .env instead, avoiding
/// conflicts with other packages (like courier) that also need databases.
///
/// We only run this in debug builds so that it affects tests and local dev but
/// not production; in release builds we don't use `DATABASE_URL`.
#[cfg(debug_assertions)]
fn sqlx_env_var_tests() {
    println!("cargo:rerun-if-env-changed=HURRY_DATABASE_URL");
    if let Ok(url) = std::env::var("HURRY_DATABASE_URL") {
        println!("cargo:rustc-env=DATABASE_URL={url}");
    }
}
