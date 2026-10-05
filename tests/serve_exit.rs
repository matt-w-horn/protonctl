//! RFC R10 against the built binary, with a throwaway home: the server's
//! download folder is deleted when it stops, whether the host closes stdin
//! (the MCP shutdown), sends SIGTERM, a terminal closes (SIGHUP), or a person
//! presses Ctrl-C; the server exits promptly each way; and no tool result
//! reaches stderr, which hosts keep in log files.

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::{Duration, Instant};

/// A server that has answered initialize, so it is past startup and serving.
#[expect(
    clippy::unwrap_used,
    reason = "a test helper: a panic is a failed test"
)]
fn start(
    home: &std::path::Path,
    env: &[(&str, &str)],
) -> (Child, ChildStdin, BufReader<ChildStdout>) {
    let config = home.join("config.toml");
    std::fs::write(
        &config,
        format!(
            "[mail]\naddress = \"t@example.test\"\ncert_sha256 = \"{}\"\n",
            "0".repeat(64)
        ),
    )
    .unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_protonctl"))
        .arg("serve")
        .env("HOME", home)
        // No D-Bus session, so a Linux run never reaches the user's keyring.
        .env("DBUS_SESSION_BUS_ADDRESS", "unix:path=/nonexistent")
        .env_remove("XDG_CACHE_HOME")
        .env("PROTONCTL_CONFIG", &config)
        .envs(env.iter().copied())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"protocolVersion":"2025-11-25","capabilities":{{}},"clientInfo":{{"name":"test","version":"0"}}}}}}"#
    )
    .unwrap();
    let mut line = String::new();
    stdout.read_line(&mut line).unwrap();
    assert!(line.contains(r#""id":1"#), "{line}");
    (child, stdin, stdout)
}

#[expect(
    clippy::unwrap_used,
    reason = "a test helper: a panic is a failed test"
)]
fn stop_and_check(signal: Option<&str>) {
    let home = tempfile::tempdir().unwrap();
    let (mut child, stdin, _stdout) = start(home.path(), &[]);
    // protonctl's cache folder under this home, on each system.
    let cache = if cfg!(target_os = "macos") {
        "Library/Caches/protonctl"
    } else {
        ".cache/protonctl"
    };
    let downloads = home
        .path()
        .join(format!("{cache}/downloads/{}", child.id()));
    std::fs::create_dir_all(&downloads).unwrap();
    std::fs::write(downloads.join("0-a.txt"), "a").unwrap();
    match signal {
        Some(signal) => {
            let pid = child.id().to_string();
            Command::new("/bin/kill")
                .args([signal, &pid])
                .status()
                .unwrap();
        }
        None => drop(stdin),
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() > deadline {
            child.kill().unwrap();
            panic!("the server did not exit within 10 s");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        !downloads.exists(),
        "the download folder outlived the server"
    );
}

#[test]
fn closing_stdin_deletes_the_download_folder() {
    stop_and_check(None);
}

#[test]
fn sigterm_deletes_the_download_folder() {
    stop_and_check(Some("-TERM"));
}

#[test]
fn ctrl_c_deletes_the_download_folder() {
    stop_and_check(Some("-INT"));
}

#[test]
fn sighup_deletes_the_download_folder() {
    stop_and_check(Some("-HUP"));
}

/// The Claude app starts a server and sometimes closes it again before
/// initializing; that is a stop like any other, not a failure.
#[test]
fn closing_stdin_before_initialize_is_a_clean_stop() {
    let home = tempfile::tempdir().unwrap();
    let config = home.path().join("config.toml");
    std::fs::write(&config, "").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_protonctl"))
        .arg("serve")
        .env("HOME", home.path())
        .env("DBUS_SESSION_BUS_ADDRESS", "unix:path=/nonexistent")
        .env("PROTONCTL_CONFIG", &config)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success() && stderr.is_empty(),
        "{}: {stderr}",
        out.status
    );
}

#[test]
fn results_stay_out_of_stderr_even_with_rust_log_set() {
    let home = tempfile::tempdir().unwrap();
    let (mut child, mut stdin, mut stdout) = start(home.path(), &[("RUST_LOG", "debug")]);
    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","method":"notifications/initialized"}}"#
    )
    .unwrap();
    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{{"name":"get_status","arguments":{{}}}}}}"#
    )
    .unwrap();
    let mut line = String::new();
    stdout.read_line(&mut line).unwrap();
    // A phrase from get_status's result, to look for in stderr: its status,
    // or, with no privacy mode chosen on this machine (RFC Q27), the refusal.
    let phrase = ["protonctl is read-only", "privacy_mode_"]
        .into_iter()
        .find(|p| line.contains(p))
        .unwrap_or_else(|| panic!("{line}"));
    drop(stdin);
    child.wait().unwrap();
    let mut stderr = String::new();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut stderr)
        .unwrap();
    assert!(!stderr.contains(phrase), "{stderr}");
}
