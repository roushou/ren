//! TUI preference commands run without a daemon or a terminal.
#![cfg(feature = "tui")]
#![allow(
    clippy::unwrap_used,
    reason = "integration tests are a shell: command helpers fail loudly"
)]

use std::process::Command;

#[test]
fn save_theme_is_local_atomic_and_available_without_a_daemon() {
    use std::os::unix::fs::PermissionsExt as _;
    let temp = tempfile::tempdir().unwrap();
    let config = temp.path().join("ren");
    std::fs::create_dir(&config).unwrap();
    let gateway = "gateway = 'http://127.0.0.1:1'\n";
    std::fs::write(config.join("client.toml"), gateway).unwrap();
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_ren"))
            .arg("tui")
            .args(args)
            .env("XDG_CONFIG_HOME", temp.path())
            .env("NO_COLOR", "1")
            .output()
            .unwrap()
    };
    for theme in ["dark", "light", "terminal"] {
        let output = run(&["--save-theme", theme]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            String::from_utf8_lossy(&output.stdout).contains(&format!("Saved TUI theme {theme}"))
        );
        assert_eq!(
            std::fs::read_to_string(config.join("tui.toml")).unwrap(),
            format!("theme = \"{theme}\"\n")
        );
        assert_eq!(
            std::fs::read_to_string(config.join("client.toml")).unwrap(),
            gateway
        );
        assert_eq!(
            std::fs::metadata(config.join("tui.toml"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    let before = std::fs::read(config.join("tui.toml")).unwrap();
    for args in [
        vec!["--save-theme", "unknown"],
        vec!["--theme", "unknown"],
        vec!["--theme", "dark", "--save-theme", "light"],
    ] {
        assert!(!run(&args).status.success());
        assert_eq!(std::fs::read(config.join("tui.toml")).unwrap(), before);
    }
    // A one-run override does not save, even if connection fails.
    assert!(
        !run(&["--theme", "dark", "--gateway", "http://127.0.0.1:1"])
            .status
            .success()
    );
    assert_eq!(std::fs::read(config.join("tui.toml")).unwrap(), before);
    let help = run(&["--help"]);
    assert!(help.status.success());
    let help = String::from_utf8_lossy(&help.stdout);
    assert!(help.contains("--theme") && help.contains("--save-theme"));
    assert!(help.contains("terminal, dark, light"));
}
