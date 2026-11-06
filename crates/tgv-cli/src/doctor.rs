//! Bounded, credential-free diagnostics. Never sign in or issue inference requests.
use crate::client::GatewayChoice;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
use tgv_daemon::config::Config;
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
    pub fn print(&self, json: bool) -> Result<(), serde_json::Error> {
        if json {
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &serde_json::json!({"ok": self.success(), "checks": self.checks})
                )?
            );
        } else {
            for check in &self.checks {
                let label = match check.status {
                    Status::Ok => "OK",
                    Status::Warning => "WARN",
                    Status::Failed => "FAIL",
                    Status::Skipped => "SKIP",
                };
                println!("{label:4} {:18} {}", check.name, check.message);
            }
        }
        Ok(())
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
    recorder: RecorderHealth,
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
            Ok((200, storage)) if storage.policy.validate().is_ok() => report.add(
                "storage API",
                Status::Ok,
                format!(
                    "{} retained calls, {} bytes on disk, {} open calls; retention {}",
                    storage.usage.calls,
                    storage
                        .usage
                        .disk
                        .metadata
                        .live_bytes()
                        .saturating_add(storage.usage.disk.blobs.live_bytes()),
                    storage.usage.open_calls,
                    if storage.policy.enabled {
                        "enabled"
                    } else {
                        "disabled"
                    }
                ),
            ),
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
