//! What the Build workflow promises about the staging key and about
//! publishing (proposal 0013 §7 and §10.4 C, D-107), checked on its text so
//! a change that breaks a promise fails `cargo test` before it reaches a
//! runner. GitHub's own settings (the environment `staging`, its branch rule)
//! are the other half: docs/ci.md lists them.

const BUILD: &str = include_str!("../../.github/workflows/build.yml");
const CI: &str = include_str!("../../.github/workflows/ci.yml");
const OPS: &str = include_str!("../../.github/workflows/ops.yml");

/// Each job of a workflow as (its id, its lines), in order: the keys two
/// spaces in under `jobs:`.
fn jobs(workflow: &str) -> Vec<(&str, Vec<&str>)> {
    let mut jobs: Vec<(&str, Vec<&str>)> = Vec::new();
    let mut inside = false;
    for line in workflow.lines() {
        if !line.starts_with(' ') && !line.is_empty() && !line.starts_with('#') {
            inside = line == "jobs:";
            continue;
        }
        if !inside {
            continue;
        }
        let id = line
            .strip_prefix("  ")
            .filter(|rest| !rest.starts_with([' ', '#']))
            .and_then(|rest| rest.strip_suffix(':'));
        match (id, jobs.last_mut()) {
            (Some(id), _) => jobs.push((id, Vec::new())),
            (None, Some((_, lines))) => lines.push(line),
            (None, None) => {}
        }
    }
    jobs
}

fn job<'a>(workflow: &'a str, id: &str) -> Vec<&'a str> {
    jobs(workflow)
        .into_iter()
        .find(|(name, _)| *name == id)
        .map(|(_, lines)| lines)
        .unwrap_or_else(|| panic!("no job {id}"))
}

/// The lines of the workflow that aren't comments.
fn code(lines: &[&str]) -> String {
    lines
        .iter()
        .filter(|l| !l.trim_start().starts_with('#'))
        .map(|l| format!("{l}\n"))
        .collect()
}

#[test]
fn the_jobs_are_found() {
    let ids: Vec<&str> = jobs(BUILD).into_iter().map(|(id, _)| id).collect();
    for id in [
        "probe",
        "hub",
        "binaries",
        "systemd",
        "staging-assemble",
        "staging-sign",
        "attest",
        "deploy",
    ] {
        assert!(ids.contains(&id), "{id} not in {ids:?}");
    }
}

/// The staging key is a secret of one job, which compiles nothing.
#[test]
fn only_staging_sign_sees_the_staging_key() {
    for (id, lines) in jobs(BUILD) {
        let text = code(&lines);
        if id == "staging-sign" {
            assert_eq!(text.matches("secrets.").count(), 1, "one secret, once");
            assert!(text.contains("${{ secrets.BLENTINEL_STAGING_SIGNING_KEY }}"));
        } else {
            assert!(!text.contains("secrets."), "{id} reads a secret");
        }
    }
    for workflow in [CI, OPS] {
        assert!(!code(&workflow.lines().collect::<Vec<_>>()).contains("secrets."));
    }
}

#[test]
fn staging_sign_runs_only_for_main_in_its_environment() {
    let text = code(&job(BUILD, "staging-sign"));
    assert!(
        text.contains("    if: github.event_name == 'push' && github.ref == 'refs/heads/main'\n")
    );
    assert!(text.contains("    environment:\n      name: staging\n      deployment: false\n"));
    // Checked again inside the job, in case the condition above is edited.
    assert!(
        text.contains(r#"[ "$GITHUB_EVENT_NAME" = push ] && [ "$GITHUB_REF" = refs/heads/main ]"#)
    );
    // Every check of the run passed first.
    for need in [
        "binaries",
        "arm-tests",
        "systemd",
        "staging-assemble",
        "signing-scripts",
    ] {
        assert!(text.contains(need), "staging-sign doesn't wait for {need}");
    }
}

/// No code of the build runs where the key is: no toolchain, no cargo, no
/// checkout beyond the scripts.
#[test]
fn staging_sign_builds_nothing() {
    let text = code(&job(BUILD, "staging-sign"));
    for word in [
        "cargo",
        "rustup",
        "rust-cache",
        "install-action",
        "npm",
        "zig",
    ] {
        assert!(!text.contains(word), "staging-sign uses {word}");
    }
    assert!(text.contains("sparse-checkout: .github/scripts\n"));
    assert!(
        !text.contains("permissions:"),
        "it keeps the workflow's read-only token"
    );
}

/// An internal build is never a release: nothing tags, publishes or writes
/// to the repository (D-106), and no tag starts a run.
#[test]
fn nothing_is_published() {
    for workflow in [BUILD, CI, OPS] {
        let text = code(&workflow.lines().collect::<Vec<_>>());
        for word in [
            "contents: write",
            "gh release",
            "git tag",
            "git push",
            "action-gh-release",
            "pull_request_target",
            "tags:",
            "self-hosted",
        ] {
            assert!(!text.contains(word), "a workflow has {word}");
        }
    }
}

/// Only the attestation job may mint an identity token, and it has no key.
#[test]
fn only_attest_mints_an_identity() {
    for (id, lines) in jobs(BUILD) {
        let text = code(&lines);
        assert_eq!(text.contains("id-token: write"), id == "attest", "{id}");
    }
    let attest = code(&job(BUILD, "attest"));
    assert!(attest.contains("needs: staging-sign\n"));
    assert!(
        !attest.contains("checkout"),
        "it runs no code of the repository"
    );
}

/// Every action is pinned to a full commit SHA, its version in a comment.
#[test]
fn actions_are_pinned_to_commits() {
    for workflow in [BUILD, CI, OPS] {
        for line in workflow.lines().filter(|l| {
            l.trim_start().starts_with("- uses:") || l.trim_start().starts_with("uses:")
        }) {
            let reference = line.split("uses:").nth(1).unwrap().trim();
            let (action, rest) = reference
                .split_once('@')
                .unwrap_or_else(|| panic!("{line}"));
            let (sha, comment) = rest
                .split_once(" # ")
                .unwrap_or_else(|| panic!("no version comment: {line}"));
            assert!(
                sha.len() == 40 && sha.bytes().all(|b| b.is_ascii_hexdigit()),
                "{action} isn't pinned to a commit: {line}"
            );
            assert!(comment.starts_with('v'), "{line}");
        }
    }
}
