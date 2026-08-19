use std::collections::BTreeMap;
use std::io::{self, Write};
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};
use clap::Args;
use serde::Deserialize;

use crate::paths::repository_root;
use crate::process::{command_output, run_command};
use crate::version::{ReleaseVersion, verify_release_tag, write_project_versions};

const DEFAULT_BRANCH: &str = "master";
const ORIGIN: &str = "origin";
const VERSION_FILES: &[&str] = &[
    "Cargo.toml",
    "Cargo.lock",
    "bindgen/wasm/Cargo.toml",
    "bindgen/wasm/Cargo.lock",
    "web/package.json",
];
const REQUIRED_CHECKS: &[&str] = &[
    "Format, check, and clippy",
    "Tests",
    "Swift bindings and example",
    "Android bindings and example",
    "WASM bindings and web packages",
    "C++ bindings and Qt example",
];

/// Selects the version and confirmation behavior for a release.
#[derive(Args)]
pub(crate) struct ReleaseArgs {
    /// Semantic version to assign to every public package.
    #[arg(long, value_name = "VERSION")]
    version: String,
    /// Confirms the atomic push without reading standard input.
    #[arg(long)]
    yes: bool,
}

/// Prepares, commits, tags, and pushes one Wallet Engine release.
pub(crate) fn run_release(args: &ReleaseArgs) -> Result<()> {
    let root = repository_root()?;
    let release = ReleaseVersion::parse(&args.version)?;

    check_release_preconditions(&root, &release)?;
    write_project_versions(&root, &release.version)?;
    update_lockfiles(&root)?;
    let _verified = verify_release_tag(&root, &release.tag)?;
    show_release_diff(&root)?;
    confirm_push(&release, args.yes)?;
    create_release_commit_and_tag(&root, &release)?;
    push_release(&root, &release)?;

    println!(
        "Release tag `{}` was pushed. GitHub Actions will publish its artifacts.",
        release.tag
    );
    Ok(())
}

/// Checks repository, remote, changelog, tag, and CI state before changing versions.
fn check_release_preconditions(root: &Path, release: &ReleaseVersion) -> Result<()> {
    require_clean_worktree(root)?;
    require_branch(root, DEFAULT_BRANCH)?;
    run_command(Command::new("git").current_dir(root).args([
        "fetch",
        ORIGIN,
        DEFAULT_BRANCH,
        "--tags",
    ]))?;
    require_local_matches_remote(root)?;
    require_tag_absent(root, &release.tag)?;
    let _notes = crate::version::release_notes(root, &release.version)?;
    require_successful_checks(root)
}

/// Rejects releases from a worktree with tracked or untracked changes.
fn require_clean_worktree(root: &Path) -> Result<()> {
    let status = git_output(root, &["status", "--porcelain"])?;
    if status.trim().is_empty() {
        Ok(())
    } else {
        bail!("release requires a clean worktree:\n{status}")
    }
}

/// Requires the release to start from the configured default branch.
fn require_branch(root: &Path, expected: &str) -> Result<()> {
    let actual = git_output(root, &["branch", "--show-current"])?;
    if actual.trim() == expected {
        Ok(())
    } else {
        bail!(
            "release must run from branch `{expected}`, current branch is `{}`",
            actual.trim()
        )
    }
}

/// Requires local master to point to the fetched remote master commit.
fn require_local_matches_remote(root: &Path) -> Result<()> {
    let local = git_output(root, &["rev-parse", "HEAD"])?;
    let remote_ref = format!("{ORIGIN}/{DEFAULT_BRANCH}");
    let remote = git_output(root, &["rev-parse", &remote_ref])?;
    if local.trim() == remote.trim() {
        Ok(())
    } else {
        bail!("local `{DEFAULT_BRANCH}` does not match `{remote_ref}`")
    }
}

/// Rejects a version whose canonical tag already exists locally or remotely.
fn require_tag_absent(root: &Path, tag: &str) -> Result<()> {
    let local = git_output(root, &["tag", "--list", tag])?;
    if !local.trim().is_empty() {
        bail!("local release tag `{tag}` already exists")
    }
    let remote = git_output(root, &["ls-remote", "--tags", ORIGIN, tag])?;
    if !remote.trim().is_empty() {
        bail!("remote release tag `{tag}` already exists")
    }
    Ok(())
}

/// Requires all platform checks for the current commit to be complete and successful.
fn require_successful_checks(root: &Path) -> Result<()> {
    let repository = command_output(Command::new("gh").current_dir(root).args([
        "repo",
        "view",
        "--json",
        "nameWithOwner",
        "--jq",
        ".nameWithOwner",
    ]))?;
    let commit = git_output(root, &["rev-parse", "HEAD"])?;
    let endpoint = format!(
        "repos/{}/commits/{}/check-runs?filter=latest&per_page=100",
        repository.trim(),
        commit.trim()
    );
    let response = command_output(Command::new("gh").current_dir(root).args([
        "api",
        "-H",
        "Accept: application/vnd.github+json",
        &endpoint,
    ]))?;
    let checks: CheckRuns =
        serde_json::from_str(&response).context("failed to parse check runs")?;
    let by_name = checks
        .check_runs
        .into_iter()
        .map(|check| (check.name.clone(), check))
        .collect::<BTreeMap<_, _>>();

    for required in REQUIRED_CHECKS {
        let check = by_name
            .get(*required)
            .with_context(|| format!("required GitHub check `{required}` was not found"))?;
        if check.status != "completed" || check.conclusion.as_deref() != Some("success") {
            bail!(
                "required GitHub check `{required}` is `{}` with conclusion `{}`",
                check.status,
                check.conclusion.as_deref().unwrap_or("none")
            );
        }
    }
    Ok(())
}

/// Refreshes lockfiles after public package versions change.
fn update_lockfiles(root: &Path) -> Result<()> {
    run_command(
        Command::new("cargo")
            .current_dir(root)
            .args(["update", "--workspace"]),
    )?;
    run_command(Command::new("cargo").current_dir(root).args([
        "update",
        "--manifest-path",
        "bindgen/wasm/Cargo.toml",
        "--workspace",
    ]))
}

/// Displays the version-only changes that the release commit will contain.
fn show_release_diff(root: &Path) -> Result<()> {
    let mut arguments = vec!["diff", "--"];
    arguments.extend_from_slice(VERSION_FILES);
    let diff = git_output(root, &arguments)?;
    println!("Release changes:\n{diff}");
    Ok(())
}

/// Requires an explicit confirmation before creating and pushing Git objects.
fn confirm_push(release: &ReleaseVersion, confirmed: bool) -> Result<()> {
    if confirmed {
        return Ok(());
    }
    print!(
        "Type `yes` to commit, tag, and push `{}` to `{ORIGIN}`: ",
        release.tag
    );
    io::stdout()
        .flush()
        .context("failed to flush confirmation prompt")?;
    let mut answer = String::new();
    io::stdin()
        .read_line(&mut answer)
        .context("failed to read release confirmation")?;
    if answer.trim() == "yes" {
        Ok(())
    } else {
        bail!("release push aborted: expected `yes`")
    }
}

/// Creates the version commit and an annotated release tag.
fn create_release_commit_and_tag(root: &Path, release: &ReleaseVersion) -> Result<()> {
    let mut add = Command::new("git");
    add.current_dir(root).arg("add").arg("--");
    add.args(VERSION_FILES);
    run_command(&mut add)?;
    run_command(Command::new("git").current_dir(root).args([
        "commit",
        "--allow-empty",
        "-m",
        &format!("chore: release {}", release.tag),
    ]))?;
    run_command(Command::new("git").current_dir(root).args([
        "tag",
        "-a",
        &release.tag,
        "-m",
        &format!("Wallet Engine {}", release.version),
    ]))
}

/// Atomically pushes the release commit and tag to origin.
fn push_release(root: &Path, release: &ReleaseVersion) -> Result<()> {
    run_command(Command::new("git").current_dir(root).args([
        "push",
        "--atomic",
        ORIGIN,
        &format!("HEAD:{DEFAULT_BRANCH}"),
        &release.tag,
    ]))
}

/// Runs Git in the repository and returns trimmed output to release checks.
fn git_output(root: &Path, args: &[&str]) -> Result<String> {
    command_output(Command::new("git").current_dir(root).args(args))
}

/// Contains the check runs returned by GitHub for one commit.
#[derive(Deserialize)]
struct CheckRuns {
    check_runs: Vec<CheckRun>,
}

/// Contains the release-relevant state of one GitHub check run.
#[derive(Deserialize)]
struct CheckRun {
    name: String,
    status: String,
    conclusion: Option<String>,
}
