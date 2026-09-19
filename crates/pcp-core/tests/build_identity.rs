#[allow(dead_code)]
#[path = "../build_support.rs"]
mod build_support;

use std::{
    fs,
    path::PathBuf,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "pcp-build-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(path.join("crates/example/src")).unwrap();
        fs::write(path.join("Cargo.toml"), "[workspace]\n").unwrap();
        fs::write(
            path.join("crates/example/src/lib.rs"),
            "pub fn original() {}\n",
        )
        .unwrap();
        Self(path)
    }
    fn git(&self, args: &[&str]) {
        let output = Command::new("git")
            .arg("-C")
            .arg(&self.0)
            .args([
                "-c",
                "user.name=PCP Test",
                "-c",
                "user.email=test@example.invalid",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    fn digest(&self) -> String {
        build_support::source_digest(&self.0, build_support::INPUTS)
            .unwrap()
            .0
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn source_identity_tracks_content_additions_and_deletions_without_runtime_data() {
    let root = Fixture::new();
    let initial = root.digest();
    fs::create_dir_all(root.0.join("data")).unwrap();
    fs::write(
        root.0.join("data/private.json"),
        "must not enter build identity",
    )
    .unwrap();
    fs::create_dir_all(root.0.join("crates/example/node_modules")).unwrap();
    fs::write(
        root.0.join("crates/example/node_modules/generated"),
        "ignored",
    )
    .unwrap();
    assert_eq!(initial, root.digest());
    let new_file = root.0.join("crates/example/src/new.rs");
    fs::write(&new_file, "new input").unwrap();
    let added = root.digest();
    assert_ne!(initial, added);
    fs::write(&new_file, "changed input").unwrap();
    assert_ne!(added, root.digest());
    fs::remove_file(new_file).unwrap();
    assert_eq!(initial, root.digest());
    let (_, watched) = build_support::source_digest(&root.0, build_support::INPUTS).unwrap();
    assert!(
        watched.contains(&root.0.join("crates/example/src")),
        "Cargo must observe new files"
    );
}

#[test]
fn git_provenance_is_unknown_without_git_and_tracks_clean_dirty_and_new_commits() {
    let root = Fixture::new();
    assert_eq!(
        build_support::git_identity(&root.0, build_support::INPUTS).0,
        None
    );
    root.git(&["init", "--quiet"]);
    root.git(&["add", "."]);
    root.git(&["commit", "--quiet", "-m", "fixture"]);
    let (first, dirty, watched) = build_support::git_identity(&root.0, build_support::INPUTS);
    assert_eq!(dirty, Some(false));
    assert!(first.is_some());
    assert!(watched.iter().any(|p| p.ends_with("HEAD")));
    fs::write(root.0.join("crates/example/src/new.rs"), "untracked input").unwrap();
    assert_eq!(
        build_support::git_identity(&root.0, build_support::INPUTS).1,
        Some(true)
    );
    root.git(&["add", "."]);
    assert_eq!(
        build_support::git_identity(&root.0, build_support::INPUTS).1,
        Some(true)
    );
    root.git(&["commit", "--quiet", "-m", "next"]);
    let (second, dirty, _) = build_support::git_identity(&root.0, build_support::INPUTS);
    assert_ne!(first, second);
    assert_eq!(dirty, Some(false));
    assert_eq!(
        build_support::git_identity(&root.0.join("crates/example"), &["src"]).0,
        None,
        "nested source archives cannot borrow the enclosing repository identity"
    );
}

#[test]
fn compiled_identity_matches_the_package_version_and_roundtrips() {
    let build = pcp_core::BuildInfo::current();
    assert_eq!(build.version, env!("CARGO_PKG_VERSION"));
    assert_eq!(build.source_digest.len(), 64);
    assert!(build.source_digest.bytes().all(|c| c.is_ascii_hexdigit()));
    assert!(!build.target.is_empty());
    let decoded: pcp_core::BuildInfo =
        serde_json::from_value(serde_json::to_value(&build).unwrap()).unwrap();
    assert_eq!(build, decoded);
}
