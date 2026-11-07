//! Bounded, credential-free diagnostics. Never sign in or issue inference requests.
use crate::Cli;
use crate::client::GatewayChoice;
use crate::style::{Styling, Tone};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
use tgv_daemon::config::Config;
use tgv_gateway::build::{Build, Comparison, Revision};
use tgv_harness::{endpoint::GatewayEndpoint, host::HostEnv};
use tgv_recorder::retention::RetentionPolicy;
use tgv_store::{Reader, query::CallQuery};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Ok,
    Warning,
    Failed,
    Skipped,
}

impl Status {
    /// The text-report label and its tone.
    fn label(self) -> (&'static str, Tone) {
        match self {
            Self::Ok => ("OK", Tone::Good),
            Self::Warning => ("WARN", Tone::Warn),
            Self::Failed => ("FAIL", Tone::Bad),
            Self::Skipped => ("SKIP", Tone::Muted),
        }
    }

    /// The tone a check with this status is shown in.
    pub(crate) fn tone(self) -> Tone {
        self.label().1
    }

    /// How the summary counts `n` checks with this status.
    fn tally(self, n: usize) -> String {
        match self {
            Self::Ok => format!("{n} ok"),
            Self::Warning if n == 1 => "1 warning".to_owned(),
            Self::Warning => format!("{n} warnings"),
            Self::Failed => format!("{n} failed"),
            Self::Skipped => format!("{n} skipped"),
        }
    }
}

#[derive(Serialize)]
pub struct Check {
    pub name: &'static str,
    pub status: Status,
    pub message: String,
}

#[derive(Default, Serialize)]
pub struct Report {
    pub checks: Vec<Check>,
}

impl Report {
    pub fn success(&self) -> bool {
        !self.checks.iter().any(|c| c.status == Status::Failed)
    }
    /// A report with one failed check, for a diagnosis that cannot start.
    pub(crate) fn failed(name: &'static str, message: impl Into<String>) -> Self {
        let mut report = Self::default();
        report.add(name, Status::Failed, message);
        report
    }

    fn add(&mut self, name: &'static str, status: Status, message: impl Into<String>) {
        self.checks.push(Check {
            name,
            status,
            message: message.into(),
        });
    }
    /// Prints the report: JSON, or one aligned line per check and a summary.
    pub fn print(&self, json: bool, styling: Styling) -> Result<(), serde_json::Error> {
        if json {
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &serde_json::json!({"ok": self.success(), "checks": self.checks})
                )?
            );
            return Ok(());
        }
        for check in &self.checks {
            let (label, tone) = check.status.label();
            // Pad before painting: escape codes would count as width.
            let label = styling.paint(tone, &format!("{label:4}"));
            let name = format!("{:18}", check.name);
            match check.status {
                Status::Skipped => println!(
                    "{label} {} {}",
                    styling.paint(Tone::Muted, &name),
                    styling.paint(Tone::Muted, &check.message)
                ),
                Status::Ok | Status::Warning | Status::Failed => {
                    println!(
                        "{label} {} {}",
                        styling.paint(Tone::Strong, &name),
                        check.message
                    );
                }
            }
        }
        println!("\n{}", self.summary(styling));
        Ok(())
    }

    /// "8 ok, 1 warning, 0 failed, 3 skipped", each nonzero count in its tone.
    fn summary(&self, styling: Styling) -> String {
        [Status::Ok, Status::Warning, Status::Failed, Status::Skipped]
            .into_iter()
            .map(|status| {
                let n = self.checks.iter().filter(|c| c.status == status).count();
                let tone = if n == 0 { Tone::Muted } else { status.tone() };
                styling.paint(tone, &status.tally(n))
            })
            .collect::<Vec<_>>()
            .join(", ")
    }
}

impl Report {
    /// Diagnose this host: configuration, storage, client executables, and the
    /// gateway: the selected one, or the local listener when none is.
    pub async fn local(
        env: &HostEnv,
        config_path: Option<&Path>,
        data_dir: Option<PathBuf>,
        gateway: &GatewayChoice,
    ) -> Self {
        let mut report = Self::default();
        let config = match Config::load(config_path) {
            Ok(config) => {
                report.add("configuration", Status::Ok, "local configuration is valid");
                config
            }
            Err(error) => {
                report.add("configuration", Status::Failed, error.to_string());
                return report;
            }
        };
        report.add("gateway choice", Status::Ok, gateway.describe());
        let remote = match gateway {
            GatewayChoice::Selected { .. } => true,
            GatewayChoice::LocalListener => false,
        };
        if remote {
            report.add(
                "local storage",
                Status::Skipped,
                "gateway selected; storage is checked through its API",
            );
            report.add(
                "upstream config",
                Status::Skipped,
                "local upstream settings do not describe the selected gateway",
            );
        } else {
            let valid_upstreams = config.check().is_ok();
            report.add(
                "upstream config",
                if valid_upstreams {
                    Status::Ok
                } else {
                    Status::Failed
                },
                if valid_upstreams {
                    "upstream URLs and transport configuration are valid; connectivity not tested"
                } else {
                    "invalid upstream URL or transport configuration"
                },
            );
            match config.resolve_data_dir(data_dir, env) {
                Ok(dir) => report.local_storage(&dir).await,
                Err(error) => report.add("local storage", Status::Failed, error.to_string()),
            }
        }
        for executable in ["claude", "codex", "pi"] {
            let present = Self::executable_available(env, executable);
            report.add(
                executable,
                if present { Status::Ok } else { Status::Warning },
                if present {
                    "executable found on PATH; version and sign-in not checked"
                } else {
                    "not found on PATH; install it if you use this client"
                },
            );
        }
        let overrides = Self::credential_overrides(env);
        if !overrides.is_empty() {
            report.add(
                "auth environment",
                Status::Warning,
                format!(
                    "{} set; these can change client authentication. Values are not printed",
                    overrides.join(", ")
                ),
            );
        }
        match gateway.endpoint(config.listen) {
            Ok(gateway) => report.checks.extend(Self::gateway(&gateway).await.checks),
            Err(error) => report.add("gateway URL", Status::Failed, error.to_string()),
        }
        report.add("subscription", Status::Skipped, "sign-in, provider connectivity, and entitlement require a real client request; no inference was sent");
        report
    }

    async fn local_storage(&mut self, dir: &Path) {
        let report = self;
        let db = dir.join("tgv.sqlite3");
        match std::fs::metadata(&db) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                report.add(
                    "local storage",
                    Status::Warning,
                    "database not created yet; start tgv serve with this data directory",
                );
                return;
            }
            Err(_) => {
                report.add(
                    "local storage",
                    Status::Failed,
                    "cannot inspect the local database",
                );
                return;
            }
            Ok(metadata) if !metadata.is_file() => {
                report.add(
                    "local storage",
                    Status::Failed,
                    "database path is not a regular file",
                );
                return;
            }
            Ok(_) => {}
        }
        let readable = tokio::time::timeout(GatewayProbe::DEADLINE, async {
            Reader::open(dir, 1)
                .await?
                .calls(&CallQuery::default())
                .await
        })
        .await;
        report.add(
        "local storage",
        if matches!(readable, Ok(Ok(_))) {
            Status::Ok
        } else {
            Status::Failed
        },
        if matches!(readable, Ok(Ok(_))) {
            "read-only history query passed; no migration or write probe performed"
        } else {
            "history query failed or timed out; check permissions, schema, and database integrity"
        },
    );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let private = [dir.to_path_buf(), db].iter().all(|path| {
                std::fs::metadata(path).is_ok_and(|m| m.permissions().mode() & 0o077 == 0)
            });
            report.add(
                "storage permissions",
                if private { Status::Ok } else { Status::Warning },
                if private {
                    "directory and database have owner-only mode bits"
                } else {
                    "directory or database is accessible to group/others; review capture privacy"
                },
            );
        }
    }

    /// Environment variables that change client authentication, by name.
    /// Presence only; auth files, credential helpers, and values are never read.
    pub fn credential_overrides(env: &HostEnv) -> Vec<&'static str> {
        [
            "ANTHROPIC_API_KEY",
            "ANTHROPIC_AUTH_TOKEN",
            "OPENAI_API_KEY",
            "CODEX_API_KEY",
        ]
        .into_iter()
        .filter(|key| env.is_set(key))
        .collect()
    }

    fn executable_available(env: &HostEnv, name: &str) -> bool {
        env.search_path().into_iter().any(|dir| {
            #[cfg(windows)]
            let file = dir.join(format!("{name}.exe"));
            #[cfg(not(windows))]
            let file = dir.join(name);
            std::fs::metadata(file).is_ok_and(|metadata| {
                if !metadata.is_file() {
                    return false;
                }
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    metadata.permissions().mode() & 0o111 != 0
                }
                #[cfg(not(unix))]
                {
                    true
                }
            })
        })
    }
}

/// The parts of `/api/storage` doctor reports.
#[derive(Deserialize)]
struct StorageResponse {
    policy: RetentionPolicy,
    usage: StorageUsage,
}

#[derive(Deserialize)]
struct StorageUsage {
    calls: u64,
    open_calls: u64,
    disk: DiskUsage,
}

#[derive(Deserialize)]
struct DiskUsage {
    metadata: FileUsage,
    blobs: FileUsage,
    /// Absent from daemons older than search, and null before it is built.
    #[serde(default)]
    search: Option<IndexUsage>,
}

#[derive(Deserialize)]
struct IndexUsage {
    file: FileUsage,
    indexed_calls: u64,
    stale_calls: u64,
}

#[derive(Deserialize)]
struct FileUsage {
    page_size: u64,
    page_count: u64,
    freelist_count: u64,
}

impl FileUsage {
    fn live_bytes(&self) -> u64 {
        self.page_count
            .saturating_sub(self.freelist_count)
            .saturating_mul(self.page_size)
    }
}

/// The parts of `/health` doctor reports.
#[derive(Deserialize)]
struct HealthResponse {
    /// Absent from daemons older than build reporting.
    build: Option<ReportedBuild>,
    recorder: RecorderHealth,
}

/// The daemon's build, with its revision kept as text until parsed, so an
/// unrecognized one is reported rather than failing the whole health check.
#[derive(Deserialize)]
struct ReportedBuild {
    version: String,
    revision: String,
}

#[derive(Deserialize)]
struct RecorderHealth {
    storage_errors: u64,
    retention_errors: u64,
    omissions: Omissions,
}

#[derive(Deserialize)]
struct Omissions {
    policy_limit_bytes: u64,
    queue_bytes_bytes: u64,
    queue_slots_bytes: u64,
    storage_failure_bytes: u64,
    unrecorded_calls: u64,
}

impl Report {
    /// Whether the daemon runs the same build as this CLI. A mismatch is a
    /// warning: the API is versioned, but one of them needs an upgrade.
    fn build(&mut self, local: &Build, remote: Option<ReportedBuild>) {
        let Some(remote) = remote else {
            self.add(
                "build",
                Status::Warning,
                format!("daemon does not report its build, so it predates this CLI ({local})"),
            );
            return;
        };
        let revision = match remote.revision.parse::<Revision>() {
            Ok(revision) => revision,
            Err(_) => {
                self.add(
                    "build",
                    Status::Warning,
                    format!(
                        "daemon reports an unrecognized revision {:?}; this CLI is {local}",
                        remote.revision
                    ),
                );
                return;
            }
        };
        let remote = Build::new(remote.version, revision);
        let (status, message) = match local.compare(&remote) {
            Comparison::Same => (
                Status::Ok,
                format!("daemon runs the same build as this CLI: {local}"),
            ),
            Comparison::Different => (
                Status::Warning,
                format!("daemon runs {remote} but this CLI is {local}; upgrade the older one"),
            ),
            Comparison::Unconfirmed => (
                Status::Warning,
                format!(
                    "cannot confirm that the daemon ({remote}) and this CLI ({local}) are the same build: a revision is unknown or has local modifications"
                ),
            ),
        };
        self.add("build", status, message);
    }

    /// Diagnose a gateway through its API: health, capture, and storage.
    pub async fn gateway(gateway: &GatewayEndpoint) -> Self {
        let mut report = Self::default();
        let probe = match GatewayProbe::new() {
            Ok(probe) => probe,
            Err(_) => {
                report.add(
                    "gateway",
                    Status::Failed,
                    "cannot initialize diagnostic transport",
                );
                return report;
            }
        };
        match probe.read::<HealthResponse>(gateway.url("/health")).await {
            Ok((status, health)) if status == 200 || status == 503 => {
                report.build(&Cli::build(), health.build);
                let health = health.recorder;
                let errors = health.storage_errors > 0 || health.retention_errors > 0;
                report.add("gateway", if status == 200 && !errors {Status::Ok} else {Status::Failed},
                if status == 200 && !errors {"daemon is reachable; recorder reports no storage errors"} else {"daemon is reachable but recording/retention reports errors; inspect daemon logs"});
                let omissions = health.omissions;
                let queue = omissions
                    .queue_bytes_bytes
                    .saturating_add(omissions.queue_slots_bytes);
                let loss = omissions.unrecorded_calls > 0
                    || queue > 0
                    || omissions.storage_failure_bytes > 0;
                report.add("capture", if loss {Status::Warning} else {Status::Ok}, format!("since daemon start: {} unrecorded calls, {} body bytes omitted by queue limits, {} lost to storage failures, {} beyond the body limit", omissions.unrecorded_calls, queue, omissions.storage_failure_bytes, omissions.policy_limit_bytes));
            }
            Ok(_) => report.add(
                "gateway",
                Status::Failed,
                "unexpected health status; check the gateway address",
            ),
            Err(error) => report.add("gateway", Status::Failed, error.to_string()),
        }
        match probe
            .read::<StorageResponse>(gateway.url("/api/storage"))
            .await
        {
            Ok((200, storage)) if storage.policy.validate().is_ok() => {
                let usage = &storage.usage;
                let search = usage.disk.search.as_ref();
                report.add(
                    "storage API",
                    Status::Ok,
                    format!(
                        "{} retained calls, {} bytes on disk, {} open calls; retention {}",
                        usage.calls,
                        usage
                            .disk
                            .metadata
                            .live_bytes()
                            .saturating_add(usage.disk.blobs.live_bytes())
                            .saturating_add(search.map_or(0, |index| index.file.live_bytes())),
                        usage.open_calls,
                        if storage.policy.enabled {
                            "enabled"
                        } else {
                            "disabled"
                        }
                    ),
                );
                report.add_search_index(search, usage.calls.saturating_sub(usage.open_calls));
            }
            Ok(_) => report.add(
                "storage API",
                Status::Failed,
                "storage API returned an unexpected status or policy",
            ),
            Err(error) => report.add(
                "storage API",
                Status::Failed,
                format!("{error}; the reader may be busy, retry doctor"),
            ),
        }
        report
    }
}

impl Report {
    /// How much of the finished history the search index covers.
    fn add_search_index(&mut self, search: Option<&IndexUsage>, finished: u64) {
        match search {
            None => self.add(
                "search index",
                Status::Warning,
                "the gateway reports no search index; a daemon with search builds it while running",
            ),
            Some(index) => {
                let kept = index.indexed_calls.saturating_sub(index.stale_calls);
                if kept >= finished {
                    self.add(
                        "search index",
                        Status::Ok,
                        format!(
                            "{kept} of {finished} finished calls indexed, {} bytes on disk",
                            index.file.live_bytes()
                        ),
                    );
                } else {
                    self.add(
                        "search index",
                        Status::Warning,
                        format!(
                            "{kept} of {finished} finished calls indexed; the daemon indexes the rest within seconds, longer while it backfills after a start"
                        ),
                    );
                }
            }
        }
    }
}

/// Why a diagnostic request failed. Remote text is never included.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ProbeError {
    #[error("cannot reach gateway within 5 seconds; check address, TLS, and that TGV is running")]
    Unreachable,
    #[error("gateway returned HTTP {0}; redirects are not followed")]
    Status(u16),
    #[error("gateway response failed or timed out")]
    Body,
    #[error("diagnostic response exceeds 64 KiB")]
    TooLarge,
    #[error("gateway returned an invalid diagnostic response")]
    Invalid,
}

/// A bounded HTTP client for diagnostics: no proxy, redirects, or retries.
struct GatewayProbe {
    client: reqwest::Client,
}

impl GatewayProbe {
    const DEADLINE: Duration = Duration::from_secs(5);
    const MAX_RESPONSE_BYTES: usize = 64 * 1024;

    fn new() -> Result<Self, reqwest::Error> {
        reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .connect_timeout(Self::DEADLINE)
            .timeout(Self::DEADLINE)
            .build()
            .map(|client| Self { client })
    }

    /// Suppress both remote response text and reqwest diagnostics: either can
    /// contain arbitrary values. The timeout covers headers and the body.
    async fn read<T: DeserializeOwned>(&self, url: reqwest::Url) -> Result<(u16, T), ProbeError> {
        let mut response = self
            .client
            .get(url)
            .header("accept", "application/json")
            .send()
            .await
            .map_err(|_| ProbeError::Unreachable)?;
        let status = response.status().as_u16();
        if status != 200 && status != 503 {
            return Err(ProbeError::Status(status));
        }
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| ProbeError::Body)? {
            if chunk.len() > Self::MAX_RESPONSE_BYTES.saturating_sub(body.len()) {
                return Err(ProbeError::TooLarge);
            }
            body.extend_from_slice(&chunk);
        }
        let data = serde_json::from_slice(&body).map_err(|_| ProbeError::Invalid)?;
        Ok((status, data))
    }
}
