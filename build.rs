// Build-time info: git commit and date, embedded for the !status output and the startup
// announcement. No external crate: falls back to "?" if git or date isn't available rather
// than failing the build.
use std::process::Command;

/// Runs an external program and returns its trimmed stdout.
/// Input: `program` — executable name (`"git"`, `"date"`); `args` — its CLI arguments.
/// Output: `Some(String)` on a successful run with non-empty output, `None` if the program
/// is missing, exits non-zero, or prints nothing. Uses: `std::process::Command`.
/// Used by: `main`, for every `git`/`date` call below.
fn command(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!s.is_empty()).then_some(s)
}

/// Cargo build-script entry point. Input: none (reads the working tree via `git`/`date`).
/// Output: none directly — instead prints `cargo:` directives that Cargo turns into
/// compile-time `env!("VERSION_COMMIT")` / `env!("VERSION_DATE")` values (consumed by
/// `src/bot/types_settings.rs`'s `VERSION_COMMIT`/`VERSION_DATE` constants) and rebuild
/// triggers. Uses: `command`.
fn main() {
    println!("cargo:rerun-if-env-changed=BUILD_COMMIT");
    let supplied_commit = std::env::var("BUILD_COMMIT")
        .ok()
        .filter(|value| !value.is_empty());
    let commit = if let Some(value) = supplied_commit.as_ref() {
        value.chars().take(7).collect::<String>()
    } else {
        let mut commit =
            command("git", &["rev-parse", "--short", "HEAD"]).unwrap_or_else(|| "?".into());
        // "+" marks uncommitted changes in builds using the working tree.
        if command("git", &["status", "--porcelain"]).is_some() {
            commit.push('+');
        }
        commit
    };
    let date = command("date", &["+%Y-%m-%d"]).unwrap_or_else(|| "?".into());
    println!("cargo:rustc-env=VERSION_COMMIT={commit}");
    println!("cargo:rustc-env=VERSION_DATE={date}");
    // rebuild when the commit changes (HEAD itself and whichever branch is currently checked out)
    println!("cargo:rerun-if-changed=.git/HEAD");
    if supplied_commit.is_none() {
        if let Some(branch) = command("git", &["symbolic-ref", "-q", "HEAD"]) {
            println!("cargo:rerun-if-changed=.git/{branch}");
        }
    }
}
