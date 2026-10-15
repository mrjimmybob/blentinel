//! `blentinelmake ci`: the CI gate, run locally exactly as GitHub Actions
//! runs it (proposal 0013 §8, D-102).
//!
//! Each step's command is defined once: the cargo aliases in
//! `.cargo/config.toml`, `deny.toml` and the browser tests' `package.json`.
//! [`STEPS`] only lists them in order; `.github/workflows/ci.yml` runs the
//! same commands, one job per step, and a test below fails if the two lists
//! drift apart.

use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

/// One step of the gate.
pub struct Step {
    /// What `blentinelmake ci <name>` and the CI job call it.
    pub name: &'static str,
    /// The commands, run in `dir`, each as program and arguments.
    pub commands: &'static [&'static [&'static str]],
    /// Where the commands run, from the workspace root.
    pub dir: &'static str,
    /// A command that must work first, and what to do if it doesn't.
    pub needs: Option<(&'static [&'static str], &'static str)>,
}

/// The gate, in the order a failure is cheapest to find.
pub const STEPS: &[Step] = &[
    Step {
        name: "fmt",
        commands: &[&["cargo", "fmt", "--all", "--check"]],
        dir: ".",
        needs: None,
    },
    Step {
        name: "lint",
        commands: &[&["cargo", "lint"], &["cargo", "lint-wasm"]],
        dir: ".",
        needs: None,
    },
    Step {
        name: "test",
        commands: &[&["cargo", "test-all"]],
        dir: ".",
        needs: None,
    },
    Step {
        name: "deny",
        commands: &[&["cargo", "deny", "check"]],
        dir: ".",
        needs: Some((
            &["cargo", "deny", "--version"],
            "cargo install cargo-deny --locked",
        )),
    },
    Step {
        name: "browser",
        commands: &[&["npm", "ci"], &["npm", "run", "build"], &["npm", "test"]],
        dir: "hub/end2end",
        needs: Some((
            &["cargo", "leptos", "--version"],
            "cargo install cargo-leptos --locked (and Google Chrome, see CONTRIBUTING.md)",
        )),
    },
];

/// The steps `names` asks for, in [`STEPS`] order; all of them when empty.
pub fn select(names: &[String]) -> Result<Vec<&'static Step>, String> {
    if let Some(unknown) = names
        .iter()
        .find(|n| !STEPS.iter().any(|s| s.name == n.as_str()))
    {
        let known: Vec<&str> = STEPS.iter().map(|s| s.name).collect();
        return Err(format!(
            "unknown step '{unknown}' (steps: {})",
            known.join(", ")
        ));
    }
    Ok(STEPS
        .iter()
        .filter(|s| names.is_empty() || names.iter().any(|n| n == s.name))
        .collect())
}

/// Runs the steps `names` asks for, stopping at the first that fails.
pub fn run(names: &[String]) -> Result<(), String> {
    if !Path::new(".cargo/config.toml").is_file() {
        return Err("run blentinelmake ci from the workspace root".into());
    }
    let steps = select(names)?;
    let mut times = Vec::new();
    for step in &steps {
        println!("\n== ci: {}", step.name);
        let started = Instant::now();
        if let Some((check, install)) = step.needs
            && !succeeds(check, step.dir)
        {
            return Err(format!(
                "step '{}' needs `{}`: {install}",
                step.name,
                check.join(" ")
            ));
        }
        for command in step.commands {
            if !succeeds(command, step.dir) {
                summary(&times);
                return Err(format!(
                    "step '{}' failed: `{}`",
                    step.name,
                    command.join(" ")
                ));
            }
        }
        times.push((step.name, started.elapsed()));
    }
    summary(&times);
    println!("ci: all {} step(s) passed", steps.len());
    Ok(())
}

fn succeeds(command: &[&str], dir: &str) -> bool {
    // `npm` is `npm.cmd` on Windows, which only a shell finds.
    let program = if cfg!(windows) && command[0] == "npm" {
        "npm.cmd"
    } else {
        command[0]
    };
    Command::new(program)
        .args(&command[1..])
        .current_dir(dir)
        .status()
        .is_ok_and(|s| s.success())
}

fn summary(times: &[(&str, Duration)]) {
    for (name, took) in times {
        println!("ci: {name:<8} passed in {} s", took.as_secs());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_are_chosen_by_name_in_gate_order() {
        let all = select(&[]).unwrap();
        assert_eq!(all.len(), STEPS.len());
        let some = select(&["deny".into(), "fmt".into()]).unwrap();
        let names: Vec<&str> = some.iter().map(|s| s.name).collect();
        assert_eq!(names, ["fmt", "deny"]);
        let err = select(&["fmt".into(), "tests".into()]).err().unwrap();
        assert!(err.contains("unknown step 'tests'"), "{err}");
    }

    /// The workflow runs every command of every step, so `blentinelmake ci`
    /// passing means CI will too (and the other way round).
    #[test]
    fn the_workflow_runs_every_step() {
        let workflow = include_str!("../../.github/workflows/ci.yml");
        for step in STEPS {
            for command in step.commands {
                let line = command.join(" ");
                assert!(
                    workflow.contains(&format!("run: {line}")),
                    "ci.yml doesn't run `{line}` (step '{}')",
                    step.name
                );
            }
        }
    }
}
