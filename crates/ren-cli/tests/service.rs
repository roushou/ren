#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::disallowed_methods,
    reason = "integration tests are a shell: helpers outside #[test] fail loudly, and mock servers are spawned tasks"
)]
use std::process::Command;

#[test]
fn rendering_resolves_paths_without_creating_state_and_publishes_once() {
    let temp = tempfile::tempdir().unwrap();
    let config = temp.path().join("config & 'settings'.toml");
    std::fs::write(&config, "data_dir = 'state & $USER %h'\n").unwrap();
    let data = temp.path().join("state & $USER %h");
    for manager in ["launchd", "systemd"] {
        let output = temp.path().join(format!("ren.{manager}"));
        let args = [
            "--config",
            config.to_str().unwrap(),
            "service",
            "render",
            "--manager",
            manager,
            "--binary",
            env!("CARGO_BIN_EXE_ren"),
            "--output",
            output.to_str().unwrap(),
        ];
        let result = Command::new(env!("CARGO_BIN_EXE_ren"))
            .args(args)
            .current_dir(temp.path())
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(result.stdout.is_empty());
        assert!(!data.exists());
        let first = std::fs::read(&output).unwrap();
        let second = Command::new(env!("CARGO_BIN_EXE_ren"))
            .args(args)
            .output()
            .unwrap();
        assert!(!second.status.success());
        assert_eq!(std::fs::read(&output).unwrap(), first);
        #[cfg(target_os = "macos")]
        if manager == "launchd" {
            let parsed = Command::new("/usr/bin/plutil")
                .args([
                    "-convert",
                    "json",
                    "-o",
                    "-",
                    "--",
                    output.to_str().unwrap(),
                ])
                .output()
                .unwrap();
            assert!(parsed.status.success());
            let plist: serde_json::Value = serde_json::from_slice(&parsed.stdout).unwrap();
            assert_eq!(
                plist["ProgramArguments"],
                serde_json::json!([
                    env!("CARGO_BIN_EXE_ren"),
                    "--config",
                    config.to_str().unwrap(),
                    "--data-dir",
                    data.to_str().unwrap(),
                    "serve"
                ])
            );
            assert_eq!(
                plist["StandardErrorPath"],
                data.join("logs/daemon.log").to_str().unwrap()
            );
            assert_eq!(plist["ExitTimeOut"], 30);
            assert_eq!(plist["KeepAlive"]["SuccessfulExit"], false);
        }
    }
    assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 3);
}
