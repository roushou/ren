//! A dev instance (docs/plans/instances.md, phase 5): a checkout's build
//! running beside the installed service, with its own configuration, data,
//! and loopback ports from its [`DevIndex`], pointed only at the fake
//! provider. Each process runs as a transient systemd user unit named for
//! the instance (`ren-dev-*` for the first, `ren-dev-<index>-*` after), so
//! it is stopped by name and never by a pattern that could match the
//! installed service or another instance.

use super::{DevError, claims::DevClaims, index::DevIndex};
use ren_container::Engine;
use ren_daemon::config::{
    Config, DeployerOptions, GithubApp, GithubOptions, RelayedOptions, RunContainerOptions,
    RunHold, RunRuntime, RunsOptions, UpstreamConfig, WatchedRepository,
};
use ren_daemon::host::HeldRuns;
use ren_deployer::file::{FileError, FileMode};
use ren_deployer::gvisor_service::GvisorService;
use ren_deployer::progress::Progress;
use ren_deployer::{
    daemon::{Busy, Ending, RunPolicy, Runs, StillBusy},
    daemon_container::{Contain, DaemonContainer},
    invocation::{Echo, Invocation, InvocationError},
    pod::Pod,
    tailscale::{Tailnet, TailscaleError},
    ui::{UiBuild, UiContainer},
    unit::{KillMode, Listener, TransientUnit},
};
use ren_gateway::{
    ControlToken, EntryIngress, Environment, InstanceOptions,
    user::{TokenDigest, UserConfig},
};
use ren_git::{Author, Git, Repository};
use ren_harness::{endpoint::UserToken, host::HostEnv};
use ren_network::tailscale::Login;
use ren_protocol::route::{Provider, Wire};
use ren_recorder::{caller::UserId, routing::UpstreamId};
use std::{
    fs,
    net::SocketAddr,
    path::{Path, PathBuf},
    thread,
    time::Duration,
};

/// One process of a dev instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    Provider,
    Daemon,
    Ui,
    Traffic,
}

impl Unit {
    /// Every unit, in the order they stop.
    pub const ALL: [Self; 4] = [Self::Traffic, Self::Ui, Self::Daemon, Self::Provider];

    /// The transient unit of `index`'s instance, named for it.
    pub fn unit(self, index: DevIndex) -> TransientUnit {
        let process = match self {
            Self::Provider => "provider",
            Self::Daemon => "daemon",
            Self::Ui => "ui",
            Self::Traffic => "traffic",
        };
        let name = format!("{}-{process}", index.name());
        // The daemon is signalled alone, as the installed unit does; the
        // rest stop as a whole.
        let kill = match self {
            Self::Daemon => KillMode::Mixed,
            Self::Provider | Self::Ui | Self::Traffic => KillMode::ControlGroup,
        };
        TransientUnit {
            description: format!("Ren dev instance: {name}"),
            name,
            kill,
        }
    }

    pub fn name(self, index: DevIndex) -> String {
        self.unit(index).name
    }

    pub fn active(self, index: DevIndex) -> Result<bool, InvocationError> {
        self.unit(index).active()
    }

    pub fn stop(self, index: DevIndex) -> Result<(), InvocationError> {
        self.unit(index).stop()
    }

    /// Run `command` as this unit of `index`'s instance, with
    /// `environment`, until stopped.
    pub fn start(
        self,
        index: DevIndex,
        dir: Option<&Path>,
        command: &[&Path],
        args: &[String],
        environment: &[(&str, String)],
    ) -> Result<(), InvocationError> {
        self.unit(index).start(dir, command, args, environment)
    }
}

/// How `dev up` restarts a dev daemon that drives runs: it waits up to
/// `timeout` seconds for runs mid-way, unless `policy` drains them.
#[derive(Debug, Clone, Copy)]
pub struct RunWait {
    pub policy: RunPolicy,
    pub timeout: u64,
}

/// Where the dev preview is reached.
#[derive(Debug, Clone, Copy)]
pub enum Preview {
    /// Published on the tailnet with Tailscale Serve.
    Tailnet(Tailnet),
    /// Only on this host's loopback.
    Local,
}

/// The host's [`DevClaims`], in their file, changed under a lock so two
/// checkouts starting at once never take the same instance.
pub struct ClaimsFile {
    path: PathBuf,
}

impl ClaimsFile {
    pub fn new(env: &HostEnv) -> Result<Self, DevError> {
        let home = env
            .absolute_path("HOME")
            .ok_or(DevError::MissingEnv("HOME"))?;
        Ok(Self {
            path: home.join(".local/state").join(DevClaims::FILE),
        })
    }

    /// The claims as they stand; none before the first.
    pub fn read(&self) -> Result<DevClaims, DevError> {
        match fs::read(&self.path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|error| DevError::Claims {
                path: self.path.clone(),
                why: error.to_string(),
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(DevClaims::default()),
            Err(error) => Err(FileError::at(&self.path)(error).into()),
        }
    }

    /// Changes the claims with `change`, under the file's lock.
    pub fn change<T>(
        &self,
        change: impl FnOnce(DevClaims) -> Result<(DevClaims, T), DevError>,
    ) -> Result<T, DevError> {
        let dir = self.path.parent().unwrap_or(Path::new("/"));
        fs::create_dir_all(dir).map_err(FileError::at(dir))?;
        let lock_path = self.path.with_extension("lock");
        let lock = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&lock_path)
            .map_err(FileError::at(&lock_path))?;
        lock.lock().map_err(FileError::at(&lock_path))?;
        let (claims, answer) = change(self.read()?)?;
        let text = serde_json::to_string_pretty(&claims).map_err(|error| DevError::Claims {
            path: self.path.clone(),
            why: error.to_string(),
        })?;
        FileMode::Private.write(&self.path, text.as_bytes())?;
        Ok(answer)
    }
}

/// Where a dev instance lives and listens.
pub struct DevInstance {
    pub index: DevIndex,
    pub repo: PathBuf,
    cargo: PathBuf,
    pub config: PathBuf,
    pub data_dir: PathBuf,
    pub bin_dir: PathBuf,
    ui_dir: PathBuf,
    ui_next: PathBuf,
    /// The dev daemon's run root, with a few fake projects to start runs in.
    pub work_dir: PathBuf,
    /// Its other place, with a few fake folders, like `~/.config`.
    places_dir: PathBuf,
    /// Where its scratch runs get their folders.
    scratch_dir: PathBuf,
    /// What of it runs in containers.
    contain: Contain,
    /// What runs its contained runs' processes.
    runtime: RunRuntime,
    /// The host's environment, which a daemon in a container is given
    /// its home and SSH agent from.
    env: HostEnv,
}

impl DevInstance {
    /// The operator key the dev config's operator-key entries read: fake.
    const OPERATOR_KEY_ENV: &'static str = "REN_DEV_OPERATOR_KEY";
    const OPERATOR_KEY: &'static str = "fake-dev-operator-key";
    /// The GitHub token the dev config's `[github]` reads: fake, and sent
    /// only to the fake provider.
    const GITHUB_TOKEN_ENV: &'static str = "REN_DEV_GITHUB_TOKEN";
    const GITHUB_TOKEN: &'static str = "fake-dev-github-token";
    /// Seconds between checks for runs mid-way.
    const RUN_POLL: Duration = Duration::from_secs(2);

    /// The checkout this xtask was built from, which its instance builds.
    pub fn checkout() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
    }

    pub fn new(env: &HostEnv, index: DevIndex) -> Result<Self, DevError> {
        let home = env
            .absolute_path("HOME")
            .ok_or(DevError::MissingEnv("HOME"))?;
        let name = index.name();
        let share = home.join(".local/share").join(&name);
        Ok(Self {
            index,
            repo: Self::checkout(),
            cargo: env
                .absolute_path("CARGO")
                .unwrap_or_else(|| PathBuf::from("cargo")),
            config: home.join(".config").join(&name).join("ren.toml"),
            data_dir: home.join(".local/state").join(&name),
            bin_dir: share.join("bin"),
            ui_dir: share.join("ui"),
            ui_next: share.join("ui.next"),
            work_dir: share.join("work"),
            places_dir: share.join("config"),
            scratch_dir: share.join("scratch"),
            contain: Contain::None,
            runtime: RunRuntime::HostKernel,
            env: env.clone(),
        })
    }

    /// The instance, with what `contain` says in containers.
    #[must_use]
    pub fn containing(mut self, contain: Contain) -> Self {
        self.contain = contain;
        self
    }

    /// The Podman API socket a daemon in a container starts runs through:
    /// Podman's own, or the one whose service runs gVisor.
    fn podman_socket(&self) -> Option<PathBuf> {
        let runtime_dir = self.env.absolute_path("XDG_RUNTIME_DIR")?;
        Some(match self.runtime {
            RunRuntime::HostKernel => runtime_dir.join("podman/podman.sock"),
            RunRuntime::Gvisor => GvisorService::socket(&runtime_dir),
        })
    }

    /// The instance, its contained runs run by `runtime`.
    #[must_use]
    pub fn running_on(mut self, runtime: RunRuntime) -> Self {
        self.runtime = runtime;
        self
    }

    /// The dev user `ci`'s token. Fixed, since the slot's data is fake and
    /// its gateway reaches only the fake provider.
    pub fn ci_token() -> UserToken {
        UserToken::from_random(
            0x00de_7000_0000_0000_0000_0000_0000_0001,
            0x00de_7000_0000_0000_0000_0000_0000_00c1,
        )
    }

    /// The dev configuration: every provider is the fake one, two named
    /// users, and operator-key entries whose key is fake.
    pub fn config(&self) -> Result<Config, DevError> {
        let provider = self.index.provider();
        let fake = |prefix: &str| format!("http://{provider}/{prefix}");
        let id = |text: &str| text.parse::<UpstreamId>().map_err(|_| DevError::Config);
        let user = |text: &str| text.parse::<UserId>().map_err(|_| DevError::Config);
        let login = "alice@example.com"
            .parse::<Login>()
            .map_err(|_| DevError::Config)?;
        Ok(Config {
            listen: self.index.daemon(),
            data_dir: Some(self.data_dir.clone()),
            anthropic_base_url: fake("anthropic"),
            codex_base_url: fake("codex"),
            openai_base_url: fake("openai"),
            deepseek_base_url: fake("deepseek"),
            users: vec![
                UserConfig {
                    id: user("alice")?,
                    tailscale_logins: vec![login],
                    token_blake3: None,
                    max_concurrent: None,
                },
                UserConfig {
                    id: user("ci")?,
                    tailscale_logins: vec![],
                    token_blake3: Some(TokenDigest::of(&Self::ci_token())),
                    max_concurrent: Some(ren_gateway::ConcurrencyLimit::saturating(4)),
                },
            ],
            upstreams: vec![
                UpstreamConfig {
                    id: id("team-key")?,
                    provider: Provider::Anthropic,
                    wire: None,
                    base_url: fake("anthropic"),
                    operator_key_env: Self::OPERATOR_KEY_ENV.into(),
                    ingress: EntryIngress::Direct,
                    users: Some(vec![user("ci")?]),
                },
                UpstreamConfig {
                    id: id("chat-key")?,
                    provider: Provider::OpenAi,
                    wire: Some(Wire::OpenAiChatCompletions),
                    base_url: fake("openai"),
                    operator_key_env: Self::OPERATOR_KEY_ENV.into(),
                    ingress: EntryIngress::Direct,
                    users: None,
                },
            ],
            instance: InstanceOptions {
                environment: Environment::Development,
            },
            runs: RunsOptions {
                roots: vec![self.work_dir.clone()],
                places: vec![self.places_dir.clone()],
                scratch: Some(self.scratch_dir.clone()),
                // Held, so a restart of the slot leaves its runs going.
                hold: match self.contain {
                    Contain::None => RunHold::Auto,
                    Contain::Runs | Contain::All => RunHold::Container,
                },
                worktree_env: Default::default(),
                container: match self.contain {
                    Contain::None => RunContainerOptions::default(),
                    // The host's programs; the fake agent, which is this
                    // slot's xtask, and its state beside it, never the data.
                    Contain::Runs | Contain::All => RunContainerOptions {
                        image: Some(Pod::image()),
                        read_only: vec!["/usr".into(), self.bin_dir.clone()],
                        read_write: vec![self.agent_state()],
                        podman_socket: match self.contain {
                            Contain::All => self.podman_socket(),
                            Contain::None | Contain::Runs => None,
                        },
                        // Relayed, as runs are meant to be (phase 12): the
                        // fake agents need only the gateway, and one public
                        // registry checks a run's way out.
                        relayed: Some(RelayedOptions {
                            runtime: self.runtime,
                            egress: vec![
                                "index.crates.io:443"
                                    .parse()
                                    .map_err(|_| DevError::Config)?,
                            ],
                        }),
                    },
                },
            },
            packages: toml::from_str::<Config>(Self::PACKAGES)
                .map_err(|_| DevError::Config)?
                .packages,
            // The fake projects' repositories, on the fake provider's GitHub.
            github: GithubOptions {
                token_env: Self::GITHUB_TOKEN_ENV.into(),
                poll_seconds: 30.try_into().map_err(|_| DevError::Config)?,
                endpoint: fake("github/graphql"),
                // The fake GitHub's App, which grants relayed runs a token
                // for their fake repository.
                app: Some(GithubApp {
                    id: ren_github::AppId::new(1),
                    key_file: self.app_key(),
                    api: fake("github/api").parse().map_err(|_| DevError::Config)?,
                }),
                repository: Self::PROJECTS
                    .iter()
                    .chain(&["ren"])
                    .map(|project| {
                        Ok(WatchedRepository {
                            project: project.parse().map_err(|_| DevError::Config)?,
                            github: format!("acme/{}", project.trim_start_matches("acme-"))
                                .parse()
                                .map_err(|_| DevError::Config)?,
                        })
                    })
                    .collect::<Result<_, DevError>>()?,
            },
            // A deployer of its own, if one listens (`ren deployer canary
            // serve --of dev --socket` here): asked to plan a preview at
            // once.
            deployer: DeployerOptions {
                socket: self.env.absolute_path("XDG_RUNTIME_DIR").map(|runtime| {
                    runtime
                        .join(format!("{}-deployer", self.index.name()))
                        .join("deployer.sock")
                }),
            },
            ..Config::default()
        })
    }

    /// An operator catalog entry, so the Packages view shows one. Its
    /// download is invented and never answers, so installing it fails at
    /// fetch.
    const PACKAGES: &'static str = r#"
[[packages]]
id = "acme-lint"
kind = "utility"
program = "acme-lint"

[packages.release]
version = "0.3.0"

[[packages.release.artifact]]
platform = "linux_x64"
url = "https://example.invalid/acme-lint-0.3.0.tar.gz"
sha256 = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
format = "tar_gz"
program_at = "acme-lint"
"#;

    /// Build this checkout and copy the binaries the units run, so a later
    /// build never replaces one under a running unit.
    fn build(&self) -> Result<(), DevError> {
        Progress("Building ren and xtask from this checkout").show();
        Invocation::new(&self.cargo)
            .current_dir(&self.repo)
            .args(["build", "--locked", "-p", "ren-rs", "-p", "xtask"])
            .run()?;
        fs::create_dir_all(&self.bin_dir).map_err(FileError::at(&self.bin_dir))?;
        for name in ["ren", "xtask"] {
            let built = self.repo.join("target/debug").join(name);
            let contents = fs::read(&built).map_err(FileError::at(&built))?;
            FileMode::Executable.write(&self.bin_dir.join(name), &contents)?;
        }
        self.write_agent()
    }

    /// The fake projects runs start in.
    pub const PROJECTS: [&'static str; 2] = ["acme-api", "acme-web"];

    /// The fake folders of its other place.
    const PLACES: [&'static str; 3] = ["nvim", "acme-cli", ".hidden-tool"];

    /// The folder holding the `codex`, `claude`, and `pi` the dev daemon starts:
    /// the fake agent.
    fn agent_dir(&self) -> PathBuf {
        self.bin_dir.join("agent")
    }

    /// Where the fake agent keeps its state, which its runs see: beside its
    /// program, never in the dev daemon's data.
    fn agent_state(&self) -> PathBuf {
        self.bin_dir.with_file_name("agent-state")
    }

    /// Put the fake agent first on the dev daemon's PATH as `codex`,
    /// `claude`, and `pi`, and create the fake projects under the run root.
    fn write_agent(&self) -> Result<(), DevError> {
        let dir = self.agent_dir();
        fs::create_dir_all(&dir).map_err(FileError::at(&dir))?;
        let state = self.agent_state();
        fs::create_dir_all(&state).map_err(FileError::at(&state))?;
        for (program, client, name) in [
            ("codex", "codex", "Codex"),
            ("claude", "claude", "Claude Code"),
            ("pi", "pi", "Pi"),
            ("opencode", "opencode", "OpenCode"),
        ] {
            let script = format!(
                "#!/bin/sh\n# The dev slot's fake {name} (`cargo xtask dev agent`); never the real one.\nexec {} dev agent --client {client} --state-dir {} \"$@\"\n",
                Self::quote(&self.bin_dir.join("xtask")),
                Self::quote(&state),
            );
            FileMode::Executable.write(&dir.join(program), script.as_bytes())?;
        }
        // Its runs' scratch folders' parent, which a contained daemon mounts.
        fs::create_dir_all(&self.scratch_dir).map_err(FileError::at(&self.scratch_dir))?;
        for place in Self::PLACES {
            let folder = self.places_dir.join(place);
            fs::create_dir_all(&folder).map_err(FileError::at(&folder))?;
        }
        for project in Self::PROJECTS {
            let folder = self.work_dir.join(project);
            fs::create_dir_all(folder.join("src")).map_err(FileError::at(&folder))?;
            let readme = folder.join("README.md");
            if !readme.exists() {
                fs::write(
                    &readme,
                    format!("# {project}\n\nA fake project for the dev slot's runs.\n"),
                )
                .map_err(FileError::at(&readme))?;
            }
            let git = Git::inherited();
            let author = Author {
                name: "Ren dev slot".into(),
                email: "dev-slot@example.com".into(),
            };
            // A repository with a commit, so a session (and Redline's
            // briefs) can work in a worktree of it.
            if Git::repository(&folder) == Repository::None {
                git.init(&folder)
                    .and_then(|()| git.commit_all(&folder, "Start the fake project", &author))?;
            }
            // Its manifest, so a worktree of it has checks and a preview.
            let manifest = folder.join(ren_manifest::Manifest::FILE);
            if !manifest.exists() {
                for (name, text) in Self::site(project) {
                    let path = folder.join(name);
                    if let Some(parent) = path.parent() {
                        fs::create_dir_all(parent).map_err(FileError::at(parent))?;
                    }
                    fs::write(&path, text).map_err(FileError::at(&path))?;
                }
                git.commit_all(
                    &folder,
                    "Declare how Ren builds, serves, and checks it",
                    &author,
                )?;
            }
        }
        Ok(())
    }

    /// A fake project's page and its manifest: a static page served by
    /// Python's own server on the worktree's first port, checked by
    /// parsing it.
    fn site(project: &str) -> [(&'static str, String); 2] {
        [
            (
                "index.html",
                format!(
                    "<!doctype html>\n<html lang=\"en\">\n<head><meta charset=\"utf-8\"><title>{project}</title></head>\n<body><main><h1>{project}</h1><p>A fake project for the dev slot's runs.</p></main></body>\n</html>\n"
                ),
            ),
            (
                ren_manifest::Manifest::FILE,
                r#"# How Ren builds, serves, and checks this fake project.

[[build]]
run = ["python3", "-c", "print('Nothing to build: the page is static.')"]

[serve]
run = ["python3", "-m", "http.server", "{port}", "--bind", "127.0.0.1"]
open = "http://127.0.0.1:{port}"
health = "/"

[[check]]
name = "page"
run = ["python3", "-c", "import html.parser; html.parser.HTMLParser().feed(open('index.html').read()); print('index.html parses')"]

# Production for a fake project: a folder under its canary's data.
[deploy]
run = ["python3", "-c", "import pathlib, shutil; p = pathlib.Path('{data}') / 'production'; p.mkdir(parents=True, exist_ok=True); shutil.copy('index.html', p / 'index.html'); print('deployed to', p)"]
"#
                .to_owned(),
            ),
        ]
    }

    /// `path` as one single-quoted shell word.
    fn quote(path: &Path) -> String {
        format!("'{}'", path.display().to_string().replace('\'', "'\\''"))
    }

    /// The fake GitHub App's private key, beside the configuration.
    fn app_key(&self) -> PathBuf {
        self.config
            .parent()
            .unwrap_or(Path::new("/"))
            .join("github-app.pem")
    }

    /// A key for the fake App, made once: the fake GitHub never checks
    /// what it signs.
    fn write_app_key(&self) -> Result<(), DevError> {
        use aws_lc_rs::{encoding::AsDer as _, rsa::KeySize, signature};
        use base64::Engine as _;
        let path = self.app_key();
        if path.exists() {
            return Ok(());
        }
        let der = signature::RsaKeyPair::generate(KeySize::Rsa2048)
            .and_then(|pair| pair.as_der())
            .map_err(|_| DevError::Config)?;
        let pem = format!(
            "-----BEGIN PRIVATE KEY-----\n{}\n-----END PRIVATE KEY-----\n",
            base64::engine::general_purpose::STANDARD.encode(der.as_ref())
        );
        Ok(FileMode::Private.write(&path, pem.as_bytes())?)
    }

    fn write_config(&self) -> Result<(), DevError> {
        let dir = self.config.parent().unwrap_or(Path::new("/"));
        fs::create_dir_all(dir).map_err(FileError::at(dir))?;
        self.write_app_key()?;
        let config = self.config()?;
        config.check().map_err(|_| DevError::Config)?;
        let text = toml::to_string(&config).map_err(|_| DevError::Config)?;
        fs::create_dir_all(&self.data_dir).map_err(FileError::at(&self.data_dir))?;
        FileMode::Private.write(
            &self.config,
            format!("# Generated by `cargo xtask dev up`; rewritten on every run.\n{text}")
                .as_bytes(),
        )?;
        Ok(())
    }

    /// Whether `address` answers `path` within a few seconds.
    pub fn wait_for(address: SocketAddr, path: &str) -> Result<(), DevError> {
        if Listener(address).answers_soon(path) {
            Ok(())
        } else {
            Err(DevError::NotReady(address))
        }
    }

    /// Build, configure, and (re)start the provider and daemon.
    pub fn up(&self, wait: RunWait) -> Result<(), DevError> {
        self.start_core(wait)
    }

    /// Start the core, then build and run the UI and, unless `Preview::Local`,
    /// publish the dev gateway and UI on the tailnet.
    pub fn up_with_ui(
        &self,
        env: &HostEnv,
        preview: Preview,
        wait: RunWait,
    ) -> Result<(), DevError> {
        let ui = UiBuild::new(env, &self.repo)?;
        self.start_core(wait)?;
        ui.build_into(&self.ui_next)?;
        Unit::Ui.stop(self.index)?;
        if self.ui_dir.exists() {
            fs::remove_dir_all(&self.ui_dir).map_err(FileError::at(&self.ui_dir))?;
        }
        fs::rename(&self.ui_next, &self.ui_dir).map_err(FileError::at(&self.ui_dir))?;
        let (public_gateway, origin) = match preview {
            Preview::Local => (
                format!("http://{}", self.index.daemon()),
                format!("http://{}", self.index.ui()),
            ),
            Preview::Tailnet(tailscale) => {
                let dns = tailscale
                    .publish(
                        self.index.tailnet().0,
                        &format!("http://{}", self.index.daemon()),
                    )?
                    .ok_or(TailscaleError::NoDnsName)?;
                (
                    format!("https://{dns}:{}", self.index.tailnet().0),
                    format!("https://{dns}:{}", self.index.tailnet().1),
                )
            }
        };
        Progress(&format!("Starting the dev UI on {}", self.index.ui())).show();
        let token = self.data_dir.join(ControlToken::FILE);
        let vars = [
            ("HOST", self.index.ui().ip().to_string()),
            ("PORT", self.index.ui().port().to_string()),
            ("REN_GATEWAY_URL", format!("http://{}", self.index.daemon())),
            ("REN_PUBLIC_GATEWAY_URL", public_gateway),
            ("ORIGIN", origin.clone()),
            ("REN_CONTROL_TOKEN_FILE", token.display().to_string()),
            // A fake project stands in for Ren's checkout: the slot's
            // fake agents never work in the real one.
            (
                "REN_SOURCE_DIR",
                self.work_dir.join(Self::PROJECTS[1]).display().to_string(),
            ),
        ];
        match self.contain {
            Contain::None | Contain::Runs => Unit::Ui.start(
                self.index,
                Some(&self.ui_dir),
                &[ui.node()],
                &[UiBuild::entry().display().to_string()],
                &vars,
            )?,
            Contain::All => {
                let container = UiContainer::new(
                    &self.index.label(),
                    Pod::image(),
                    &self.ui_dir,
                    Some(&token),
                );
                Progress(&format!("In container {}", container.name())).show();
                Unit::Ui.start(
                    self.index,
                    None,
                    &[Path::new(Engine::Podman.program())],
                    &container.run_args(),
                    &vars,
                )?;
            }
        }
        // The UI's /health relays the dev daemon's.
        Self::wait_for(self.index.ui(), "/health")?;
        if let Preview::Tailnet(tailscale) = preview {
            tailscale.publish(
                self.index.tailnet().1,
                &format!("http://{}", self.index.ui()),
            )?;
        }
        println!("\nDev preview: {origin}\n");
        Ok(())
    }

    fn start_core(&self, wait: RunWait) -> Result<(), DevError> {
        self.build()?;
        match self.contain {
            Contain::None => {}
            Contain::Runs => Pod::ensure_image(&self.repo)?,
            Contain::All => {
                Pod::ensure_image(&self.repo)?;
                // The socket its daemon starts runs' containers through.
                Invocation::new("systemctl")
                    .args(["--user", "enable", "--now", "podman.socket"])
                    .run()?;
                match self.runtime {
                    RunRuntime::HostKernel => {}
                    RunRuntime::Gvisor => GvisorService::new(&self.env)?.ensure()?,
                }
            }
        }
        self.write_config()?;
        let ending = self.settle_runs(wait)?;
        ending.announce();
        for unit in [Unit::Daemon, Unit::Provider] {
            unit.stop(self.index)?;
        }
        let xtask = self.bin_dir.join("xtask");
        let ren = self.bin_dir.join("ren");
        Progress(&format!(
            "Starting the fake provider on {}",
            self.index.provider()
        ))
        .show();
        Unit::Provider.start(
            self.index,
            None,
            &[&xtask],
            &[
                "dev".into(),
                "provider".into(),
                "--listen".into(),
                self.index.provider().to_string(),
                "--settings".into(),
                self.traffic_settings().display().to_string(),
                "--github".into(),
                self.fake_github().display().to_string(),
            ],
            &[],
        )?;
        Self::wait_for(self.index.provider(), "/anthropic/v1/models")?;
        Progress(&format!(
            "Starting the dev daemon on {}",
            self.index.daemon()
        ))
        .show();
        let vars = vec![
            (
                Self::OPERATOR_KEY_ENV.to_owned(),
                Self::OPERATOR_KEY.to_owned(),
            ),
            (
                Self::GITHUB_TOKEN_ENV.to_owned(),
                Self::GITHUB_TOKEN.to_owned(),
            ),
            // Runs start the fake agent as `codex`, never a real client.
            (
                "PATH".to_owned(),
                format!(
                    "{}:/usr/local/bin:/usr/bin:/bin",
                    self.agent_dir().display()
                ),
            ),
        ];
        match self.contain {
            Contain::None | Contain::Runs => {
                let vars: Vec<(&str, String)> = vars
                    .iter()
                    .map(|(name, value)| (name.as_str(), value.clone()))
                    .collect();
                Unit::Daemon.start(
                    self.index,
                    None,
                    &[&ren],
                    &[
                        "--config".into(),
                        self.config.display().to_string(),
                        "serve".into(),
                    ],
                    &vars,
                )?;
            }
            Contain::All => {
                let config = self.config()?;
                let container = DaemonContainer::new(
                    &self.index.label(),
                    Pod::image(),
                    &ren,
                    (&self.config, &config),
                    &self.data_dir,
                    &self.env,
                    vars,
                )?;
                Progress(&format!("In container {}", container.name())).show();
                Unit::Daemon.start(
                    self.index,
                    None,
                    &[Path::new(Engine::Podman.program())],
                    &container.run_args(),
                    &[],
                )?;
            }
        }
        Self::wait_for(self.index.daemon(), "/health")?;
        self.register_projects();
        ending.remind();
        self.start_traffic()
    }

    /// Registers the fake projects, and this checkout as Ren itself, as
    /// `install-vps` registers production's (docs/plans/hierarchy.md). One
    /// registered already is left as it is; a refusal is reported, not
    /// failed.
    fn register_projects(&self) {
        let projects = Self::PROJECTS
            .iter()
            .map(|project| (*project, self.work_dir.join(project), "contract"))
            .chain([("ren", self.repo.clone(), "ren")]);
        for (name, dir, factory) in projects {
            let said = Invocation::new(self.bin_dir.join("ren"))
                .arg("--config")
                .arg(&self.config)
                .args([
                    "project",
                    "add",
                    "--name",
                    name,
                    "--factory",
                    factory,
                    "--dir",
                ])
                .arg(&dir)
                .capture();
            // The slot's data is the host's: another checkout may hold the
            // name already. The slot runs either way.
            match said {
                Ok(said) => Progress(said.trim()).show(),
                Err(error) => {
                    println!(
                        "warning: {name} is not registered for {}: {error}",
                        dir.display()
                    );
                }
            }
        }
    }

    /// Waits until the running dev daemon has no run mid-way (calls are not
    /// waited for: the fake traffic never stops), and returns the runs that
    /// end with it. Polls are counted rather than timed, so this reads no
    /// clock.
    fn settle_runs(&self, wait: RunWait) -> Result<Ending, DevError> {
        if !Unit::Daemon.active(self.index)? {
            return Ok(Ending::default());
        }
        let ren = || {
            Invocation::new(self.bin_dir.join("ren"))
                .arg("--config")
                .arg(&self.config)
        };
        let polls = wait.timeout.div_ceil(Self::RUN_POLL.as_secs()).max(1);
        let mut reported = None;
        for _ in 0..polls {
            let runs = Runs::read(ren())?;
            let busy = Busy::of_runs(&runs, wait.policy);
            if busy.is_quiet() {
                return Ok(runs.ending());
            }
            busy.report(&mut reported, wait.timeout);
            thread::sleep(Self::RUN_POLL);
        }
        Err(StillBusy {
            busy: reported.unwrap_or_else(|| Busy::of_runs(&Runs::default(), wait.policy)),
            waited: wait.timeout,
        }
        .into())
    }

    /// (Re)start the traffic generator, which follows the settings file.
    pub fn start_traffic(&self) -> Result<(), DevError> {
        Unit::Traffic.stop(self.index)?;
        Progress("Starting the traffic generator").show();
        Unit::Traffic.start(
            self.index,
            None,
            &[&self.bin_dir.join("xtask")],
            &[
                "dev".into(),
                "--instance".into(),
                self.index.to_string(),
                "traffic".into(),
                "run".into(),
                "--control".into(),
                self.index.control().to_string(),
            ],
            // The user manager gives its units HOME, which the slot reads.
            &[],
        )?;
        Ok(())
    }

    /// The live traffic settings' file.
    pub fn traffic_settings(&self) -> PathBuf {
        self.config.with_file_name("traffic.toml")
    }

    /// The fake GitHub's `acme/ren`, when written by hand (its `main`, and
    /// `[[pull]]`, each a `number`, a `head`, and whether it is a
    /// `draft`); without it, invented.
    pub fn fake_github(&self) -> PathBuf {
        self.config.with_file_name("fake-github.toml")
    }

    /// Stop the slot's units, then its runs' run hosts, which outlive its
    /// daemon: each in a unit or a container of its own, named for the run.
    pub fn down(&self) -> Result<(), DevError> {
        for unit in Unit::ALL {
            Progress(&format!("Stopping {}", unit.name(self.index))).show();
            unit.stop(self.index)?;
        }
        let runs = self.data_dir.join(HeldRuns::FOLDER);
        for run in HeldRuns::folders(&runs).map_err(FileError::at(&runs))? {
            let unit = format!("{}.service", HeldRuns::unit(run));
            Invocation::new("systemctl")
                .args(["--user", "stop", &unit])
                .succeeds(Echo::Hide)?;
            Invocation::new("systemctl")
                .args(["--user", "reset-failed", &unit])
                .succeeds(Echo::Hide)?;
            // A run host in a container: removing it stops it. Without
            // Podman there is no container to remove.
            if let Err(error) = Engine::Podman.remove_container(&HeldRuns::unit(run)) {
                println!("warning: run {run}'s container was not removed: {error}");
            }
        }
        Ok(())
    }

    /// Remove the dev preview's Serve routes, each only if it still proxies
    /// to the slot; the installed service's routes are never touched.
    pub fn unpublish(&self, tailscale: Tailnet) -> Result<(), DevError> {
        Ok(tailscale.unpublish(&[
            (
                self.index.tailnet().0,
                format!("http://{}", self.index.daemon()),
            ),
            (
                self.index.tailnet().1,
                format!("http://{}", self.index.ui()),
            ),
        ])?)
    }

    /// Stop the instance and remove its data. Only the instance's own
    /// directory is removed, and only if it is the one its index names.
    pub fn wipe(&self) -> Result<(), DevError> {
        self.down()?;
        let ours = Path::new(".local/state").join(self.index.name());
        if self.data_dir.ends_with(&ours) && self.data_dir.exists() {
            Progress(&format!("Removing {}", self.data_dir.display())).show();
            fs::remove_dir_all(&self.data_dir).map_err(FileError::at(&self.data_dir))?;
        }
        Ok(())
    }

    /// The file that says the instance was seeded, in its data directory,
    /// which a reset removes.
    const SEEDED: &str = "seeded";

    /// Whether the instance's data was ever seeded.
    pub fn seeded(&self) -> bool {
        self.data_dir.join(Self::SEEDED).exists()
    }

    /// Write the history `seed` invents into the instance's store with its
    /// own `ren seed`, the daemon stopped meanwhile and started again.
    pub fn seed(&self, seed: u64) -> Result<(), DevError> {
        Unit::Daemon.stop(self.index)?;
        Progress(&format!("Seeding the history of seed {seed}")).show();
        Invocation::new(self.bin_dir.join("ren"))
            .arg("--config")
            .arg(&self.config)
            .arg("--data-dir")
            .arg(&self.data_dir)
            .args(["seed", "--seed", &seed.to_string()])
            .run()?;
        let marker = self.data_dir.join(Self::SEEDED);
        fs::write(&marker, seed.to_string()).map_err(FileError::at(&marker))?;
        // The daemon was stopped above: no run is left to wait for.
        self.up(RunWait {
            policy: RunPolicy::Drain,
            timeout: 0,
        })
    }

    pub fn status(&self) -> Result<(), DevError> {
        for unit in Unit::ALL.iter().rev() {
            let state = if unit.active(self.index)? {
                "running"
            } else {
                "stopped"
            };
            println!("{:<18} {state}", unit.name(self.index));
        }
        println!("fake provider      http://{}", self.index.provider());
        println!("dev gateway        http://{}", self.index.daemon());
        println!("dev UI             http://{}", self.index.ui());
        println!("traffic control    http://{}", self.index.control());
        println!("config             {}", self.config.display());
        println!("data               {}", self.data_dir.display());
        println!("run root           {}", self.work_dir.display());
        println!(
            "history            {} --config {} history",
            self.bin_dir.join("ren").display(),
            self.config.display()
        );
        let units: Vec<String> = Unit::ALL
            .iter()
            .rev()
            .map(|unit| format!("-u {}", unit.name(self.index)))
            .collect();
        println!("logs               journalctl --user {}", units.join(" "));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{DevIndex, DevInstance, Unit};
    use ren_harness::host::HostEnv;

    #[test]
    fn every_unit_is_named_for_its_instance() {
        for index in (0..DevIndex::COUNT).map(|index| index.to_string().parse().unwrap()) {
            for unit in Unit::ALL {
                let name = unit.name(index);
                assert!(name.starts_with(&format!("{}-", index.name())), "{name}");
                assert_ne!(name, "ren");
                assert_ne!(name, "ren-ui");
            }
        }
        // The first's are the dev slot's.
        assert_eq!(Unit::Daemon.name(DevIndex::FIRST), "ren-dev-daemon");
        assert_eq!(Unit::Daemon.name("2".parse().unwrap()), "ren-dev-2-daemon");
    }

    #[test]
    fn every_fake_project_declares_a_manifest_ren_reads() {
        for project in DevInstance::PROJECTS {
            let [_, (name, text)] = DevInstance::site(project);
            assert_eq!(name, ren_manifest::Manifest::FILE);
            let manifest = ren_manifest::Manifest::read(&text).unwrap();
            assert!(manifest.serve().is_some() && !manifest.checks().is_empty());
            assert!(manifest.deploy().is_some());
        }
    }

    #[test]
    fn the_dev_config_is_valid_and_reaches_only_the_fake_provider() {
        let slot = DevInstance::new(
            &HostEnv::from_pairs([("HOME", "/home/dev")]),
            DevIndex::FIRST,
        )
        .unwrap();
        let config = slot.config().unwrap();
        config.check().unwrap();
        assert_eq!(config.packages.entries().len(), 1);
        let text = toml::to_string(&config).unwrap();
        for url in [
            &config.anthropic_base_url,
            &config.codex_base_url,
            &config.openai_base_url,
            &config.deepseek_base_url,
        ] {
            assert!(url.starts_with("http://127.0.0.1:18999/"), "{url}");
        }
        assert!(
            config
                .upstreams
                .iter()
                .all(|upstream| upstream.base_url.starts_with("http://127.0.0.1:18999/"))
        );
        assert!(config.listen.ip().is_loopback());
        assert_eq!(
            config.runs.roots,
            [std::path::Path::new("/home/dev/.local/share/ren-dev/work")]
        );
        assert_eq!(
            config.instance.environment,
            ren_gateway::Environment::Development
        );
        assert_eq!(
            slot.data_dir,
            std::path::Path::new("/home/dev/.local/state/ren-dev")
        );
        // The token itself is never written, only its digest.
        assert!(!text.contains(DevInstance::ci_token().expose()));
    }
}
