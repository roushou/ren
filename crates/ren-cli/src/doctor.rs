//! Bounded, credential-free diagnostics. Never sign in or issue inference requests.
use crate::Cli;
use crate::client::GatewayChoice;
use crate::style::{Styling, Tone};
use ren_daemon::config::{Config, ToolsOptions};
use ren_daemon::tools::HostTools;
use ren_depot::{
    Surveyed, ToolHost,
    package::{Kind, PackageId, Requirement},
    provision::{Fit, Provision},
    version::ObservedVersion,
};
use ren_gateway::{
    Environment,
    build::{Build, Comparison, Revision},
};
use ren_harness::{
    Harness,
    endpoint::{GatewayEndpoint, UserToken},
    host::HostEnv,
};
use ren_protocol::label::{Labeled, Stored};
use ren_recorder::caller::CallerKind;
use ren_recorder::retention::RetentionPolicy;
use ren_store::{Reader, query::CallQuery};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_with::{DeserializeFromStr, SerializeDisplay};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
use strum::{Display, EnumString, IntoStaticStr, VariantArray};

/// How a check came out.
#[derive(Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, SerializeDisplay, DeserializeFromStr)]
pub enum Status {
    Ok,
    Warning,
    Failed,
    Skipped,
}

/// What a check looks at. Its label is the name `ren doctor` prints and
/// reports in JSON, which other tools read (`cargo xtask install-vps` reads
/// `build`).
#[derive(
    Display,
    EnumString,
    IntoStaticStr,
    VariantArray,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Debug,
    SerializeDisplay,
    DeserializeFromStr,
)]
pub enum CheckId {
    #[strum(to_string = "configuration")]
    Configuration,
    #[strum(to_string = "gateway choice")]
    GatewayChoice,
    #[strum(to_string = "local storage")]
    LocalStorage,
    #[strum(to_string = "upstream config")]
    UpstreamConfig,
    #[strum(to_string = "auth environment")]
    AuthEnvironment,
    #[strum(to_string = "gateway URL")]
    GatewayUrl,
    #[strum(to_string = "subscription")]
    Subscription,
    #[strum(to_string = "storage permissions")]
    StoragePermissions,
    #[strum(to_string = "tool catalog")]
    ToolCatalog,
    #[strum(to_string = "build")]
    Build,
    #[strum(to_string = "gateway")]
    Gateway,
    #[strum(to_string = "capture")]
    Capture,
    #[strum(to_string = "storage API")]
    StorageApi,
    #[strum(to_string = "instance")]
    Instance,
    #[strum(to_string = "caller")]
    Caller,
    #[strum(to_string = "digests")]
    Digests,
}

/// A check's name: one of Ren's own checks, or a package the host provides.
#[derive(Clone, PartialEq, Eq, Debug, SerializeDisplay)]
pub enum CheckName {
    Check(CheckId),
    Package(PackageId),
}

impl std::fmt::Display for CheckName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // `pad`, so a report's column width applies.
        match self {
            Self::Check(id) => f.pad(id.label()),
            Self::Package(package) => f.pad(package.as_str()),
        }
    }
}

impl Status {
    /// The text-report badge and its tone.
    fn badge(self) -> (&'static str, Tone) {
        match self {
            Self::Ok => ("OK", Tone::Good),
            Self::Warning => ("WARN", Tone::Warn),
            Self::Failed => ("FAIL", Tone::Bad),
            Self::Skipped => ("SKIP", Tone::Muted),
        }
    }

    /// The tone a check with this status is shown in.
    pub(crate) fn tone(self) -> Tone {
        self.badge().1
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
    pub name: CheckName,
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
    pub(crate) fn failed(name: CheckId, message: impl Into<String>) -> Self {
        let mut report = Self::default();
        report.add(name, Status::Failed, message);
        report
    }

    fn add(&mut self, name: CheckId, status: Status, message: impl Into<String>) {
        self.checks.push(Check {
            name: CheckName::Check(name),
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
            let (label, tone) = check.status.badge();
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
                report.add(
                    CheckId::Configuration,
                    Status::Ok,
                    "local configuration is valid",
                );
                config
            }
            Err(error) => {
                report.add(CheckId::Configuration, Status::Failed, error.to_string());
                return report;
            }
        };
        report.add(CheckId::GatewayChoice, Status::Ok, gateway.describe());
        let remote = match gateway {
            GatewayChoice::Selected { .. } => true,
            GatewayChoice::LocalListener => false,
        };
        if remote {
            report.add(
                CheckId::LocalStorage,
                Status::Skipped,
                "gateway selected; storage is checked through its API",
            );
            report.add(
                CheckId::UpstreamConfig,
                Status::Skipped,
                "local upstream settings do not describe the selected gateway",
            );
        } else {
            let valid_upstreams = config.check().is_ok();
            report.add(
                CheckId::UpstreamConfig,
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
                Err(error) => report.add(CheckId::LocalStorage, Status::Failed, error.to_string()),
            }
        }
        report.tools(&config.tools, &HostTools::new(env.search_path()));
        let overrides = Self::credential_overrides(env);
        if !overrides.is_empty() {
            report.add(
                CheckId::AuthEnvironment,
                Status::Warning,
                format!(
                    "{} set; these can change client authentication. Values are not printed",
                    overrides.join(", ")
                ),
            );
        }
        match gateway.endpoint(config.listen) {
            Ok(gateway) => report
                .checks
                .extend(Self::gateway(&gateway, env).await.checks),
            Err(error) => report.add(CheckId::GatewayUrl, Status::Failed, error.to_string()),
        }
        report.add(CheckId::Subscription, Status::Skipped, "sign-in, provider connectivity, and entitlement require a real client request; no inference was sent");
        report
    }

    async fn local_storage(&mut self, dir: &Path) {
        let report = self;
        let db = dir.join("ren.sqlite3");
        match std::fs::metadata(&db) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                report.add(
                    CheckId::LocalStorage,
                    Status::Warning,
                    "database not created yet; start ren serve with this data directory",
                );
                return;
            }
            Err(_) => {
                report.add(
                    CheckId::LocalStorage,
                    Status::Failed,
                    "cannot inspect the local database",
                );
                return;
            }
            Ok(metadata) if !metadata.is_file() => {
                report.add(
                    CheckId::LocalStorage,
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
        CheckId::LocalStorage,
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
                CheckId::StoragePermissions,
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

    /// One check per package the depot knows: where this host's search path
    /// has it and which version it reported. A missing client is a warning;
    /// a missing runtime or utility is one only when a client found here
    /// needs it.
    fn tools(&mut self, tools: &ToolsOptions, host: &impl ToolHost) {
        let depot = match tools.depot() {
            Ok(depot) => depot,
            Err(error) => {
                self.add(CheckId::ToolCatalog, Status::Failed, error.to_string());
                return;
            }
        };
        let survey = depot.survey(host);
        let present = |id: &PackageId| {
            survey.iter().any(|surveyed| {
                surveyed.package.id() == id && !matches!(surveyed.provision, Provision::Missing)
            })
        };
        let requirements: Vec<(Harness, Requirement)> = Harness::ALL
            .iter()
            .flat_map(|harness| {
                harness
                    .requirements()
                    .into_iter()
                    .map(|requirement| (*harness, requirement))
            })
            .collect();
        for Surveyed { package, provision } in &survey {
            // Requirements of this package from clients installed here.
            let needed_by: Vec<&(Harness, Requirement)> = requirements
                .iter()
                .filter(|(harness, requirement)| {
                    requirement.package() == package.id()
                        && harness
                            .requirements()
                            .iter()
                            .filter(|other| other.package() != package.id())
                            .all(|other| present(other.package()))
                })
                .collect();
            let (status, message) = match provision {
                Provision::Missing => match package.kind() {
                    Kind::Client => (
                        Status::Warning,
                        "not found on PATH; install it if you use this client".to_owned(),
                    ),
                    Kind::Runtime | Kind::Utility if needed_by.is_empty() => (
                        Status::Skipped,
                        "not found on PATH; no client found here needs it".to_owned(),
                    ),
                    Kind::Runtime | Kind::Utility => (
                        Status::Warning,
                        format!(
                            "not found on PATH; {} needs it",
                            Self::names(needed_by.iter().map(|(harness, _)| *harness))
                        ),
                    ),
                },
                Provision::System(found) => {
                    let outside: Vec<&(Harness, Requirement)> = needed_by
                        .iter()
                        .copied()
                        .filter(|(_, requirement)| {
                            matches!(
                                provision.fit(requirement.wants()),
                                Fit::Outside | Fit::Unverified
                            )
                        })
                        .collect();
                    let at = found.path.display();
                    let version = match &found.version {
                        ObservedVersion::Read(version) => format!("{version} at {at}"),
                        ObservedVersion::Unparsed(raw) => {
                            format!("found at {at}; version not read from \"{raw}\"")
                        }
                        ObservedVersion::Unread(failure) => {
                            format!("found at {at}; version not read: {failure}")
                        }
                    };
                    if outside.is_empty() {
                        (Status::Ok, version)
                    } else {
                        (
                            Status::Warning,
                            format!(
                                "{version}; not a version {} accepts",
                                Self::names(outside.iter().map(|(harness, _)| *harness))
                            ),
                        )
                    }
                }
            };
            self.checks.push(Check {
                name: CheckName::Package(package.id().clone()),
                status,
                message,
            });
        }
    }

    fn names(harnesses: impl Iterator<Item = Harness>) -> String {
        harnesses
            .map(|harness| harness.to_string())
            .collect::<Vec<_>>()
            .join(", ")
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

#[cfg(test)]
mod tests {
    use super::{CheckId, CheckName};

    #[test]
    fn a_check_name_fills_the_report_column_and_reports_its_label() {
        let name = CheckName::Check(CheckId::GatewayUrl);
        assert_eq!(format!("{name:18}|"), "gateway URL       |");
        assert_eq!(serde_json::to_string(&name).unwrap(), r#""gateway URL""#);
        let package = CheckName::Package("node".parse().unwrap());
        assert_eq!(format!("{package:6}|"), "node  |");
    }
}
