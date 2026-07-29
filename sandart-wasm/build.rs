// Stamps this crate with a short git SHA and a build-time timestamp, so the deployed page can
// show a build identifier (see `build_git_sha`/`build_timestamp_epoch` in src/lib.rs and the
// "Build" readout in web/index.html). The whole point is to let the user tell a fresh deploy
// apart from a stale cached bundle, so the values here must never go stale relative to what
// actually got compiled - see the rerun-if-changed reasoning below.
//
// Never fails the build: git may be missing (e.g. a tarball checkout with no .git), so every
// failure path falls back to an honest placeholder rather than a fabricated or frozen value.

use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn main() {
    // Rerun-if-changed reasoning:
    //
    // Cargo's default rebuild heuristic (rerun if any file under this crate's directory changes)
    // would never fire when only *other* crates change, or when someone commits without
    // touching sandart-wasm at all - which is the common case, since this crate is mostly glue.
    // A more targeted fix would be to watch git's own HEAD-tracking files (`.git/HEAD` for branch
    // switches, `.git/logs/HEAD` - the reflog - for commits/merges/rebases). We tried exactly
    // that with `git rev-parse --git-path`, and it surfaced a real failure mode worth recording:
    // in an environment where git isn't installed at all (this repo's own "sandart-dev" build
    // container, as it happens), that lookup fails, so *zero* watch paths get registered -
    // leaving nothing to invalidate the cached stamp if git ever became available later without
    // build.rs itself being touched.
    //
    // Rather than depend on git's internals being present and reachable just to know *when* to
    // rerun, we sidestep the question: force this script to run on every single build, via a
    // rerun-if-changed path that can never exist. Cargo treats a nonexistent watched path as
    // "always changed", so the stamp is recomputed on every `cargo build`/`wasm-pack build`
    // invocation - CI's fresh-checkout builds and local incremental builds alike. The cost is two
    // fast subprocess spawns (or two fast failures, if git is missing) per build: negligible next
    // to the linking step that follows. This is strictly stronger than the git-path approach and
    // removes the staleness question entirely, at the cost of doing a little redundant work on
    // rebuilds where nothing actually changed.
    println!("cargo:rerun-if-changed=SANDART_BUILD_RS_ALWAYS_RERUN_SENTINEL_DOES_NOT_EXIST");
    // Also watch the script itself, for the (redundant, but cheap and conventional) case where
    // rerun-if-changed semantics ever changed out from under the trick above.
    println!("cargo:rerun-if-changed=build.rs");

    // This does NOT distinguish a dirty working tree from a clean one (no commit = same SHA
    // either way). That's deliberate: we don't stamp a "-dirty" suffix, so there is nothing about
    // uncommitted state for the stamp to get wrong - it always honestly names the last commit
    // that was actually checked out, whatever else is sitting uncommitted on top of it.
    let sha = git_output(&["rev-parse", "--short=9", "HEAD"]).unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=SANDART_GIT_SHA={sha}");

    // Build wall-clock time, as seconds since the Unix epoch. Formatted for display in JS
    // (`new Date(epoch * 1000)`) rather than here, so this file stays free of date-formatting
    // logic and its attendant bugs. Deliberately NOT the git commit time: the point of this
    // second field is to disambiguate two builds of the *same* commit (e.g. a re-run deploy),
    // which a commit timestamp can't do.
    let epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    println!("cargo:rustc-env=SANDART_BUILD_EPOCH={epoch}");
}

/// Runs a git command and returns its trimmed stdout, or `None` on any failure (git missing,
/// not a repo, non-zero exit, non-UTF8 output). Callers are expected to fall back to an honest
/// placeholder rather than propagate the error - a missing build identifier is fine, a
/// misleading one is not.
fn git_output(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}
