// Copyright Nixort & Itan Winter <https://github.com/Nixort/Fjord> 2026.
//
// License: GNU General Public License v3
// You can find the license file in the project root.
//
// Fjord OS — version 0.0.2
// The code was written for Fjord.
// 23 june 2026

//! # Shipwright — host build orchestrator
//!
//! Runs on the developer host (not `no_std`). It drives Fjord developer tasks:
//! building the freestanding kernel ELF, checking host/bare-metal crates, and
//! booting the image in QEMU. Cask sealing arrives in later phases.
//!
//! This is the entry point behind `cargo shipwright -- <command>`.

use std::env;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

const TOOLCHAIN: &str = "+nightly-2026-06-01";
const TARGET_SPEC: &str = "boot/x86_64-fjord.json";
const TARGET_TRIPLE: &str = "x86_64-fjord";
const KERNEL_PACKAGE: &str = "boot";
const KERNEL_BIN: &str = "fjord-kernel";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("shipwright: {err}");
            ExitCode::from(1)
        }
    }
}

fn run() -> Result<(), String> {
    let mut args = env::args().skip(1).collect::<Vec<_>>();

    // Cargo aliases commonly insert a `--` separator before user arguments.
    // `cargo shipwright -- build` therefore reaches us as [`--`, `build`].
    if args.first().is_some_and(|arg| arg == "--") {
        args.remove(0);
    }

    let cmd = args.first().cloned().unwrap_or_else(|| "help".to_owned());
    let rest = if args.is_empty() { Vec::new() } else { args[1..].to_vec() };

    match cmd.as_str() {
        "build" => build_kernel(&rest),
        "check" => check_kernel(&rest),
        "qemu" => qemu(&rest),
        "test" => Err("no_std QEMU test harness is not implemented yet".to_owned()),
        "seal" => Err("Cask sealing is scheduled for ROADMAP Phase 2/3".to_owned()),
        "help" | "--help" | "-h" => {
            print_help();
            Ok(())
        }
        _ => {
            print_help();
            Err(format!("unknown command `{cmd}`"))
        }
    }
}

fn print_help() {
    eprintln!("Shipwright — Fjord build orchestrator");
    eprintln!("usage: cargo shipwright -- <build|check|test|seal|qemu> [--profile <dev|release>]");
    eprintln!();
    eprintln!("implemented now:");
    eprintln!("  build    build the x86_64 freestanding kernel ELF");
    eprintln!("  check    type-check the x86_64 freestanding kernel ELF");
    eprintln!("  qemu     build the ELF and boot it in qemu-system-x86_64 (serial on stdio)");
}

fn build_kernel(args: &[String]) -> Result<(), String> {
    let profile = profile_from_args(args)?;
    let root = workspace_root()?;

    println!("Shipwright: building Fjord kernel ELF ({profile})");
    cargo_kernel_command(&root, "build", &profile)?.run()
}

fn check_kernel(args: &[String]) -> Result<(), String> {
    let profile = profile_from_args(args)?;
    let root = workspace_root()?;

    println!("Shipwright: checking Fjord kernel ELF ({profile})");
    cargo_kernel_command(&root, "check", &profile)?.run()
}

fn qemu(args: &[String]) -> Result<(), String> {
    let profile = profile_from_args(args)?;
    let root = workspace_root()?;

    println!("Shipwright: preparing QEMU boot ({profile})");
    cargo_kernel_command(&root, "build", &profile)?.run()?;

    let kernel = kernel_path(&root, &profile);
    if !kernel.is_file() {
        return Err(format!("kernel ELF not found at {}", kernel.display()));
    }
    if !command_exists("qemu-system-x86_64") {
        return Err("qemu-system-x86_64 not found in PATH".to_owned());
    }

    println!("Shipwright: booting {} in QEMU", kernel.display());
    println!("Shipwright: serial routed to stdio; press Ctrl-C to stop");

    let mut command = Command::new("qemu-system-x86_64");
    command
        .current_dir(&root)
        .arg("-kernel")
        .arg(&kernel)
        .arg("-serial")
        .arg("stdio")
        .arg("-display")
        .arg("none")
        .arg("-no-reboot")
        .arg("-D")
        .arg("target/qemu-fjord.log")
        .arg("-d")
        .arg("guest_errors");

    Runner { command }.run()
}

fn profile_from_args(args: &[String]) -> Result<String, String> {
    let mut profile = "dev".to_owned();
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--profile" => {
                profile = iter
                    .next()
                    .ok_or_else(|| "--profile requires a value".to_owned())?
                    .to_owned();
            }
            "--release" => profile = "release".to_owned(),
            unknown => return Err(format!("unsupported argument `{unknown}`")),
        }
    }

    match profile.as_str() {
        "dev" | "release" => Ok(profile),
        other => Err(format!("unsupported profile `{other}` (expected dev|release)")),
    }
}

fn workspace_root() -> Result<PathBuf, String> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .ok_or_else(|| "failed to locate workspace root".to_owned())
}

fn cargo_kernel_command(root: &Path, verb: &str, profile: &str) -> Result<Runner, String> {
    let mut command = Command::new("cargo");
    command
        .current_dir(root)
        .arg(TOOLCHAIN)
        .arg(verb)
        .arg("-Zjson-target-spec")
        .arg("-p")
        .arg(KERNEL_PACKAGE)
        .arg("--target")
        .arg(TARGET_SPEC);

    if profile == "release" {
        command.arg("--release");
    }

    Ok(Runner { command })
}

fn kernel_path(root: &Path, profile: &str) -> PathBuf {
    let cargo_profile_dir = if profile == "release" { "release" } else { "debug" };
    root.join("target")
        .join(TARGET_TRIPLE)
        .join(cargo_profile_dir)
        .join(KERNEL_BIN)
}

fn command_exists(name: &str) -> bool {
    let Some(path) = env::var_os("PATH") else {
        return false;
    };

    env::split_paths(&path).any(|dir| {
        let candidate = dir.join(name);
        candidate.is_file() && is_executable(&candidate)
    })
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;

    path.metadata()
        .map(|metadata| metadata.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.exists()
}

struct Runner {
    command: Command,
}

impl Runner {
    fn run(&mut self) -> Result<(), String> {
        eprintln!("$ {}", render_command(&self.command));
        let status = self
            .command
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status()
            .map_err(|err| format!("failed to spawn `{}`: {err}", render_program(&self.command)))?;

        if status.success() {
            Ok(())
        } else {
            Err(format!("command exited with status {status}"))
        }
    }
}

fn render_program(command: &Command) -> String {
    command.get_program().to_string_lossy().into_owned()
}

fn render_command(command: &Command) -> String {
    let mut parts = vec![render_program(command)];
    parts.extend(command.get_args().map(render_arg));
    parts.join(" ")
}

fn render_arg(arg: &OsStr) -> String {
    let s = arg.to_string_lossy();
    if s.chars().all(|c| c.is_ascii_alphanumeric() || "-_=./+".contains(c)) {
        s.into_owned()
    } else {
        format!("'{s}'")
    }
}
