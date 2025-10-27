//! Exercises e2e functionality for building/caching third-party dependencies on
//! the local machine.

use std::time::SystemTime;

use color_eyre::Result;
use e2e::{
    Build, Command,
    ext::{ArtifactIterExt, MessageIterExt},
    temporary_directory,
};
use itertools::Itertools;

// "Shotgun restore" is the idea that if we back up all the outputs of a build
// across multiple runs with different feature sets or other configuration
// targets, and then restore all of them together, cargo should be able to
// figure out the right one and use it.
#[tokio::test]
async fn shotgun_restore() -> Result<()> {
    color_eyre::install()?;

    // This will contain a unified view of `target/` across all builds.
    let shotgun_target = temporary_directory()?;

    // Not all of `target/` should be backed up; we only back up these
    // subdirectories (and all their contents).
    let dirs = ["debug/build", "debug/deps", "debug/.fingerprint"];

    let temp_ws = temporary_directory()?;
    let pwd = temp_ws.path();
    Command::clone_github()
        .pwd(pwd)
        .user("attunehq")
        .repo("hurry-tests")
        .branch("test/native")
        .dir(pwd)
        .finish()
        .run_local()?;

    // The intention is that shotgun restores work across all different compiler
    // configurations so we may add other configurations in the future; for this
    // test we're focused on features which are the most common varying config.
    let features = ["bundled-sqlite", "static-openssl"];
    for set in features.iter().copied().powerset() {
        println!("cleaning {:?}", temp_ws.path());
        Command::cargo_clean(temp_ws.path()).run_local()?;

        println!("building with features: {set:?}");
        Build::new().pwd(pwd).features(set).finish().run_local()?;

        // We test that we can actually run the binary because the test cases
        // contain dynamically linked native libraries; theoretically it's
        // possible for the build to succeed but the binary to fail to run
        // (although in practice this seems to be checked during the build).
        Command::new()
            .pwd(pwd)
            .name(pwd.join("target/debug/tiny"))
            .finish()
            .run_local()?;

        for dir in &dirs {
            let src = pwd.join("target").join(dir);
            let dst = shotgun_target.path().join(dir);

            println!("backing up {src:?} to {dst:?}");
            let (files, bytes) = e2e::copy_dir(src, dst)?;
            println!("backed up {files} files; {bytes} bytes");
        }
    }

    // Now that we have a target that contains the unified outputs of all the
    // builds run with different sets of features, ensure that if we use that
    // target to restore an otherwise fresh build we get fresh 3rd party
    // artifacts.
    for set in features.iter().copied().powerset() {
        println!("cleaning and restoring {:?}", temp_ws.path());
        Command::cargo_clean(pwd).run_local()?;

        // Note that when we restore the shotgunned target directory we have to
        // set the mtime for everything inside; otherwise a bunch of things get
        // marked dirty because Cargo sees that they built at different times
        // (e.g. the `rlib` was built before the `build` output etc).
        //
        // When we actually implement this we will probably want to do something
        // a little more intelligent than just brute force overwriting the
        // mtimes for everything but it's fine here (and who knows, maybe that's
        // what we'll end up doing anyway).
        let (files, bytes) = e2e::copy_dir(shotgun_target.path(), pwd.join("target"))?;
        e2e::set_mtime(pwd.join("target"), SystemTime::now())?;
        println!("restored {files} files; {bytes} bytes");

        println!("building with features: {set:?}");
        let messages = Build::new()
            .pwd(pwd)
            .features(set.clone())
            .finish()
            .run_local()?;
        Command::new()
            .pwd(pwd)
            .name(pwd.join("target/debug/tiny"))
            .finish()
            .run_local()?;

        let expected = messages
            .iter()
            .thirdparty_artifacts()
            .package_ids()
            .map(|id| (id, true))
            .sorted()
            .collect::<Vec<_>>();
        let freshness = messages
            .iter()
            .thirdparty_artifacts()
            .freshness()
            .sorted()
            .collect::<Vec<_>>();
        pretty_assertions::assert_eq!(
            expected,
            freshness,
            "all artifacts should be fresh; features: {set:?}"
        );
    }

    Ok(())
}
