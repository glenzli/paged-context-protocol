use sha2::{Digest, Sha256};
use std::{
    env, fs, io,
    path::{Path, PathBuf},
    process::Command,
};

// Only repository inputs: never inspect PCP data, credentials, plugin caches or target/.
pub const INPUTS: &[&str] = &[
    "Cargo.toml",
    "Cargo.lock",
    "crates",
    "assets",
    "integrations/TOOL_INTEGRATION.md",
];

fn excluded(path: &Path) -> bool {
    matches!(
        path.file_name().and_then(|v| v.to_str()),
        Some("target" | "node_modules" | ".git" | ".DS_Store" | "__pycache__")
    )
}

fn collect(path: &Path, files: &mut Vec<PathBuf>, watched: &mut Vec<PathBuf>) -> io::Result<()> {
    if excluded(path) {
        return Ok(());
    }
    watched.push(path.to_owned());
    let metadata = match fs::symlink_metadata(path) {
        Ok(value) => value,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if metadata.is_dir() {
        for entry in fs::read_dir(path)? {
            collect(&entry?.path(), files, watched)?;
        }
    } else if metadata.is_file() {
        files.push(path.to_owned());
    } else if metadata.file_type().is_symlink() {
        return Err(io::Error::other(format!(
            "build identity input must not be a symlink: {}",
            path.display()
        )));
    }
    Ok(())
}

pub fn source_digest(root: &Path, inputs: &[&str]) -> io::Result<(String, Vec<PathBuf>)> {
    let mut files = Vec::new();
    let mut watched = Vec::new();
    for input in inputs {
        collect(&root.join(input), &mut files, &mut watched)?;
    }
    files.sort();
    files.dedup();
    let mut hash = Sha256::new();
    hash.update(b"pcp-source-v1\0");
    for path in files {
        let relative = path
            .strip_prefix(root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        let bytes = fs::read(&path)?;
        hash.update((relative.len() as u64).to_le_bytes());
        hash.update(relative.as_bytes());
        hash.update((bytes.len() as u64).to_le_bytes());
        hash.update(bytes);
    }
    Ok((format!("{:x}", hash.finalize()), watched))
}

fn git(root: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

pub fn git_identity(root: &Path, inputs: &[&str]) -> (Option<String>, Option<bool>, Vec<PathBuf>) {
    // A source archive inside some other checkout must not borrow that checkout's identity.
    let top = git(root, &["rev-parse", "--show-toplevel"]).map(PathBuf::from);
    if top.as_deref().and_then(|p| p.canonicalize().ok()) != root.canonicalize().ok() {
        return (None, None, vec![]);
    }
    let revision = git(root, &["rev-parse", "--verify", "HEAD"]);
    let mut args = vec!["status", "--porcelain=v1", "--untracked-files=all", "--"];
    args.extend_from_slice(inputs);
    let dirty = git(root, &args).map(|status| !status.is_empty());
    let mut watched = Vec::new();
    let mut refs = vec![
        "HEAD".to_owned(),
        "index".to_owned(),
        "packed-refs".to_owned(),
    ];
    if let Some(reference) = git(root, &["symbolic-ref", "-q", "HEAD"]) {
        refs.push(reference);
    }
    for reference in refs {
        if let Some(path) = git(
            root,
            &[
                "rev-parse",
                "--path-format=absolute",
                "--git-path",
                &reference,
            ],
        ) {
            // Watch the parent of an absent ref too (e.g. a packed branch ref becoming loose).
            let path = PathBuf::from(path);
            watched.push(if path.exists() {
                path
            } else {
                path.parent().unwrap().to_owned()
            });
        }
    }
    (revision, dirty, watched)
}

pub fn emit() -> io::Result<()> {
    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let candidate = manifest
        .parent()
        .and_then(Path::parent)
        .unwrap_or(&manifest);
    let workspace =
        candidate.join("Cargo.toml").is_file() && candidate.join("crates/pcp-core").is_dir();
    let root = if workspace { candidate } else { &manifest };
    let inputs = if workspace {
        INPUTS
    } else {
        &["Cargo.toml", "src", "build.rs", "build_support.rs"]
    };
    let (digest, mut watched) = source_digest(root, inputs)?;
    let (revision, dirty, git_watched) = git_identity(root, inputs);
    watched.extend(git_watched);
    for path in watched {
        println!("cargo:rerun-if-changed={}", path.display());
    }
    let generated = format!(
        "pub const SOURCE_DIGEST: &str = {digest:?};\npub const GIT_REVISION: Option<&str> = {revision:?};\npub const DIRTY: Option<bool> = {dirty:?};\npub const TARGET: &str = {:?};\n",
        env::var("TARGET").unwrap()
    );
    fs::write(
        PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("build_identity.rs"),
        generated,
    )
}
