use std::path::PathBuf;

use e2e::{Build, Command, TestEnv};

use color_eyre::{Result, eyre::Context};

pub mod thirdparty;

#[test_log::test(tokio::test)]
async fn run_compose() -> Result<()> {
    color_eyre::install()?;

    // Start test environment
    let env = TestEnv::new().await?;

    // Run a simple command in the hurry container
    Command::new()
        .pwd("/workspace")
        .name("ls")
        .arg("-alh")
        .finish()
        .run_compose(env.service(TestEnv::HURRY_INSTANCE_1)?)
        .await
        .context("run command in compose context")?;

    println!("finished test");
    Ok(())
}

#[test_log::test(tokio::test)]
async fn build_hurry_in_compose() -> Result<()> {
    color_eyre::install()?;

    // Start test environment
    let env = TestEnv::new().await?;

    let pwd = PathBuf::from("/hurry-src");

    // Build hurry (it's already installed in the image, but we can rebuild it)
    Build::new()
        .pwd(&pwd)
        .finish()
        .run_compose(env.service(TestEnv::HURRY_INSTANCE_1)?)
        .await
        .context("build hurry")?;

    // Run hurry --version to verify it works
    Command::new()
        .pwd(&pwd)
        .name("hurry")
        .arg("--version")
        .finish()
        .run_compose(env.service(TestEnv::HURRY_INSTANCE_1)?)
        .await
        .context("run hurry --version")?;

    Ok(())
}
