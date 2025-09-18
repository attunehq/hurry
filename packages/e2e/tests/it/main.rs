use std::path::PathBuf;

use e2e::{Build, Command, Container};

use color_eyre::{Result, eyre::Context};

pub mod thirdparty;

#[test_log::test(tokio::test)]
async fn run_docker() -> Result<()> {
    let _ = color_eyre::install()?;

    let container = Container::new()
        .repo("docker.io/library/ubuntu")
        .tag("latest")
        .start()
        .await
        .context("start container")?;
    Command::new()
        .pwd("/")
        .name("ls")
        .arg("-alh")
        .finish()
        .run_docker(&container)
        .await
        .context("run command in docker context")?;

    println!("finished test");
    Ok(())
}

#[test_log::test(tokio::test)]
async fn build_in_docker() -> Result<()> {
    let _ = color_eyre::install()?;

    let pwd = PathBuf::from("/");
    let hurry_root = pwd.join("hurry");
    let container = Container::new()
        .repo("docker.io/library/rust") //rust:1.89.0-bookworm
        .tag("latest")
        .start()
        .await?;
    Command::new()
        .pwd(&pwd)
        .name("apt-get")
        .arg("update")
        .finish()
        .run_docker(&container)
        .await?;
    Command::new()
        .pwd(&pwd)
        .name("apt-get")
        .arg("install")
        .arg("-y")
        .arg("git")
        .finish()
        .run_docker(&container)
        .await?;
    Command::clone_github("attunehq", "hurry", &pwd, "main")
        .run_docker(&container)
        .await?;

    Build::hurry(&hurry_root).run_docker(&container).await?;
    Command::new()
        .pwd(&hurry_root)
        .name("./target/release/hurry")
        .arg("--version")
        .finish()
        .run_docker(&container)
        .await?;

    println!("finished test");
    Ok(())
}
