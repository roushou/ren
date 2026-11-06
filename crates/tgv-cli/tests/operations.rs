#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::disallowed_methods,
    reason = "integration tests are a shell: helpers outside #[test] fail loudly, and mock servers are spawned tasks"
)]
use axum::{Router, body::Body, response::Response, routing::get};
use std::{
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
use tgv_cli::{
    Cli,
    client::{GatewayChoice, GatewaySource},
    doctor::{Report, Status},
};
use tgv_daemon::{Daemon, config::Config};
use tgv_gateway::build::{Revision, Tree};
use tgv_harness::{endpoint::GatewayEndpoint, host::HostEnv};

const HEALTH: &str = r#"{"api_version":1,"status":"ok","recorder":{"storage_errors":0,"retention_errors":0,"omissions":{"policy_limit_bytes":0,"queue_bytes_bytes":0,"queue_slots_bytes":0,"storage_failure_bytes":0,"unrecorded_calls":0}}}"#;
const STORAGE: &str = r#"{"api_version":1,"policy":{"enabled":true,"max_age_seconds":600,"max_calls":10,"max_storage_bytes":1048576},"usage":{"calls":0,"open_calls":0,"disk":{"metadata":{"page_size":4096,"page_count":10,"freelist_count":0},"blobs":{"page_size":16384,"page_count":2,"freelist_count":0}}}}"#;

async fn mock(app: Router) -> (GatewayEndpoint, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint =
        GatewayEndpoint::parse(&format!("http://{}", listener.local_addr().unwrap())).unwrap();
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (endpoint, task)
}

/// A gateway chosen with --gateway.
fn flag(gateway: &str) -> GatewayChoice {
    GatewayChoice::Selected {
        endpoint: GatewayEndpoint::parse(gateway).unwrap(),
        source: GatewaySource::Flag,
    }
}

#[tokio::test]
async fn explicit_gateway_and_dry_run_ignore_unused_local_upstreams() {
    let (endpoint, task) = mock(
        Router::new()
            .route("/health", get(|| async { HEALTH }))
            .route("/api/storage", get(|| async { STORAGE })),
    )
    .await;
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("tgv.toml");
    std::fs::write(
        &config,
        "codex_base_url = 'https://user:SECRET@example.com'\n",
    )
    .unwrap();
    let gateway = endpoint.url("/").to_string();
    let report = Report::local(&HostEnv::default(), Some(&config), None, &flag(&gateway)).await;
    assert!(report.success());
    assert!(
        report
            .checks
            .iter()
            .any(|c| c.name == "upstream config" && c.status == Status::Skipped)
    );
    assert!(!serde_json::to_string(&report).unwrap().contains("SECRET"));
    let data = dir.path().join("must-not-exist");
    let output = Command::new(env!("CARGO_BIN_EXE_tgv"))
        .args([
            "--config",
            config.to_str().unwrap(),
            "--data-dir",
            data.to_str().unwrap(),
            "run",
            "codex",
            "--dry-run",
            "--gateway",
            &gateway,
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(!data.exists());
    assert!(!String::from_utf8_lossy(&output.stdout).contains("SECRET"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("SECRET"));
    // Serving does consume the local settings and must reject them before storage opens.
    let output = Command::new(env!("CARGO_BIN_EXE_tgv"))
        .args([
            "--config",
            config.to_str().unwrap(),
            "--data-dir",
            data.to_str().unwrap(),
            "serve",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(!data.exists());
    assert!(!String::from_utf8_lossy(&output.stderr).contains("SECRET"));
    task.abort();
}

#[tokio::test]
async fn diagnostic_errors_are_bounded_redacted_and_never_follow_redirects() {
    let followed = Arc::new(AtomicUsize::new(0));
    let count = followed.clone();
    let app = Router::new()
        .route(
            "/health",
            get(|| async {
                Response::builder()
                    .status(302)
                    .header("location", "/SECRET")
                    .body(Body::from("SECRET"))
                    .unwrap()
            }),
        )
        .route("/api/storage", get(|| async { "SECRET invalid body" }))
        .route(
            "/SECRET",
            get(move || async move {
                count.fetch_add(1, Ordering::SeqCst);
                HEALTH
            }),
        );
    let (endpoint, task) = mock(app).await;
    let report = Report::gateway(&endpoint).await;
    assert!(!report.success());
    assert_eq!(followed.load(Ordering::SeqCst), 0);
    assert!(!serde_json::to_string(&report).unwrap().contains("SECRET"));
    task.abort();

    let (endpoint, task) = mock(
        Router::new()
            .route("/health", get(|| async { "x".repeat(65537) }))
            .route("/api/storage", get(|| async { STORAGE })),
    )
    .await;
    let report = Report::gateway(&endpoint).await;
    assert!(report.checks.iter().any(|c| c.message.contains("64 KiB")));
    task.abort();
}

#[tokio::test]
async fn distinguishes_capture_loss_from_storage_failure() {
    for storage_errors in [0, 1] {
        let app = Router::new()
            .route(
                "/health",
                get(move || async move {
                    let mut value: serde_json::Value = serde_json::from_str(HEALTH).unwrap();
                    value["recorder"]["omissions"]["queue_bytes_bytes"] = 42.into();
                    value["recorder"]["storage_errors"] = storage_errors.into();
                    // Even a contradictory 200 must not hide the error counter.
                    axum::Json(value)
                }),
            )
            .route("/api/storage", get(|| async { STORAGE }));
        let (endpoint, task) = mock(app).await;
        let report = Report::gateway(&endpoint).await;
        assert_eq!(report.success(), storage_errors == 0);
        assert!(
            report
                .checks
                .iter()
                .any(|c| c.name == "capture" && c.status == Status::Warning)
        );
        task.abort();
    }
}

#[tokio::test]
async fn reports_whether_the_daemon_runs_this_build() {
    let ours = Cli::build();
    let commit = "0123456789abcdef0123456789abcdef01234567";
    let same = match ours.revision() {
        Revision::Commit {
            tree: Tree::Clean, ..
        } => Status::Ok,
        // A test binary built from a modified or untracked tree cannot be
        // confirmed to match anything.
        Revision::Commit {
            tree: Tree::Modified,
            ..
        }
        | Revision::Unknown => Status::Warning,
    };
    let cases = [
        (None, Status::Warning, "predates this CLI"),
        (
            Some(
                serde_json::json!({ "version": ours.version(), "revision": ours.revision().to_string() }),
            ),
            same,
            "",
        ),
        (
            Some(serde_json::json!({ "version": "0.0.1", "revision": commit })),
            Status::Warning,
            "upgrade the older one",
        ),
        (
            Some(serde_json::json!({ "version": ours.version(), "revision": "not-a-commit" })),
            Status::Warning,
            "unrecognized revision \"not-a-commit\"",
        ),
    ];
    for (build, status, message) in cases {
        let app = Router::new()
            .route(
                "/health",
                get(move || {
                    let build = build.clone();
                    async move {
                        let mut value: serde_json::Value = serde_json::from_str(HEALTH).unwrap();
                        if let Some(build) = build {
                            value["build"] = build;
                        }
                        axum::Json(value)
                    }
                }),
            )
            .route("/api/storage", get(|| async { STORAGE }));
        let (endpoint, task) = mock(app).await;
        let report = Report::gateway(&endpoint).await;
        let check = report.checks.iter().find(|c| c.name == "build").unwrap();
        assert_eq!(check.status, status, "{}", check.message);
        assert!(check.message.contains(message), "{}", check.message);
        // A build mismatch never fails the gateway check.
        assert!(report.success());
        task.abort();
    }
}

#[test]
fn malformed_config_and_dry_run_do_not_leak_or_create_storage() {
    let temp = tempfile::tempdir().unwrap();
    let config = temp.path().join("broken.toml");
    std::fs::write(&config, "unknown = 'SECRET'\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_tgv"))
        .args(["--config", config.to_str().unwrap(), "doctor", "--json"])
        .env("HOME", temp.path())
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("TGV_GATEWAY")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["ok"], false);
    assert!(!String::from_utf8_lossy(&output.stdout).contains("SECRET"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("SECRET"));
    let data = temp.path().join("must-not-exist");
    let output = Command::new(env!("CARGO_BIN_EXE_tgv"))
        .args([
            "--data-dir",
            data.to_str().unwrap(),
            "run",
            "codex",
            "--dry-run",
            "--gateway",
            "http://127.0.0.1:1",
            "--",
            "PRIVATE_PROMPT",
        ])
        .env("OPENAI_API_KEY", "SECRET")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let plan: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(plan["executable"], "codex");
    assert!(!data.exists());
    assert!(!String::from_utf8_lossy(&output.stdout).contains("SECRET"));
    assert!(!String::from_utf8_lossy(&output.stdout).contains("PRIVATE_PROMPT"));
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn launch_preserves_arguments_credentials_and_exit_status_without_editing_files() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let mut daemon = Daemon::new(
        Config {
            listen: "127.0.0.1:0".parse().unwrap(),
            ..Config::default()
        }
        .resolve()
        .unwrap(),
        temp.path(),
        tgv_cli::Cli::build(),
    );
    daemon.start().await.unwrap();
    let fake = temp.path().join("claude");
    std::fs::write(&fake, "#!/bin/sh\nprintf '%s\\n' \"$ANTHROPIC_BASE_URL\" \"$CLAUDE_CODE_GATEWAY_HINT_HEADERS\" \"$ANTHROPIC_API_KEY\" \"$@\"\nexit 37\n").unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o700)).unwrap();
    let home = temp.path().join("client-home");
    std::fs::create_dir(&home).unwrap();
    let gateway = format!("http://{}", daemon.address().unwrap());
    let expected =
        format!("{gateway}/anthropic\n1\nFIXTURE_TOKEN\n-p\n$(touch unwanted); 'literal'\n");
    let output = Command::new(env!("CARGO_BIN_EXE_tgv"))
        .args([
            "run",
            "claude",
            "--gateway",
            &gateway,
            "--",
            "-p",
            "$(touch unwanted); 'literal'",
        ])
        .env("PATH", temp.path())
        .env("HOME", &home)
        .env("ANTHROPIC_API_KEY", "FIXTURE_TOKEN")
        .current_dir(&home)
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(37),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8(output.stdout).unwrap(), expected);
    assert_eq!(std::fs::read_dir(&home).unwrap().count(), 0);
    assert!(!String::from_utf8_lossy(&output.stderr).contains("FIXTURE_TOKEN"));
    let report = Report::local(
        &HostEnv::default(),
        None,
        Some(temp.path().to_owned()),
        &flag(&gateway),
    )
    .await;
    assert!(report.success());
    assert!(
        report
            .checks
            .iter()
            .any(|c| c.name == "local storage" && c.status == Status::Skipped)
    );
    let config = temp.path().join("tgv.toml");
    std::fs::write(
        &config,
        format!("listen = \"{}\"\n", daemon.address().unwrap()),
    )
    .unwrap();
    let local_report = Report::local(
        &HostEnv::default(),
        Some(&config),
        Some(temp.path().to_owned()),
        &GatewayChoice::LocalListener,
    )
    .await;
    assert!(local_report.success());
    assert!(
        local_report
            .checks
            .iter()
            .any(|c| c.name == "local storage" && c.status == Status::Ok)
    );

    std::fs::write(&fake, "#!/bin/sh\nkill -TERM $$\n").unwrap();
    let signal_status = Command::new(env!("CARGO_BIN_EXE_tgv"))
        .args(["run", "claude", "--gateway", &gateway])
        .env("PATH", temp.path())
        .env("HOME", &home)
        .output()
        .unwrap()
        .status;
    use std::os::unix::process::ExitStatusExt;
    assert_eq!(signal_status.signal(), Some(15));
    assert!(
        tgv_store::Reader::open(temp.path(), 1)
            .await
            .unwrap()
            .calls(&tgv_store::query::CallQuery::default())
            .await
            .unwrap()
            .calls
            .is_empty()
    );
    daemon.shutdown();
    daemon.wait().await.unwrap();

    // Failed preflight must never reach the client executable.
    std::fs::write(&fake, "#!/bin/sh\nexit 37\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_tgv"))
        .args(["run", "claude", "--gateway", &gateway])
        .env("PATH", temp.path())
        .env("HOME", &home)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
}

#[tokio::test]
async fn stalled_health_body_has_a_total_deadline() {
    use futures_util::stream;
    use std::{convert::Infallible, time::Duration};
    let (endpoint, task) = mock(
        Router::new()
            .route(
                "/health",
                get(|| async {
                    Body::from_stream(stream::pending::<Result<bytes::Bytes, Infallible>>())
                }),
            )
            .route("/api/storage", get(|| async { STORAGE })),
    )
    .await;
    let report = tokio::time::timeout(Duration::from_secs(7), Report::gateway(&endpoint))
        .await
        .unwrap();
    assert!(!report.success());
    task.abort();
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn pi_launcher_materializes_only_bundled_code_and_preserves_client_arguments() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let fake = temp.path().join("pi");
    std::fs::write(
        &fake,
        "#!/bin/sh\nprintf '%s\\n' \"$TGV_PI_GATEWAY\" \"$@\"\nexit 29\n",
    )
    .unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o700)).unwrap();
    let (endpoint, task) = mock(
        Router::new()
            .route("/health", get(|| async { HEALTH }))
            .route("/api/storage", get(|| async { STORAGE })),
    )
    .await;
    let cache = temp.path().join("cache");
    let home = temp.path().join("home");
    std::fs::create_dir(&home).unwrap();
    let gateway = endpoint.url("/").to_string();
    let output = Command::new(env!("CARGO_BIN_EXE_tgv"))
        .args([
            "run",
            "pi",
            "--gateway",
            &gateway,
            "--",
            "--model",
            "anthropic/fixture",
            "literal $PROMPT",
        ])
        .env("PATH", temp.path())
        .env("HOME", &home)
        .env("XDG_CACHE_HOME", &cache)
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(29),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    let lines: Vec<_> = stdout.lines().collect();
    assert_eq!(lines[0], gateway);
    assert_eq!(lines[1], "--extension");
    let path = std::path::Path::new(lines[2]);
    assert!(path.starts_with(&cache));
    assert_eq!(
        std::fs::read_to_string(path).unwrap(),
        tgv_harness::pi::PiCache::SOURCE
    );
    assert_eq!(
        &lines[3..],
        ["--model", "anthropic/fixture", "literal $PROMPT"]
    );
    assert_eq!(std::fs::read_dir(home).unwrap().count(), 0);
    task.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn init_saves_a_checked_gateway_that_run_and_doctor_then_use() {
    use std::os::unix::fs::PermissionsExt;
    let (endpoint, task) = mock(
        Router::new()
            .route("/health", get(|| async { HEALTH }))
            .route("/api/storage", get(|| async { STORAGE })),
    )
    .await;
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    std::fs::create_dir(&home).unwrap();
    let file = home.join(".config/tgv/client.toml");
    let tgv = |args: &[&str], gateway_env: Option<&str>| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_tgv"));
        command
            .args(args)
            .env("HOME", &home)
            .env_remove("XDG_CONFIG_HOME")
            .env_remove("TGV_GATEWAY")
            .stdin(Stdio::null());
        if let Some(value) = gateway_env {
            command.env("TGV_GATEWAY", value);
        }
        command.output().unwrap()
    };

    // Without a terminal, init never waits for an answer.
    let output = tgv(&["init"], None);
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("--gateway"));
    assert!(!file.exists());
    // A gateway that fails its check is not saved.
    let output = tgv(&["init", "--gateway", "http://127.0.0.1:1"], None);
    assert_eq!(output.status.code(), Some(1));
    assert!(!file.exists());

    let gateway = endpoint.url("/").to_string();
    let output = tgv(&["init", "--gateway", &gateway], None);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        format!("gateway = \"{gateway}\"\n")
    );
    assert_eq!(
        std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
        0o600
    );

    // run and doctor use the saved gateway without --gateway.
    let output = tgv(&["run", "claude", "--dry-run"], None);
    assert!(output.status.success());
    let plan: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        plan["environment"]["ANTHROPIC_BASE_URL"],
        format!("{gateway}anthropic")
    );
    let output = tgv(&["doctor", "--json"], None);
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let check = |name: &str| {
        report["checks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|check| check["name"] == name)
            .cloned()
            .unwrap()
    };
    assert!(
        check("gateway choice")["message"]
            .as_str()
            .unwrap()
            .contains("client.toml")
    );
    assert_eq!(check("local storage")["status"], "skipped");
    assert_eq!(check("gateway")["status"], "ok");
    // Piped text stays plain and aligned; CLICOLOR_FORCE colors it.
    let output = tgv(&["doctor"], None);
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(!text.contains('\u{1b}'));
    assert!(
        text.lines()
            .any(|line| line.starts_with("SKIP local storage      "))
    );
    let summary = text.lines().last().unwrap();
    assert!(
        summary.contains(" ok, ") && summary.contains(" failed, ") && summary.ends_with(" skipped"),
        "{summary}"
    );
    let output = Command::new(env!("CARGO_BIN_EXE_tgv"))
        .arg("doctor")
        .env("HOME", &home)
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("TGV_GATEWAY")
        .env_remove("NO_COLOR")
        .env("CLICOLOR_FORCE", "1")
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&output.stdout).contains("\u{1b}["));

    // TGV_GATEWAY wins over the file, and --gateway over both.
    let output = tgv(
        &["run", "claude", "--dry-run"],
        Some("https://env.example.ts.net"),
    );
    let plan: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        plan["environment"]["ANTHROPIC_BASE_URL"],
        "https://env.example.ts.net/anthropic"
    );
    let output = tgv(
        &[
            "run",
            "claude",
            "--dry-run",
            "--gateway",
            "https://flag.example.ts.net",
        ],
        Some("https://env.example.ts.net"),
    );
    let plan: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        plan["environment"]["ANTHROPIC_BASE_URL"],
        "https://flag.example.ts.net/anthropic"
    );
    task.abort();
}
