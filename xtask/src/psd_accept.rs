//! `cargo xtask psd-accept`: open our PSD exports in an independent reader.
//!
//! The reader is [psd-tools](https://github.com/psd-tools/psd-tools) (MIT), the reference
//! implementation of the format outside Adobe: it parses the whole layer tree, so it catches a
//! file that only PhotoCraft can read. The check itself lives in
//! `crates/io/tests/psd_acceptance.rs`; this command installs the reader and runs it.

use std::path::PathBuf;
use std::process::Command;

use crate::corpus_pins::PSD_TOOLS_PYTHON;
use crate::root;

/// The virtual environment with the third-party reader (`--setup` creates it).
fn venv_dir() -> PathBuf {
    root().join(".venv-psd")
}

/// The interpreter inside that environment.
fn venv_python() -> PathBuf {
    let dir = venv_dir();
    if cfg!(windows) { dir.join("Scripts").join("python.exe") } else { dir.join("bin").join("python") }
}

/// Creates the virtual environment and installs the pinned reader.
fn setup() -> Result<(), String> {
    let dir = venv_dir();
    let _ = std::fs::remove_dir_all(&dir);
    let mut c = Command::new("python3");
    c.args(["-m", "venv"]).arg(&dir).current_dir(root());
    let status = c.status().map_err(|e| format!("python3 -m venv: {e}"))?;
    if !status.success() {
        return Err("python3 -m venv failed".into());
    }
    let pip = if cfg!(windows) { dir.join("Scripts").join("pip.exe") } else { dir.join("bin").join("pip") };
    let mut c = Command::new(pip);
    c.args(["install", "--disable-pip-version-check", &format!("psd-tools=={PSD_TOOLS_PYTHON}")]);
    let status = c.status().map_err(|e| format!("pip install psd-tools: {e}"))?;
    if !status.success() {
        return Err("pip install psd-tools failed".into());
    }
    println!("the third-party reader is ready: {}", venv_python().display());
    Ok(())
}

/// Runs the acceptance test with the reader.
pub fn run_cmd(args: &[&str]) -> Result<(), String> {
    let (ours, passthrough) = match args.iter().position(|a| *a == "--") {
        Some(i) => (&args[..i], &args[i + 1..]),
        None => (args, &[][..]),
    };
    for a in ours {
        if *a != "--setup" && *a != "--setup-only" {
            return Err(format!("psd-accept: unknown argument `{a}`"));
        }
    }
    if ours.contains(&"--setup") || ours.contains(&"--setup-only") {
        setup()?;
    }
    if ours.contains(&"--setup-only") {
        return Ok(());
    }
    if !venv_python().is_file() {
        println!("no third-party reader yet: run `cargo xtask psd-accept --setup` (the test then skips itself)");
    }
    let mut c = crate::cargo();
    c.args(["test", "-p", "photocraft-io", "--features", "corpus", "--test", "psd_acceptance", "--", "--nocapture"]);
    c.args(passthrough);
    crate::run(c, "cargo test -p photocraft-io --features corpus --test psd_acceptance")
}
