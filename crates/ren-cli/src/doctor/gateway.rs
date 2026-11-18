//! A gateway's side of the report: its `/health`, `/api/storage`, and
//! caller views, read with one bounded probe each.

use super::{CheckId, Report, Status};
use crate::Cli;
use ren_gateway::{
    Environment,
    build::{Build, Comparison, Revision},
};
use ren_harness::{
    endpoint::{GatewayEndpoint, UserToken},
    host::HostEnv,
};
use ren_protocol::label::Stored;
use ren_recorder::caller::CallerKind;
use ren_recorder::retention::RetentionPolicy;
use serde::{Deserialize, de::DeserializeOwned};
use std::time::Duration;

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
    digested_calls: u64,
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

/// `/api/caller`'s answer.
#[derive(Deserialize)]
struct CallerResponse {
    caller: CallerSeen,
}

/// A caller as the gateway reported it.
#[derive(Deserialize)]
struct CallerSeen {
    kind: Stored<CallerKind>,
    user: Option<String>,
    login: Option<String>,
}

impl CallerSeen {
    fn describe(&self) -> String {
        let user = self.user.as_deref().unwrap_or("?");
        let login = self.login.as_deref().unwrap_or("?");
        match self.kind.known() {
            Some(CallerKind::UserTailscale) => {
                format!("calls are recorded as user {user}, by Tailscale login {login}")
            }
            Some(CallerKind::UserToken) => {
                format!("calls are recorded as user {user}, by REN_USER_TOKEN")
            }
            Some(CallerKind::Unlisted) => format!(
                "calls are recorded as Tailscale login {login}, which no [[users]] entry lists"
            ),
            Some(CallerKind::Unidentified) => {
                "calls are recorded as unidentified; set REN_USER_TOKEN, or reach the gateway through Tailscale Serve, to be a named user".to_owned()
            }
            None => format!("the gateway reports an unknown caller kind ({})", self.kind),
        }
    }
}

/// The parts of `/health` doctor reports.
#[derive(Deserialize)]
struct HealthResponse {
    build: ReportedBuild,
    /// Absent from daemons that predate it: a production instance.
    #[serde(default)]
    instance: ReportedInstance,
    recorder: RecorderHealth,
}

/// What the daemon says it is.
#[derive(Deserialize, Default)]
struct ReportedInstance {
    #[serde(default)]
    environment: Option<Stored<Environment>>,
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
    fn build(&mut self, local: &Build, remote: ReportedBuild) {
        let revision = match remote.revision.parse::<Revision>() {
            Ok(revision) => revision,
            Err(_) => {
                self.add(
                    CheckId::Build,
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
        self.add(CheckId::Build, status, message);
    }

    /// Diagnose a gateway through its API: health, capture, storage, and
    /// how it identifies this caller.
    pub async fn gateway(gateway: &GatewayEndpoint, env: &HostEnv) -> Self {
        let mut report = Self::default();
        let probe = match GatewayProbe::new() {
            Ok(probe) => probe,
            Err(_) => {
                report.add(
                    CheckId::Gateway,
                    Status::Failed,
                    "cannot initialize diagnostic transport",
                );
                return report;
            }
        };
        report.caller(&probe, gateway, env).await;
        match probe
            .read::<HealthResponse>(gateway.url("/health"), None)
            .await
        {
            Ok((status, health)) if status == 200 || status == 503 => {
                report.build(&Cli::build(), health.build);
                report.instance(&health.instance);
                let health = health.recorder;
                let errors = health.storage_errors > 0 || health.retention_errors > 0;
                report.add(CheckId::Gateway, if status == 200 && !errors {Status::Ok} else {Status::Failed},
                if status == 200 && !errors {"daemon is reachable; recorder reports no storage errors"} else {"daemon is reachable but recording/retention reports errors; inspect daemon logs"});
                let omissions = health.omissions;
                let queue = omissions
                    .queue_bytes_bytes
                    .saturating_add(omissions.queue_slots_bytes);
                let loss = omissions.unrecorded_calls > 0
                    || queue > 0
                    || omissions.storage_failure_bytes > 0;
                report.add(CheckId::Capture, if loss {Status::Warning} else {Status::Ok}, format!("since daemon start: {} unrecorded calls, {} body bytes omitted by queue limits, {} lost to storage failures, {} beyond the body limit", omissions.unrecorded_calls, queue, omissions.storage_failure_bytes, omissions.policy_limit_bytes));
            }
            Ok(_) => report.add(
                CheckId::Gateway,
                Status::Failed,
                "unexpected health status; check the gateway address",
            ),
            Err(error) => report.add(CheckId::Gateway, Status::Failed, error.to_string()),
        }
        match probe
            .read::<StorageResponse>(gateway.url("/api/storage"), None)
            .await
        {
            Ok((200, storage)) if storage.policy.validate().is_ok() => {
                let usage = &storage.usage;
                report.add(
                    CheckId::StorageApi,
                    Status::Ok,
                    format!(
                        "{} retained calls, {} bytes on disk, {} open calls; retention {}",
                        usage.calls,
                        usage
                            .disk
                            .metadata
                            .live_bytes()
                            .saturating_add(usage.disk.blobs.live_bytes()),
                        usage.open_calls,
                        if storage.policy.enabled {
                            "enabled"
                        } else {
                            "disabled"
                        }
                    ),
                );
                report.add_digests(
                    usage.digested_calls,
                    usage.calls.saturating_sub(usage.open_calls),
                );
            }
            Ok(_) => report.add(
                CheckId::StorageApi,
                Status::Failed,
                "storage API returned an unexpected status or policy",
            ),
            Err(error) => report.add(
                CheckId::StorageApi,
                Status::Failed,
                format!("{error}; the reader may be busy, retry doctor"),
            ),
        }
        report
    }
}

impl Report {
    /// Says so when the gateway is not a production instance, so a dev
    /// slot is never mistaken for the installed service.
    fn instance(&mut self, instance: &ReportedInstance) {
        match instance.environment.as_ref().map(Stored::known) {
            None | Some(Some(Environment::Production)) => {}
            Some(Some(Environment::Development)) => self.add(
                CheckId::Instance,
                Status::Ok,
                "a development instance (such as the dev slot), not a production gateway",
            ),
            Some(None) => self.add(
                CheckId::Instance,
                Status::Warning,
                "the gateway reports an environment this CLI does not know",
            ),
        }
    }

    /// Who the gateway records this caller's calls as: by the user token in
    /// `REN_USER_TOKEN`, or by the Tailscale login Serve adds.
    async fn caller(&mut self, probe: &GatewayProbe, gateway: &GatewayEndpoint, env: &HostEnv) {
        let token = match UserToken::from_env(env) {
            Ok(token) => token,
            Err(error) => {
                self.add(CheckId::Caller, Status::Failed, error.to_string());
                return;
            }
        };
        let answer = probe
            .read::<CallerResponse>(gateway.url("/api/caller"), token.as_ref())
            .await;
        let (status, message) = match answer {
            Ok((_, CallerResponse { caller })) => (Status::Ok, caller.describe()),
            Err(ProbeError::Status(401)) => (
                Status::Failed,
                "the gateway refused this caller's identity: an unknown REN_USER_TOKEN, or one naming another user than the Tailscale login".to_owned(),
            ),
            Err(ProbeError::Status(404)) => (
                Status::Skipped,
                "the gateway does not report callers; it predates named users".to_owned(),
            ),
            Err(error) => (Status::Warning, error.to_string()),
        };
        self.add(CheckId::Caller, status, message);
    }

    /// How much of the finished history has its activity derived: what
    /// turns, reports, and search read.
    fn add_digests(&mut self, digested: u64, finished: u64) {
        if digested >= finished {
            self.add(
                CheckId::Digests,
                Status::Ok,
                format!("{digested} of {finished} finished calls derived"),
            );
        } else {
            self.add(
                CheckId::Digests,
                Status::Warning,
                format!(
                    "{digested} of {finished} finished calls derived; the daemon derives the rest from stored bodies within seconds, longer for a large history"
                ),
            );
        }
    }
}

/// Why a diagnostic request failed. Remote text is never included.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ProbeError {
    #[error("cannot reach gateway within 5 seconds; check address, TLS, and that Ren is running")]
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
pub(super) struct GatewayProbe {
    client: reqwest::Client,
}

impl GatewayProbe {
    pub(super) const DEADLINE: Duration = Duration::from_secs(5);
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
    async fn read<T: DeserializeOwned>(
        &self,
        url: reqwest::Url,
        token: Option<&UserToken>,
    ) -> Result<(u16, T), ProbeError> {
        let mut request = self.client.get(url).header("accept", "application/json");
        if let Some(token) = token {
            request = request.header(UserToken::HEADER, token.expose());
        }
        let mut response = request.send().await.map_err(|_| ProbeError::Unreachable)?;
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
