use std::path::PathBuf;
use std::process::Command;

fn main() {
    let git_hash = git(&["rev-parse", "--short", "HEAD"]).unwrap_or_else(|| "unknown".to_string());

    println!("cargo:rustc-env=GIT_HASH={git_hash}");

    for path in watched_paths() {
        println!("cargo:rerun-if-changed={}", path.display());
    }
}

/// Run a git command, returning its trimmed stdout if it succeeded.
fn git(args: &[&str]) -> Option<String> {
    Command::new("git")
        .args(args)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|stdout| stdout.trim().to_string())
        .filter(|stdout| !stdout.is_empty())
}

/// The git files whose contents determine `HEAD`'s hash.
///
/// Watching `.git/HEAD` alone is not enough: on a branch it holds a symbolic
/// ref (`ref: refs/heads/main`) that does *not* change when you commit — the
/// branch's ref file does. Missing that meant the build script kept a stale
/// `GIT_HASH` baked in across commits while the crate itself recompiled.
///
/// Paths come from `git rev-parse --git-path` so this works from a linked
/// worktree, where `.git` is a file and the real ref store lives elsewhere.
/// Only existing paths are emitted — Cargo re-runs the build script
/// unconditionally for a `rerun-if-changed` path that does not exist.
fn watched_paths() -> Vec<PathBuf> {
    let git_path = |name: &str| git(&["rev-parse", "--git-path", name]).map(PathBuf::from);

    // `HEAD` itself (changes on checkout), the ref it points at (changes on
    // commit), and `packed-refs` (holds the ref when it has been packed).
    let head_ref = git(&["symbolic-ref", "--quiet", "HEAD"]);
    let names = ["HEAD".to_string(), "packed-refs".to_string()]
        .into_iter()
        .chain(head_ref);

    names
        .filter_map(|name| git_path(&name))
        .filter(|path| path.exists())
        .collect()
}
