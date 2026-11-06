//! Contracts for documented shell commands.
//!
//! A shell fence cannot run as a doctest, so these tests check each documented
//! command against the manifest, and run the example pair the README promises
//! the way a reader would: the server first, then the client against it.

use std::{
    net::{SocketAddr, TcpStream},
    path::PathBuf,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use super::{documentation_examples::load_documented_examples, parsed_manifest};

const ECHO_ADDRESS: &str = "127.0.0.1:7878";

/// Setup commands that have nothing in this repository to check them against,
/// other than the clone URL, which must name this project.
const RECOGNIZED_COMMANDS: &[&str] = &[
    "cargo new hello-wireframe",
    "cd hello-wireframe",
    "cargo run",
    "git clone https://github.com/leynos/wireframe.git",
    "cd wireframe",
];

/// Kills the wrapped child on drop, so a failed assertion cannot leak a server
/// that would hold the port for later runs.
struct RunningChild(Child);

impl Drop for RunningChild {
    fn drop(&mut self) {
        // The child may already have exited, and either way cleanup is done.
        drop(self.0.kill());
        drop(self.0.wait());
    }
}

/// Directory holding Cargo's final build outputs.
///
/// Test executables may sit in a separate build directory, and coverage tools
/// override the target directory, so only Cargo can say where this is.
fn target_directory() -> PathBuf {
    let output = Command::new(env!("CARGO"))
        .args(["metadata", "--format-version=1", "--no-deps"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("cargo metadata should run");
    assert!(output.status.success(), "cargo metadata should succeed");
    let metadata: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("cargo metadata should print JSON");
    metadata
        .get("target_directory")
        .and_then(serde_json::Value::as_str)
        .map(PathBuf::from)
        .expect("cargo metadata should report the target directory")
}

/// Path of a built example binary; `cargo test` builds examples by default.
fn built_example(name: &str) -> PathBuf {
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    let path = target_directory().join(profile).join("examples").join(name);
    assert!(
        path.is_file(),
        "{} is missing; build examples with `--all-features` first",
        path.display()
    );
    path
}

/// Start a built example with a cleared environment.
fn spawn_example(name: &str) -> Command {
    let mut command = Command::new(built_example(name));
    command
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

fn wait_until_listening(address: SocketAddr, server: &mut Child) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        if let Some(status) = server.try_wait().expect("server status is readable") {
            panic!("echo server exited early with {status}");
        }
        if TcpStream::connect(address).is_ok() {
            return;
        }
        thread::sleep(Duration::from_millis(50));
    }
    panic!("echo server did not listen on {address} within 30 seconds");
}

/// Check one documented `cargo run --example` against the manifest.
fn assert_example_command_matches_manifest(id: &str, line: &str, manifest: &toml::Table) {
    let Some(name) = line
        .strip_prefix("cargo run --example ")
        .and_then(|rest| rest.strip_suffix(" --features examples"))
    else {
        panic!("{id} should use `--features examples`: {line}");
    };
    let Some(declared) = manifest
        .get("example")
        .and_then(toml::Value::as_array)
        .expect("Cargo.toml should declare examples")
        .iter()
        .find(|entry| entry.get("name").and_then(toml::Value::as_str) == Some(name))
    else {
        panic!("{id} names undeclared example '{name}'");
    };
    let required: Vec<_> = declared
        .get("required-features")
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(toml::Value::as_str)
        .collect();
    assert_eq!(
        required,
        ["examples"],
        "{id}: --features should match the manifest"
    );
}

/// Every documented command is one this suite understands.
///
/// An unrecognized command fails here, so adding one forces a decision about
/// how it is validated rather than letting it go unchecked.
#[test]
fn documented_commands_are_recognized_and_match_the_repository() {
    let manifest = parsed_manifest();
    let examples = load_documented_examples().expect("documents should load");
    for example in examples
        .iter()
        .filter(|example| example.language() == "bash")
    {
        for line in example.body.lines() {
            if line.starts_with("cargo run --example") {
                assert_example_command_matches_manifest(&example.id, line, &manifest);
            } else {
                assert!(
                    RECOGNIZED_COMMANDS.contains(&line),
                    "{} uses unrecognized command `{line}`",
                    example.id
                );
            }
        }
    }
}

/// The README's two-terminal walkthrough should succeed end to end.
#[test]
fn the_documented_login_succeeds_against_the_documented_echo_server() {
    let address: SocketAddr = ECHO_ADDRESS.parse().expect("the address is valid");
    let mut server = RunningChild(
        spawn_example("echo")
            .spawn()
            .expect("the echo example should start"),
    );
    wait_until_listening(address, &mut server.0);

    let client = spawn_example("client_echo_login")
        .output()
        .expect("the client example should run");
    assert!(
        client.status.success(),
        "client_echo_login failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&client.stdout),
        String::from_utf8_lossy(&client.stderr)
    );
}
