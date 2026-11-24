//! Strict startup configuration. This revision is immutable for the daemon lifetime.

use ren_depot::{CatalogEntry, CatalogError, Depot};
use ren_gateway::{
    BufferOptions, ConcurrencyLimit, ConfiguredEntry, ControlOptions, EntryIngress, GatewayConfig,
    InstanceOptions, NativeUrls, OperatorKey, TimeoutSecs, TransportTimeouts, UpstreamAuth,
    UpstreamEntry, UpstreamTable, UpstreamUrl, WebSocketOptions,
    policy::{EntryUsers, Policy, PolicyConfig},
    user::{UserConfig, Users},
};
use ren_github::{Client, Endpoint, PollPeriod, RepositorySlug, Token};
use ren_harness::{
    attachment::{AttachmentOptions, Limits},
    host::HostEnv,
    pi::PiCache,
};
use ren_protocol::route::{Api, Provider, UnsupportedWire, Wire};
use ren_recorder::retention::RetentionPolicy;
use ren_recorder::{
    CaptureOptions,
    caller::UserId,
    project::ProjectName,
    routing::{ConfigRevision, UpstreamId},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
    time::Duration,
};

/// Why a configuration was refused. Invalid values are named, never echoed:
/// they can carry credentials.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("cannot read configuration file")]
    Read(#[source] std::io::Error),
    #[error("invalid TOML configuration or unknown configuration field")]
    Syntax,
    #[error("unsupported configuration version")]
    Version,
    #[error("listen must be loopback; use Tailscale Serve for private remote access")]
    PublicListener,
    #[error(transparent)]
    Capture(#[from] ren_recorder::CaptureOptionsError),
    #[error(transparent)]
    Retention(#[from] ren_recorder::retention::RetentionError),
    #[error(transparent)]
    Buffer(#[from] ren_gateway::BufferOptionsError),
    #[error(transparent)]
    Attachments(#[from] ren_harness::attachment::AttachmentOptionsError),
    #[error("upstream {id}: {source}")]
    UpstreamUrl {
        id: String,
        source: ren_gateway::UpstreamConfigError,
    },
    #[error(transparent)]
    ProviderUrl(#[from] ren_gateway::UpstreamConfigError),
    #[error("upstream {id}: {source}")]
    Wire { id: String, source: UnsupportedWire },
    #[error("upstream {upstream}: users names {user}, which no [[users]] entry is")]
    UnknownEntryUser { upstream: String, user: String },
    #[error("upstream {0}: users is empty, so no caller could use it; omit it to admit anyone")]
    EmptyEntryUsers(String),
    #[error("upstream {0}: the Codex backend accepts no operator keys")]
    CodexOperatorKey(String),
    #[error(transparent)]
    Upstreams(#[from] ren_gateway::UpstreamTableError),
    #[error(transparent)]
    Policy(#[from] ren_gateway::policy::PolicyError),
    #[error(transparent)]
    Users(#[from] ren_gateway::user::UsersError),
    #[error("operator key variable {name} for upstream {upstream} is not set")]
    MissingKey { name: String, upstream: String },
    #[error("operator key variable {0} is not a valid header value")]
    InvalidKey(String),
    #[error("configuration cannot be serialized for its revision")]
    Revision(#[source] serde_json::Error),
    #[error("set --data-dir when HOME is unavailable")]
    NoDataDir,
    #[error("packages: {0}")]
    Packages(#[from] CatalogError),
    #[error(
        "runs.roots entries must be absolute paths, and runs.places and runs.scratch absolute or under ~/"
    )]
    RunRoot,
    #[error(
        "[runs] worktree_env names must be non-empty, without `=`, and not REN_WORKTREE* or REN_WORKSPACE*"
    )]
    WorktreeEnv,
    #[error("[runs] hold = \"container\" needs [runs.container] image")]
    RunImageMissing,
    #[error("[runs] hold = \"container\" needs HOME, the home a run's container takes")]
    RunHomeMissing,
    #[error("[runs.container] runtime = \"gvisor\" needs network = \"relayed\"")]
    RunRuntimeNotRelayed,
    #[error("[github] endpoint: {0}")]
    GithubEndpoint(ren_github::EndpointError),
    #[error("the GitHub client could not be built")]
    GithubClient(#[source] reqwest::Error),
    #[error("[github] watches project {0} twice")]
    GithubProjectTwice(String),
    #[error("GitHub token variable {0} is not set, or not a token")]
    MissingGithubToken(String),
    #[error("[github.app] api: {0}")]
    GithubAppApi(ren_github::EndpointError),
    #[error("[github.app] key_file {0} could not be read")]
    GithubAppKeyFile(PathBuf, #[source] std::io::Error),
    #[error("[github.app] key_file {0}: {1}")]
    GithubAppKey(PathBuf, ren_github::AppKeyError),
}

/// Where operator keys come from while resolving.
#[derive(Debug, Clone, Copy)]
enum Keys<'a> {
    Env(&'a HostEnv),
    /// Structure only: every key is a placeholder that is never used.
    Unchecked,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
/// Editable TOML representation. Use `resolve` before constructing a daemon.
pub struct Config {
    pub version: u32,
    pub listen: SocketAddr,
    pub data_dir: Option<PathBuf>,
    pub anthropic_base_url: String,
    pub codex_base_url: String,
    pub openai_base_url: String,
    pub deepseek_base_url: String,
    pub max_concurrent_requests: ConcurrencyLimit,
    /// Calls callers that are not named users may have running at once,
    /// together.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_concurrent_unnamed: Option<ConcurrencyLimit>,
    pub connect_timeout_seconds: TimeoutSecs,
    pub response_header_timeout_seconds: TimeoutSecs,
    pub stream_idle_timeout_seconds: TimeoutSecs,
    pub capture: CaptureOptions,
    pub retention: RetentionPolicy,
    pub websocket: WebSocketOptions,
    /// Extra upstream entries, each with an operator API key. Every
    /// provider also has a client-credential entry named after it.
    pub upstreams: Vec<UpstreamConfig>,
    pub buffer: BufferOptions,
    pub policy: PolicyConfig,
    pub control: ControlOptions,
    /// The gateway's named users, each proven by a Tailscale login or a
    /// user token.
    pub users: Vec<UserConfig>,
    /// Whether this is a production or a development instance.
    pub instance: InstanceOptions,
    /// Agent runs: off until roots are configured.
    pub runs: RunsOptions,
    /// How much a session's attachments may hold.
    pub attachments: AttachmentOptions,
    /// The packages the operator adds to the built-in catalog.
    #[serde(skip_serializing_if = "PackagesOptions::is_empty")]
    pub packages: PackagesOptions,
    /// The `[[packages]]` section's old place, refused by its new name.
    #[serde(skip_serializing)]
    pub tools: RenamedTools,
    /// The GitHub repositories watched for registered projects.
    #[serde(skip_serializing_if = "GithubOptions::is_off")]
    pub github: GithubOptions,
}

/// The `[github]` section: each registered project's repository on GitHub,
/// whose open pull requests and checks Ren polls, and the token it reads
/// them with. Off while no repository is named.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct GithubOptions {
    /// The environment variable holding a token that can read the
    /// repositories' pull requests and checks (a fine-grained token with
    /// read access to pull requests, commit statuses, and metadata).
    pub token_env: String,
    /// Seconds between two polls of each repository, 10 to 3600.
    pub poll_seconds: PollPeriod,
    /// GitHub's GraphQL endpoint.
    pub endpoint: String,
    /// The repositories watched, one per project.
    pub repository: Vec<WatchedRepository>,
    /// The GitHub App that grants relayed runs a token for their
    /// repository; none grants nothing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app: Option<GithubApp>,
}

/// The `[github.app]` section: a GitHub App installed on the watched
/// repositories, with permission to write their contents and pull
/// requests. A relayed run working in a checkout of one of them is
/// granted a token for that repository alone.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GithubApp {
    /// The App's id, from its settings page.
    pub id: u64,
    /// Its private key, as GitHub gave it (PEM).
    pub key_file: PathBuf,
    /// GitHub's REST API.
    #[serde(default = "GithubApp::api")]
    pub api: String,
}

impl GithubApp {
    fn api() -> String {
        ren_github::Installations::GITHUB.into()
    }

    /// The App's installations client, with its key read from its file.
    fn installations(&self) -> Result<ren_github::Installations, ConfigError> {
        let pem = std::fs::read_to_string(&self.key_file)
            .map_err(|error| ConfigError::GithubAppKeyFile(self.key_file.clone(), error))?;
        let key = ren_github::AppKey::from_pem(&pem)
            .map_err(|error| ConfigError::GithubAppKey(self.key_file.clone(), error))?;
        let api = Endpoint::parse(&self.api).map_err(ConfigError::GithubAppApi)?;
        ren_github::Installations::new(
            api,
            ren_github::App::new(ren_github::AppId::new(self.id), key),
        )
        .map_err(ConfigError::GithubClient)
    }
}

/// One project's repository on GitHub.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WatchedRepository {
    /// A registered project's name.
    pub project: ProjectName,
    /// Its repository on GitHub, `owner/name`.
    pub github: RepositorySlug,
}

impl Default for GithubOptions {
    fn default() -> Self {
        Self {
            token_env: "REN_GITHUB_TOKEN".into(),
            poll_seconds: PollPeriod::DEFAULT,
            endpoint: Endpoint::GITHUB.into(),
            repository: Vec::new(),
            app: None,
        }
    }
}

impl GithubOptions {
    fn is_off(&self) -> bool {
        self == &Self::default()
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        Endpoint::parse(&self.endpoint).map_err(ConfigError::GithubEndpoint)?;
        if let Some(app) = &self.app {
            Endpoint::parse(&app.api).map_err(ConfigError::GithubAppApi)?;
        }
        let mut projects = std::collections::BTreeSet::new();
        for watched in &self.repository {
            if !projects.insert(&watched.project) {
                return Err(ConfigError::GithubProjectTwice(watched.project.to_string()));
            }
        }
        Ok(())
    }

    /// What the daemon watches, with the token read from `env`; none while
    /// no repository is named.
    fn watch(&self, env: &HostEnv) -> Result<Option<GithubSetup>, ConfigError> {
        if self.repository.is_empty() {
            return Ok(None);
        }
        let token = env
            .get(&self.token_env)
            .and_then(Token::new)
            .ok_or_else(|| ConfigError::MissingGithubToken(self.token_env.clone()))?;
        let endpoint = Endpoint::parse(&self.endpoint).map_err(ConfigError::GithubEndpoint)?;
        Ok(Some(GithubSetup {
            client: Client::new(endpoint, token).map_err(ConfigError::GithubClient)?,
            period: self.poll_seconds,
            projects: self
                .repository
                .iter()
                .map(|watched| (watched.project.clone(), watched.github.clone()))
                .collect(),
            installations: self
                .app
                .as_ref()
                .map(GithubApp::installations)
                .transpose()?
                .map(std::sync::Arc::new),
        }))
    }
}

/// What the daemon watches on GitHub, resolved: the client, how often it
/// reads, and each project's repository.
#[derive(Debug, Clone)]
pub struct GithubSetup {
    pub client: Client,
    pub period: PollPeriod,
    pub projects: Vec<(ProjectName, RepositorySlug)>,
    /// The App that grants runs a token for their repository, if one is
    /// set up.
    pub installations: Option<std::sync::Arc<ren_github::Installations>>,
}

/// The `[[packages]]` entries: catalog entries in the built-in catalog's
/// form, each adding a runtime or utility Ren can install for its agents,
/// or replacing the built-in one of the same name.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(transparent)]
pub struct PackagesOptions(Vec<CatalogEntry>);

impl PackagesOptions {
    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The entries, in the order the file writes them.
    pub fn entries(&self) -> &[CatalogEntry] {
        &self.0
    }

    /// The built-in catalog with these entries.
    pub fn depot(&self) -> Result<Depot, ConfigError> {
        Ok(Depot::configured(&self.0)?)
    }
}

/// The `[[packages]]` section, renamed to `[[packages]]`: a file that still
/// has it is refused with the new name rather than read without it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RenamedTools;

impl<'de> Deserialize<'de> for RenamedTools {
    fn deserialize<D: serde::Deserializer<'de>>(_: D) -> Result<Self, D::Error> {
        Err(serde::de::Error::custom(
            "`[tools]` was renamed to `[[packages]]`: write each `[[tools.package]]` entry as `[[packages]]`",
        ))
    }
}

/// The `[runs]` section: the folders agent runs may work in. Runs are off
/// while `roots` is empty.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct RunsOptions {
    /// Absolute paths. A run may start in a root or any folder under one,
    /// after links are resolved; the projects offered are found under them.
    pub roots: Vec<PathBuf>,
    /// Other places runs may start in, offered only when asked for: each
    /// one's own folders, hidden ones included (`~/.config`). Absolute, or
    /// under the home as `~/…`.
    pub places: Vec<PathBuf>,
    /// Where scratch runs each get a new folder. Absolute or `~/…`; by
    /// default `$XDG_DATA_HOME/ren/scratch`, else `~/.local/share/ren/scratch`.
    pub scratch: Option<PathBuf>,
    /// Whether runs outlive the daemon, held by run hosts.
    pub hold: RunHold,
    /// Variables every run in a worktree gets, such as a compiler cache
    /// (`RUSTC_WRAPPER = "sccache"`). Ren's own `REN_WORKTREE*` (and their
    /// old `REN_WORKSPACE*` names) win. Read under its old name
    /// `workspace_env` too, for configs written before the rename.
    #[serde(alias = "workspace_env")]
    pub worktree_env: std::collections::BTreeMap<String, String>,
    /// What a run's container is, under `hold = "container"`.
    #[serde(skip_serializing_if = "RunContainerOptions::is_unset")]
    pub container: RunContainerOptions,
}

/// `[runs.container]`: what a run's container runs, what every run sees
/// besides what it works on and what its client keeps, and what it
/// reaches. Paths are absolute, or under the home as `~/…`, and mounted at
/// the same path.
#[serde_with::serde_as]
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct RunContainerOptions {
    /// The image a run host starts in: the instance image
    /// (`deploy/instance/Containerfile`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    /// Seen read-only: the host's programs (`/usr`), toolchains
    /// (`~/.rustup`).
    pub read_only: Vec<PathBuf>,
    /// Seen and written: caches runs share (`~/.cargo`).
    pub read_write: Vec<PathBuf>,
    /// The operator's Podman API socket, for a daemon that itself runs in a
    /// container: run containers are started through it. Without one, a
    /// daemon on the host starts them with `podman` from a systemd scope.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub podman_socket: Option<PathBuf>,
    /// What a run's container reaches: the host's network, or only Ren's
    /// gateway and its relay (docs/plans/self-build.md, phase 12).
    #[serde(skip_serializing_if = "RunNetwork::is_host")]
    pub network: RunNetwork,
    /// What runs a relayed run's processes: the host's kernel, or gVisor's.
    #[serde(skip_serializing_if = "RunRuntime::is_host_kernel")]
    pub runtime: RunRuntime,
    /// Where a relayed run may connect through the relay, besides where its
    /// client does itself: `host:port`, or `*.parent:port` for any name
    /// under one. A name reaches only public addresses.
    #[serde_as(as = "Vec<serde_with::DisplayFromStr>")]
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub reach: Vec<ren_relay::Destination>,
}

/// What a run's container reaches.
#[derive(strum::Display, strum::EnumString, strum::IntoStaticStr, strum::VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    Hash,
    serde_with::SerializeDisplay,
    serde_with::DeserializeFromStr,
)]
pub enum RunNetwork {
    /// The host's network: everything the host reaches.
    #[default]
    Host,
    /// No network but its loopback, where Ren's gateway and its relay are
    /// reached; the relay lets it out only where `reach` and its client
    /// list.
    Relayed,
}

impl RunNetwork {
    fn is_host(&self) -> bool {
        *self == Self::Host
    }
}

/// What runs a run's processes (`[runs.container] runtime`).
#[derive(strum::Display, strum::EnumString, strum::IntoStaticStr, strum::VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    Hash,
    serde_with::SerializeDisplay,
    serde_with::DeserializeFromStr,
)]
pub enum RunRuntime {
    /// Podman's own: on the host's kernel.
    #[default]
    HostKernel,
    /// gVisor (`runsc`, installed on the host): a kernel of its own in
    /// user space between the run and the host's, for relayed runs only.
    /// Through `podman_socket`, the socket must be a Podman service that
    /// runs gVisor (`ren-gvisor`, which the install sets up); a run is
    /// refused otherwise.
    Gvisor,
}

impl RunRuntime {
    fn is_host_kernel(&self) -> bool {
        *self == Self::HostKernel
    }

    fn runtime(self) -> ren_container::Runtime {
        match self {
            Self::HostKernel => ren_container::Runtime::HostKernel,
            Self::Gvisor => ren_container::Runtime::Gvisor,
        }
    }
}

/// Whether a run's client process outlives the daemon that started it.
#[derive(strum::Display, strum::EnumString, strum::IntoStaticStr, strum::VariantArray)]
#[strum(serialize_all = "snake_case")]
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    Hash,
    serde_with::SerializeDisplay,
    serde_with::DeserializeFromStr,
)]
pub enum RunHold {
    /// Held in systemd user units when the daemon itself runs under
    /// systemd on Linux and `systemd-run` is on the search path; bound to
    /// the daemon otherwise.
    #[default]
    Auto,
    /// Always held in systemd user units.
    Systemd,
    /// Held in rootless Podman containers, one per run, that see only what
    /// the run works on and what its client keeps (`[runs.container]`).
    Container,
    /// Bound to the daemon: drained as it stops.
    Off,
}

impl RunHold {
    /// How run hosts are started on a host whose environment is `env`, with
    /// `container` for runs in containers, or `None` when runs are bound to
    /// the daemon.
    pub fn launcher(
        self,
        env: &HostEnv,
        container: &RunContainerOptions,
    ) -> Result<Option<crate::host::Launcher>, ConfigError> {
        Ok(match self {
            Self::Off => None,
            Self::Systemd => Some(crate::host::Launcher::Systemd),
            Self::Container => Some(crate::host::Launcher::Container(container.containers(env)?)),
            Self::Auto => {
                let systemd = cfg!(target_os = "linux")
                    // Set by systemd for the services it runs.
                    && env.get("INVOCATION_ID").is_some()
                    && env
                        .search_path()
                        .iter()
                        .any(|dir| dir.join("systemd-run").is_file());
                systemd.then_some(crate::host::Launcher::Systemd)
            }
        })
    }
}

impl RunContainerOptions {
    /// Nothing set: the table is left out of a written configuration.
    pub fn is_unset(&self) -> bool {
        *self == Self::default()
    }

    /// How run hosts start in containers on a host whose environment is
    /// `env`: its home, and its SSH agent's socket when it has one.
    fn containers(&self, env: &HostEnv) -> Result<crate::host::RunContainers, ConfigError> {
        use ren_container::Mount;
        let image = self.image.clone().ok_or(ConfigError::RunImageMissing)?;
        let home = env
            .absolute_path("HOME")
            .ok_or(ConfigError::RunHomeMissing)?;
        let anchored = |path: &PathBuf| match path.strip_prefix("~") {
            Ok(rest) => home.join(rest),
            Err(_) => path.clone(),
        };
        let shared = self
            .read_only
            .iter()
            .map(|path| Mount::read_only(anchored(path)))
            .chain(
                self.read_write
                    .iter()
                    .map(|path| Mount::read_write(anchored(path))),
            )
            .collect();
        let reach = match &self.podman_socket {
            Some(socket) => ren_container::Reach::Socket(anchored(socket)),
            None => ren_container::Reach::Scope,
        };
        let network = match (self.network, self.runtime) {
            (RunNetwork::Host, RunRuntime::HostKernel) => crate::host::RunReach::Host,
            (RunNetwork::Host, RunRuntime::Gvisor) => {
                return Err(ConfigError::RunRuntimeNotRelayed);
            }
            (RunNetwork::Relayed, runtime) => crate::host::RunReach::Relayed(
                ren_relay::RelayPolicy::new(self.reach.clone()),
                runtime.runtime(),
            ),
        };
        Ok(crate::host::RunContainers::new(
            image,
            reach,
            home.clone(),
            shared,
            env.absolute_path("SSH_AUTH_SOCK"),
            network,
        ))
    }
}

/// The `[runs]` paths with the home resolved, as the supervisor takes them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RunPlaces {
    pub places: Vec<PathBuf>,
    pub scratch: Option<PathBuf>,
    pub home: Option<PathBuf>,
}

impl RunsOptions {
    pub fn validate(&self) -> Result<(), ConfigError> {
        let anchored = |path: &PathBuf| path.is_absolute() || path.starts_with("~");
        let names = self.worktree_env.keys().all(|name| {
            !name.is_empty()
                && !name.contains(['=', '\0'])
                && !["REN_WORKTREE", "REN_WORKSPACE"]
                    .iter()
                    .any(|own| name.starts_with(own))
        });
        if !names {
            return Err(ConfigError::WorktreeEnv);
        }
        if self.roots.iter().all(|root| root.is_absolute())
            && self
                .places
                .iter()
                .chain(&self.scratch)
                .chain(&self.container.read_only)
                .chain(&self.container.read_write)
                .chain(&self.container.podman_socket)
                .all(anchored)
        {
            Ok(())
        } else {
            Err(ConfigError::RunRoot)
        }
    }

    /// The other places and the scratch folder, `~` read as the home `env`
    /// names. A path under a home that is not known is left out.
    pub fn places_in(&self, env: &HostEnv) -> RunPlaces {
        let home = env.absolute_path("HOME");
        let anchored = |path: &PathBuf| match path.strip_prefix("~") {
            Ok(rest) => home.as_ref().map(|home| home.join(rest)),
            Err(_) => Some(path.clone()),
        };
        let scratch = match &self.scratch {
            Some(scratch) => anchored(scratch),
            None => env
                .absolute_path("XDG_DATA_HOME")
                .or_else(|| home.as_ref().map(|home| home.join(".local/share")))
                .map(|data| data.join("ren/scratch")),
        };
        RunPlaces {
            places: self.places.iter().filter_map(anchored).collect(),
            scratch,
            home,
        }
    }
}

/// An operator-key upstream entry. The key is read from the daemon's
/// environment at start, never from the file.
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct UpstreamConfig {
    pub id: UpstreamId,
    pub provider: Provider,
    /// The wire format the entry speaks; the provider's native one when
    /// omitted. Refused at load unless the provider offers it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wire: Option<Wire>,
    pub base_url: String,
    /// The environment variable holding the API key.
    pub operator_key_env: String,
    /// Whether the entry has routes of its own; fallback only when omitted.
    #[serde(default, skip_serializing_if = "EntryIngress::is_fallback_only")]
    pub ingress: EntryIngress,
    /// The named users who may use the entry; anyone when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub users: Option<Vec<UserId>>,
}

impl UpstreamConfig {
    /// Who may use the entry. Every name must be a configured user.
    fn users(&self, known: &[UserConfig]) -> Result<EntryUsers, ConfigError> {
        let Some(users) = &self.users else {
            return Ok(EntryUsers::Anyone);
        };
        if users.is_empty() {
            return Err(ConfigError::EmptyEntryUsers(self.id.to_string()));
        }
        if let Some(unknown) = users
            .iter()
            .find(|user| !known.iter().any(|config| &config.id == *user))
        {
            return Err(ConfigError::UnknownEntryUser {
                upstream: self.id.to_string(),
                user: unknown.to_string(),
            });
        }
        Ok(EntryUsers::Only(users.clone()))
    }

    /// The provider's API in the configured wire format, or its native API.
    fn api(&self) -> Result<Api, ConfigError> {
        match self.wire {
            None => Ok(self.provider.native_api()),
            Some(wire) => self
                .provider
                .speak(wire)
                .map_err(|source| ConfigError::Wire {
                    id: self.id.to_string(),
                    source,
                }),
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: 1,
            listen: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 8787),
            data_dir: None,
            anthropic_base_url: "https://api.anthropic.com".into(),
            codex_base_url: "https://chatgpt.com/backend-api/codex".into(),
            openai_base_url: "https://api.openai.com".into(),
            deepseek_base_url: "https://api.deepseek.com/anthropic".into(),
            max_concurrent_requests: ConcurrencyLimit::saturating(32),
            max_concurrent_unnamed: None,
            connect_timeout_seconds: TimeoutSecs::saturating(10),
            response_header_timeout_seconds: TimeoutSecs::saturating(120),
            stream_idle_timeout_seconds: TimeoutSecs::saturating(120),
            capture: CaptureOptions::default(),
            retention: RetentionPolicy::default(),
            websocket: WebSocketOptions::default(),
            upstreams: Vec::new(),
            buffer: BufferOptions::default(),
            policy: PolicyConfig::default(),
            control: ControlOptions::default(),
            users: Vec::new(),
            instance: InstanceOptions::default(),
            runs: RunsOptions::default(),
            attachments: AttachmentOptions::default(),
            packages: PackagesOptions::default(),
            tools: RenamedTools,
            github: GithubOptions::default(),
        }
    }
}

impl Config {
    /// Resolve the editable file representation into an immutable runtime
    /// snapshot, with no environment: configured operator keys are an error.
    /// Keep `load` usable by remote diagnostics and dry-run launchers, which do
    /// not use this host's upstream settings.
    pub fn resolve(&self) -> Result<RuntimeConfig, ConfigError> {
        self.resolve_with(&HostEnv::default())
    }

    /// Everything [`Self::resolve_with`] checks, except that operator keys
    /// are present: for installing a service whose environment differs.
    pub fn check(&self) -> Result<(), ConfigError> {
        self.build(Keys::Unchecked).map(drop)
    }

    /// Resolve with operator keys read from `env`.
    pub fn resolve_with(&self, env: &HostEnv) -> Result<RuntimeConfig, ConfigError> {
        self.build(Keys::Env(env))
    }

    fn key(upstream: &UpstreamConfig, keys: Keys<'_>) -> Result<OperatorKey, ConfigError> {
        let name = &upstream.operator_key_env;
        let text = match keys {
            Keys::Unchecked => "unchecked",
            Keys::Env(env) => env.get(name).ok_or_else(|| ConfigError::MissingKey {
                name: name.clone(),
                upstream: upstream.id.to_string(),
            })?,
        };
        OperatorKey::new(text).ok_or_else(|| ConfigError::InvalidKey(name.clone()))
    }

    fn build(&self, keys: Keys<'_>) -> Result<RuntimeConfig, ConfigError> {
        self.validate()?;
        let extra = self
            .upstreams
            .iter()
            .map(|upstream| {
                if upstream.provider == Provider::Codex {
                    return Err(ConfigError::CodexOperatorKey(upstream.id.to_string()));
                }
                let entry = UpstreamEntry {
                    id: upstream.id.clone(),
                    api: upstream.api()?,
                    url: upstream.base_url.parse::<UpstreamUrl>().map_err(|source| {
                        ConfigError::UpstreamUrl {
                            id: upstream.id.to_string(),
                            source,
                        }
                    })?,
                    auth: UpstreamAuth::OperatorKey(Self::key(upstream, keys)?),
                };
                Ok(ConfiguredEntry {
                    entry,
                    ingress: upstream.ingress,
                    users: upstream.users(&self.users)?,
                })
            })
            .collect::<Result<Vec<_>, ConfigError>>()?;
        let upstreams = UpstreamTable::new(
            NativeUrls {
                anthropic: self.anthropic_base_url.parse::<UpstreamUrl>()?,
                codex: self.codex_base_url.parse::<UpstreamUrl>()?,
                openai: self.openai_base_url.parse::<UpstreamUrl>()?,
                deepseek: self.deepseek_base_url.parse::<UpstreamUrl>()?,
            },
            extra,
        )?;
        let policy = Policy::new(self.policy.clone(), &upstreams.summaries())?;
        let users = Users::new(self.users.clone())?;
        let depot = self.packages.depot()?;
        Ok(RuntimeConfig {
            listen: self.listen,
            upstreams,
            policy,
            users,
            instance: self.instance,
            max_concurrent_requests: self.max_concurrent_requests,
            max_concurrent_unnamed: self.max_concurrent_unnamed,
            connect_timeout: self.connect_timeout_seconds.duration(),
            response_header_timeout: self.response_header_timeout_seconds.duration(),
            stream_idle_timeout: self.stream_idle_timeout_seconds.duration(),
            capture: self.capture.clone(),
            retention: self.retention.clone(),
            websocket: self.websocket,
            buffer: self.buffer,
            control: self.control.clone(),
            runs: self.runs.clone(),
            attachment_limits: self.attachments.limits()?,
            run_places: match keys {
                Keys::Env(env) => self.runs.places_in(env),
                Keys::Unchecked => RunPlaces::default(),
            },
            pi_cache: match keys {
                Keys::Env(env) => PiCache::locate(env).ok(),
                Keys::Unchecked => None,
            },
            agent_path: match keys {
                Keys::Env(env) => env.search_path(),
                Keys::Unchecked => Vec::new(),
            },
            run_hold: match keys {
                Keys::Env(env) => self.runs.hold.launcher(env, &self.runs.container)?,
                Keys::Unchecked => None,
            },
            depot,
            github: match keys {
                Keys::Env(env) => self.github.watch(env)?,
                Keys::Unchecked => None,
            },
            revision: self.revision()?,
        })
    }

    pub fn revision(&self) -> Result<String, ConfigError> {
        Ok(
            Sha256::digest(serde_json::to_vec(self).map_err(ConfigError::Revision)?)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect(),
        )
    }

    pub fn load(path: Option<&Path>) -> Result<Self, ConfigError> {
        let mut config: Self = match path {
            Some(path) => {
                let contents = std::fs::read_to_string(path).map_err(ConfigError::Read)?;
                // TOML diagnostics can echo credential-bearing invalid values.
                toml::from_str(&contents).map_err(|_| ConfigError::Syntax)?
            }
            None => Self::default(),
        };
        if let (Some(path), Some(dir)) = (path, config.data_dir.as_mut())
            && dir.is_relative()
        {
            *dir = path.parent().unwrap_or(Path::new(".")).join(&*dir);
        }
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.version != 1 {
            return Err(ConfigError::Version);
        }
        if !self.listen.ip().is_loopback() {
            return Err(ConfigError::PublicListener);
        }
        self.capture.validate()?;
        self.retention.validate()?;
        self.buffer.validate()?;
        self.runs.validate()?;
        self.packages.depot()?;
        self.github.validate()?;
        Ok(())
    }

    /// The data directory: the override, then `data_dir`, then
    /// `$XDG_STATE_HOME/ren`, then `~/.local/state/ren`.
    pub fn resolve_data_dir(
        &self,
        override_dir: Option<PathBuf>,
        env: &HostEnv,
    ) -> Result<PathBuf, ConfigError> {
        if let Some(dir) = override_dir.or_else(|| self.data_dir.clone()) {
            return Ok(dir);
        }
        if let Some(dir) = env.absolute_path("XDG_STATE_HOME") {
            return Ok(dir.join("ren"));
        }
        env.absolute_path("HOME")
            .map(|home| home.join(".local/state/ren"))
            .ok_or(ConfigError::NoDataDir)
    }
}

/// Validated settings consumed by daemon assembly. No mutable access or
/// deserialization bypass: all instances are produced by `Config::resolve`.
#[derive(Clone)]
pub struct RuntimeConfig {
    listen: SocketAddr,
    upstreams: UpstreamTable,
    policy: Policy,
    users: Users,
    instance: InstanceOptions,
    max_concurrent_requests: ConcurrencyLimit,
    max_concurrent_unnamed: Option<ConcurrencyLimit>,
    connect_timeout: Duration,
    response_header_timeout: Duration,
    stream_idle_timeout: Duration,
    capture: CaptureOptions,
    retention: RetentionPolicy,
    websocket: WebSocketOptions,
    buffer: BufferOptions,
    control: ControlOptions,
    runs: RunsOptions,
    attachment_limits: Limits,
    /// The runs' other places and scratch folder, with the home resolved.
    run_places: RunPlaces,
    /// Where driven Pi processes find Ren's extension, when the host
    /// environment names a cache.
    pi_cache: Option<PiCache>,
    /// The host's search path, which a run's package set folder is put before.
    agent_path: Vec<PathBuf>,
    /// How run hosts are started; `None` binds runs to the daemon.
    run_hold: Option<crate::host::Launcher>,
    /// The packages Ren knows: the built-in catalog with `[[packages]]`.
    depot: Depot,
    /// What is watched on GitHub; none while nothing is.
    github: Option<GithubSetup>,
    revision: String,
}

impl RuntimeConfig {
    pub fn listen(&self) -> SocketAddr {
        self.listen
    }
    pub fn upstreams(&self) -> &UpstreamTable {
        &self.upstreams
    }
    pub fn control(&self) -> &ControlOptions {
        &self.control
    }
    pub fn instance(&self) -> InstanceOptions {
        self.instance
    }
    /// How much a session's attachments may hold.
    pub fn attachment_limits(&self) -> Limits {
        self.attachment_limits
    }

    pub fn github(&self) -> Option<&GithubSetup> {
        self.github.as_ref()
    }

    pub fn runs(&self) -> &RunsOptions {
        &self.runs
    }
    pub fn run_places(&self) -> &RunPlaces {
        &self.run_places
    }
    pub fn agent_path(&self) -> &[PathBuf] {
        &self.agent_path
    }
    /// How run hosts are started; `None` binds runs to the daemon.
    pub fn run_hold(&self) -> Option<&crate::host::Launcher> {
        self.run_hold.as_ref()
    }
    pub fn depot(&self) -> &Depot {
        &self.depot
    }

    pub fn pi_cache(&self) -> Option<&PiCache> {
        self.pi_cache.as_ref()
    }
    /// The gateway's settings from this snapshot.
    pub fn gateway(&self) -> GatewayConfig {
        GatewayConfig {
            max_concurrent_requests: self.max_concurrent_requests,
            response_header_timeout: self.response_header_timeout,
            stream_idle_timeout: self.stream_idle_timeout,
            config_revision: ConfigRevision::new(self.revision.as_str()),
            websocket: self.websocket,
            buffer: self.buffer,
            policy: self.policy.clone(),
            upstreams: self.upstreams.clone(),
            users: self.users.clone(),
            max_concurrent_unnamed: self.max_concurrent_unnamed,
        }
    }
    /// Lifecycle units the recorder reserves at once: one per attempt every
    /// admitted call's plan allows, each channel's end, and a margin for
    /// calls whose response is still draining after their admission
    /// returned.
    pub fn recorder_calls(&self) -> usize {
        self.max_concurrent_requests
            .get()
            .saturating_mul(4)
            .saturating_mul(ren_gateway::policy::Candidates::MAX)
            .saturating_add(self.websocket.max_channels.get().saturating_mul(2))
    }
    pub fn transport(&self) -> TransportTimeouts {
        TransportTimeouts {
            connect: self.connect_timeout,
            idle: self.stream_idle_timeout,
        }
    }
    pub fn max_concurrent_requests(&self) -> usize {
        self.max_concurrent_requests.get()
    }
    pub fn connect_timeout(&self) -> Duration {
        self.connect_timeout
    }
    pub fn response_header_timeout(&self) -> Duration {
        self.response_header_timeout
    }
    pub fn stream_idle_timeout(&self) -> Duration {
        self.stream_idle_timeout
    }
    pub fn capture(&self) -> &CaptureOptions {
        &self.capture
    }
    pub fn retention(&self) -> &RetentionPolicy {
        &self.retention
    }
    pub fn revision(&self) -> &str {
        &self.revision
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unknown_keys_and_public_listener() {
        assert!(toml::from_str::<Config>("max_concurrent_request = 10").is_err());
        let config = Config {
            listen: "0.0.0.0:8787".parse().unwrap(),
            ..Config::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn package_entries_join_the_builtin_catalog_and_bad_ones_are_refused_at_load() {
        let entry =
            "[[packages]]\nid = \"acme-lint\"\nkind = \"utility\"\nprogram = \"acme-lint\"\n\
                     [packages.release]\nversion = \"0.3.0\"\n\
                     [[packages.release.artifact]]\nplatform = \"linux_x64\"\n\
                     url = \"https://example.test/acme-lint.tar.gz\"\nsha256 = \"{digest}\"\n\
                     format = \"tar_gz\"\nprogram_at = \"acme-lint\"\n"
                .replace("{digest}", &"a".repeat(64));
        let config: Config = toml::from_str(&entry).unwrap();
        config.validate().unwrap();
        let depot = config.resolve().unwrap().depot().clone();
        let acme = depot.package(&"acme-lint".parse().unwrap()).unwrap();
        assert_eq!(acme.origin(), ren_depot::package::Origin::Operator);
        // The section round-trips, and is left out of the revision when empty.
        let written: Config = toml::from_str(&toml::to_string(&config).unwrap()).unwrap();
        assert_eq!(written.packages, config.packages);
        let default = toml::to_string(&Config::default()).unwrap();
        assert!(!default.contains("packages") && !default.contains("tools"));

        let client = "[[packages]]\nid = \"codex\"\nkind = \"utility\"\nprogram = \"codex\"\n";
        let config: Config = toml::from_str(client).unwrap();
        assert!(matches!(
            config.validate(),
            Err(ConfigError::Packages(ren_depot::CatalogError::Client(_)))
        ));
        assert!(toml::from_str::<Config>(&entry.replace("https://", "http://")).is_err());

        // The section's old name is refused by its new one.
        let old = entry
            .replace("[[packages]]", "[[tools.package]]")
            .replace("[packages.", "[tools.package.");
        let error = toml::from_str::<Config>(&old).err().unwrap().to_string();
        assert!(error.contains("renamed to `[[packages]]`"), "{error}");
    }

    #[test]
    fn worktree_variables_may_not_be_rens_own_or_unnameable() {
        for names in [
            "REN_WORKTREE_PORTS = \"1\"",
            "REN_WORKSPACE = \"1\"",
            "\"\" = \"x\"",
            "\"A=B\" = \"x\"",
        ] {
            let config: Config = toml::from_str(&format!(
                "[runs]\nroots = [\"/home/op/dev\"]\nworktree_env = {{ {names} }}\n"
            ))
            .unwrap();
            assert!(
                matches!(config.runs.validate(), Err(ConfigError::WorktreeEnv)),
                "{names}"
            );
        }
        let config: Config = toml::from_str(
            "[runs]\nroots = [\"/home/op/dev\"]\nworktree_env = { RUSTC_WRAPPER = \"sccache\" }\n",
        )
        .unwrap();
        assert!(config.runs.validate().is_ok());
    }

    #[test]
    fn worktree_variables_are_read_under_their_old_key() {
        let config: Config = toml::from_str(
            "[runs]\nroots = [\"/home/op/dev\"]\nworkspace_env = { RUSTC_WRAPPER = \"sccache\" }\n",
        )
        .unwrap();
        assert_eq!(
            config
                .runs
                .worktree_env
                .get("RUSTC_WRAPPER")
                .map(String::as_str),
            Some("sccache")
        );
        assert!(config.runs.validate().is_ok());
    }

    #[test]
    fn run_places_and_scratch_read_the_home() {
        let config: Config = toml::from_str(
            "[runs]\nroots = [\"/home/op/dev\"]\nplaces = [\"~/.config\", \"/srv/dotfiles\"]\n",
        )
        .unwrap();
        let env = HostEnv::from_pairs([("HOME", "/home/op")]);
        let places = config.runs.places_in(&env);
        assert_eq!(
            places.places,
            [
                PathBuf::from("/home/op/.config"),
                PathBuf::from("/srv/dotfiles")
            ]
        );
        assert_eq!(
            places.scratch,
            Some(PathBuf::from("/home/op/.local/share/ren/scratch"))
        );
        let env = HostEnv::from_pairs([("HOME", "/home/op"), ("XDG_DATA_HOME", "/data")]);
        assert_eq!(
            config.runs.places_in(&env).scratch,
            Some(PathBuf::from("/data/ren/scratch"))
        );
        // Without a home, a place under it is left out.
        let homeless = config.runs.places_in(&HostEnv::from_pairs([]));
        assert_eq!(homeless.places, [PathBuf::from("/srv/dotfiles")]);
        let relative: Config = toml::from_str("[runs]\nplaces = [\"dotfiles\"]\n").unwrap();
        assert!(relative.runs.validate().is_err());
    }

    #[test]
    fn runs_in_containers_need_an_image_and_see_paths_under_the_home() {
        let env = HostEnv::from_pairs([("HOME", "/home/op"), ("SSH_AUTH_SOCK", "/run/agent")]);
        let without: Config = toml::from_str("[runs]\nhold = \"container\"\n").unwrap();
        assert!(matches!(
            without.runs.hold.launcher(&env, &without.runs.container),
            Err(ConfigError::RunImageMissing)
        ));
        let config: Config = toml::from_str(
            "[runs]\nhold = \"container\"\n[runs.container]\nimage = \"localhost/ren-instance:abc\"\nread_only = [\"/usr\", \"~/.rustup\"]\nread_write = [\"~/.cargo\"]\n",
        )
        .unwrap();
        assert!(config.runs.validate().is_ok());
        let Ok(Some(crate::host::Launcher::Container(containers))) =
            config.runs.hold.launcher(&env, &config.runs.container)
        else {
            panic!("runs are held in containers");
        };
        use crate::host::RunContainers;
        use ren_container::{Mount, Reach};
        assert_eq!(
            containers,
            RunContainers::new(
                "localhost/ren-instance:abc".into(),
                Reach::Scope,
                "/home/op".into(),
                vec![
                    Mount::read_only("/usr"),
                    Mount::read_only("/home/op/.rustup"),
                    Mount::read_write("/home/op/.cargo"),
                ],
                Some("/run/agent".into()),
                crate::host::RunReach::Host,
            )
        );
        let relative: Config = toml::from_str("[runs.container]\nread_only = [\"usr\"]\n").unwrap();
        assert!(relative.runs.validate().is_err());
    }

    #[test]
    fn relayed_runs_run_on_the_hosts_kernel_unless_gvisor_is_chosen() {
        let config: Config = toml::from_str(
            "[runs]\nhold = \"container\"\n[runs.container]\nimage = \"i\"\nnetwork = \"relayed\"\nruntime = \"gvisor\"\n",
        )
        .unwrap();
        assert_eq!(config.runs.container.runtime, RunRuntime::Gvisor);
        let env = HostEnv::from_pairs([("HOME", "/home/op")]);
        let Ok(Some(crate::host::Launcher::Container(containers))) =
            config.runs.hold.launcher(&env, &config.runs.container)
        else {
            panic!("runs are held in containers");
        };
        assert!(matches!(
            containers.network(),
            crate::host::RunReach::Relayed(_, ren_container::Runtime::Gvisor)
        ));
        // gVisor keeps only its loopback: a run on the host's network
        // cannot have it.
        let unrelayed: Config = toml::from_str(
            "[runs]\nhold = \"container\"\n[runs.container]\nimage = \"i\"\nruntime = \"gvisor\"\n",
        )
        .unwrap();
        assert!(matches!(
            unrelayed
                .runs
                .hold
                .launcher(&env, &unrelayed.runs.container),
            Err(ConfigError::RunRuntimeNotRelayed)
        ));
        let written = toml::to_string(&config.runs.container).unwrap();
        assert!(written.contains("runtime = \"gvisor\""), "{written}");
        assert_eq!(
            RunContainerOptions::default().runtime,
            RunRuntime::HostKernel
        );
        assert!(
            !toml::to_string(&RunContainerOptions::default())
                .unwrap()
                .contains("runtime")
        );
        assert!(toml::from_str::<Config>("[runs.container]\nruntime = \"kata\"\n").is_err());
    }

    #[test]
    fn relayed_runs_reach_what_their_configuration_lists() {
        let env = HostEnv::from_pairs([("HOME", "/home/op")]);
        let config: Config = toml::from_str(
            "[runs]\nhold = \"container\"\n[runs.container]\nimage = \"i\"\nnetwork = \"relayed\"\nreach = [\"index.crates.io:443\", \"*.npmjs.org:443\"]\n",
        )
        .unwrap();
        let Ok(Some(crate::host::Launcher::Container(containers))) =
            config.runs.hold.launcher(&env, &config.runs.container)
        else {
            panic!("runs are held in containers");
        };
        let crate::host::RunReach::Relayed(policy, ren_container::Runtime::HostKernel) =
            containers.network()
        else {
            panic!("its runs are relayed");
        };
        assert_eq!(
            policy
                .destinations()
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            ["index.crates.io:443", "*.npmjs.org:443"]
        );
        // Written back as it reads; a host network is the default, left out.
        let written = toml::to_string(&config.runs.container).unwrap();
        assert!(written.contains("network = \"relayed\""), "{written}");
        assert!(written.contains("\"*.npmjs.org:443\""), "{written}");
        assert!(
            !toml::to_string(&RunContainerOptions::default())
                .unwrap()
                .contains("network")
        );
        for bad in [
            "crates.io",
            "*.crates.io",
            "crates.io:0",
            "https://crates.io",
        ] {
            let text = format!("[runs.container]\nreach = [\"{bad}\"]\n");
            assert!(toml::from_str::<Config>(&text).is_err(), "{bad}");
        }
    }

    #[test]
    fn every_clients_reach_reads_as_destinations() {
        use ren_protocol::label::Labeled as _;
        for harness in ren_harness::Harness::ALL {
            for reach in harness.profile().needs.reaches {
                assert!(
                    reach.parse::<ren_relay::Destination>().is_ok(),
                    "{harness}: {reach}"
                );
            }
        }
    }

    #[test]
    fn runs_need_absolute_roots() {
        let config: Config = toml::from_str("[runs]\nroots = [\"/home/op/dev\"]\n").unwrap();
        config.resolve().unwrap();
        let config: Config = toml::from_str("[runs]\nroots = [\"dev\"]\n").unwrap();
        assert!(matches!(config.validate(), Err(ConfigError::RunRoot)));
        assert!(
            Config::default().runs.roots.is_empty(),
            "runs are off by default"
        );
    }

    #[test]
    fn example_is_valid() {
        let config: Config = toml::from_str(include_str!("../../../ren.example.toml")).unwrap();
        config.resolve().unwrap();
    }

    #[test]
    fn users_parse_and_a_duplicate_login_is_refused() {
        let digest = "ab".repeat(32);
        let config: Config = toml::from_str(&format!(
            "[[users]]\nid = \"alice\"\ntailscale_logins = [\"alice@example.com\"]\n\
             [[users]]\nid = \"ci\"\ntoken_blake3 = \"{digest}\"\n"
        ))
        .unwrap();
        config.resolve().unwrap();
        let config: Config = toml::from_str(
            "[[users]]\nid = \"a\"\ntailscale_logins = [\"x@y\"]\n\
             [[users]]\nid = \"b\"\ntailscale_logins = [\"x@y\"]\n",
        )
        .unwrap();
        assert_eq!(
            config
                .resolve()
                .err()
                .map(|error| error.to_string())
                .as_deref(),
            Some("Tailscale login x@y belongs to two users")
        );
        assert!(
            toml::from_str::<Config>("[[users]]\nid = \"Alice\"\ntailscale_logins = [\"a@b\"]\n")
                .is_err()
        );
        let entry = |users: &str| -> Option<String> {
            let config: Config = toml::from_str(&format!(
                "[[users]]\nid = \"alice\"\ntailscale_logins = [\"a@b\"]\n\
                 [[upstreams]]\nid = \"key\"\nprovider = \"anthropic\"\n\
                 base_url = \"https://api.anthropic.com\"\noperator_key_env = \"K\"\n{users}"
            ))
            .unwrap();
            config.check().err().map(|error| error.to_string())
        };
        assert_eq!(entry("users = [\"alice\"]"), None);
        assert_eq!(
            entry("users = [\"bob\"]").as_deref(),
            Some("upstream key: users names bob, which no [[users]] entry is")
        );
        assert!(entry("users = []").is_some());
    }

    #[test]
    fn limits_and_timeouts_out_of_range_are_refused_as_the_file_loads() {
        for (line, expected) in [
            ("max_concurrent_requests = 0", "between 1 and 1024"),
            ("max_concurrent_unnamed = 1025", "between 1 and 1024"),
            ("connect_timeout_seconds = 0", "between 1 and 3600"),
            ("stream_idle_timeout_seconds = 3601", "between 1 and 3600"),
            ("[buffer]\nidle_timeout_seconds = 0", "between 1 and 3600"),
            ("[websocket]\nmax_channels = 0", "between 1 and 1024"),
            (
                "[[users]]\nid = \"a\"\ntailscale_logins = [\"a@b\"]\nmax_concurrent = 2000",
                "between 1 and 1024",
            ),
        ] {
            let error = toml::from_str::<Config>(line).err().unwrap().to_string();
            assert!(error.contains(expected), "{line}: {error}");
        }
        let config = toml::from_str::<Config>(
            "max_concurrent_requests = 1024\nconnect_timeout_seconds = 3600",
        )
        .unwrap();
        assert_eq!(config.max_concurrent_requests.get(), 1024);
        assert_eq!(
            config.connect_timeout_seconds.duration(),
            Duration::from_secs(3600)
        );
    }

    #[test]
    fn runtime_snapshot_is_independent_of_later_document_edits() {
        let mut config = Config::default();
        let revision = config.revision().unwrap();
        let runtime = config.resolve().unwrap();
        config.listen = "0.0.0.0:1".parse().unwrap();
        config.max_concurrent_requests = ConcurrencyLimit::saturating(2);
        config.connect_timeout_seconds = TimeoutSecs::saturating(1);
        config.capture.queue_bytes = 0;
        config.retention.max_calls = 0;
        config.codex_base_url = "http://example.com".into();
        assert!(config.resolve().is_err());
        assert_eq!(runtime.listen(), "127.0.0.1:8787".parse().unwrap());
        assert_eq!(runtime.max_concurrent_requests(), 32);
        assert_eq!(runtime.connect_timeout(), Duration::from_secs(10));
        assert_eq!(runtime.capture().queue_bytes, 64 * 1024 * 1024);
        assert_eq!(runtime.retention().max_calls, 10_000);
        assert_eq!(runtime.revision(), revision);
    }

    #[test]
    fn runtime_resolution_validates_upstreams_without_echoing_invalid_values() {
        for base in [
            "https://user:SECRET@example.com",
            "https://example.com?token=SECRET",
            "https://example.com/#SECRET",
            "http://example.com",
        ] {
            let config = Config {
                openai_base_url: base.into(),
                ..Config::default()
            };
            // Remote diagnostics and launch dry-runs don't consume local upstreams.
            assert!(config.validate().is_ok());
            let error = config.resolve().err().unwrap();
            assert!(!format!("{error:?}").contains("SECRET"));
        }
    }

    fn with_operator_key(policy: &str) -> Config {
        Config {
            upstreams: vec![UpstreamConfig {
                id: "anthropic-key".parse::<UpstreamId>().unwrap(),
                provider: Provider::Anthropic,
                wire: None,
                base_url: "https://api.anthropic.com".into(),
                operator_key_env: "REN_FIXTURE_KEY".into(),
                ingress: EntryIngress::FallbackOnly,
                users: None,
            }],
            policy: toml::from_str(policy).unwrap(),
            ..Config::default()
        }
    }

    #[test]
    fn operator_keys_come_from_the_environment_and_never_appear_in_errors() {
        let config = with_operator_key("");
        let missing = format!("{:#}", config.resolve().err().unwrap());
        assert!(missing.contains("REN_FIXTURE_KEY"), "{missing}");
        // Structure is checked without the key, for service installs.
        config.check().unwrap();
        let env = HostEnv::from_pairs([("REN_FIXTURE_KEY", "sk-SECRET")]);
        let runtime = config.resolve_with(&env).unwrap();
        let entry = runtime
            .upstreams()
            .get(&"anthropic-key".parse::<UpstreamId>().unwrap())
            .unwrap();
        assert!(!format!("{entry:?}").contains("SECRET"));
        let invalid = HostEnv::from_pairs([("REN_FIXTURE_KEY", "sk-\nSECRET")]);
        let error = format!("{:#}", config.resolve_with(&invalid).err().unwrap());
        assert!(!error.contains("SECRET"), "{error}");
    }

    #[test]
    fn unsafe_upstreams_and_protected_transforms_are_refused_at_load() {
        let mut codex = with_operator_key("");
        codex.upstreams[0].provider = Provider::Codex;
        assert!(codex.check().is_err());
        let mut reserved = with_operator_key("");
        reserved.upstreams[0].id = "anthropic".parse::<UpstreamId>().unwrap();
        assert!(reserved.check().is_err());
        // Loading checks the policy; its rules are tested with the policy.
        let spoof = "[[transform]]\nid = \"bad\"\nmatch = { provider = \"anthropic\" }\nsteps = [{ op = \"set_header\", name = \"user-agent\", value = \"spoof\" }]\nreason = \"fixture\"\n";
        assert!(with_operator_key(spoof).check().is_err());
    }

    #[test]
    fn an_upstream_speaks_its_providers_native_wire_unless_one_it_offers_is_named() {
        let key = "anthropic-key".parse::<UpstreamId>().unwrap();
        let env = HostEnv::from_pairs([("REN_FIXTURE_KEY", "sk-FIXTURE")]);
        let api = |config: &Config| {
            config
                .resolve_with(&env)
                .unwrap()
                .upstreams()
                .get(&key)
                .unwrap()
                .api
        };
        let omitted = with_operator_key("");
        assert_eq!(api(&omitted), Provider::Anthropic.native_api());

        let mut named = with_operator_key("");
        named.upstreams[0].wire = Some(Wire::AnthropicMessages);
        assert_eq!(api(&named), Provider::Anthropic.native_api());

        let mut unsupported = with_operator_key("");
        unsupported.upstreams[0].wire = Some(Wire::OpenAiResponses);
        let error = unsupported.check().err().unwrap().to_string();
        assert_eq!(
            error,
            "upstream anthropic-key: provider anthropic does not offer the openai_responses wire format"
        );
    }

    #[test]
    fn optional_upstream_keys_read_labels_and_omitted_ones_are_not_written() {
        let entry = |wire: &str| {
            toml::from_str::<UpstreamConfig>(&format!(
                "id = \"k\"\nprovider = \"openai\"\n{wire}base_url = \"https://api.openai.com\"\noperator_key_env = \"K\"\n"
            ))
        };
        assert_eq!(
            entry("wire = \"openai_responses\"\n").unwrap().wire,
            Some(Wire::OpenAiResponses)
        );
        assert_eq!(
            entry("wire = \"openai_chat_completions\"\n").unwrap().wire,
            Some(Wire::OpenAiChatCompletions)
        );
        assert!(entry("wire = \"chat\"\n").is_err());
        let omitted = entry("").unwrap();
        assert_eq!(omitted.wire, None);
        // A configuration without the key keeps the revision it had.
        assert!(!serde_json::to_string(&omitted).unwrap().contains("wire"));
        let anthropic = |ingress: &str| {
            toml::from_str::<UpstreamConfig>(&format!(
                "id = \"k\"\nprovider = \"anthropic\"\nbase_url = \"https://api.anthropic.com\"\noperator_key_env = \"K\"\n{ingress}"
            ))
        };
        let omitted_ingress = anthropic("").unwrap();
        assert_eq!(omitted_ingress.ingress, EntryIngress::FallbackOnly);
        assert!(
            !serde_json::to_string(&omitted_ingress)
                .unwrap()
                .contains("ingress")
        );
        assert_eq!(
            anthropic("ingress = \"direct\"\n").unwrap().ingress,
            EntryIngress::Direct
        );
        assert!(anthropic("ingress = \"public\"\n").is_err());
    }
}
