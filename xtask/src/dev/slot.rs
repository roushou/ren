//! The dev slot: this checkout's build running beside the installed service,
//! with its own configuration, data, and loopback ports, pointed only at the
//! fake provider. Each process runs as a transient systemd user unit named
//! `ren-dev-*`, so it is stopped by name and never by a pattern that could
//! match the installed service.

use super::DevError;
use crate::file::{FileError, FileMode};
use crate::progress::Progress;
use crate::{
    daemon::{Busy, Ending, RunPolicy, Runs, StillBusy},
    invocation::{Echo, Invocation, InvocationError},
    tailscale::{TailscaleCli, TailscaleError},
    ui::UiBuild,
    unit::{KillMode, Listener, TransientUnit},
};
use ren_daemon::config::{Config, RunHold, RunsOptions, ToolsOptions, UpstreamConfig};
use ren_daemon::host::HeldRuns;
use ren_gateway::{
    ControlToken, EntryIngress, Environment, InstanceOptions,
    user::{TokenDigest, UserConfig},
};
use ren_git::{Author, Git, Repository};
use ren_harness::{endpoint::UserToken, host::HostEnv};
use ren_protocol::route::{Provider, Wire};
use ren_recorder::{
    caller::{TailscaleLogin, UserId},
    routing::UpstreamId,
};
use std::{
    fs,
    net::SocketAddr,
    path::{Path, PathBuf},
    thread,
    time::Duration,
};

/// One process of the slot.
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

    /// The transient unit; every one's name starts with `ren-dev-`.
    pub fn unit(self) -> TransientUnit {
        let (name, description) = match self {
            Self::Provider => ("ren-dev-provider", "Ren dev slot: ren-dev-provider"),
            Self::Daemon => ("ren-dev-daemon", "Ren dev slot: ren-dev-daemon"),
            Self::Ui => ("ren-dev-ui", "Ren dev slot: ren-dev-ui"),
            Self::Traffic => ("ren-dev-traffic", "Ren dev slot: ren-dev-traffic"),
        };
        // The daemon is signalled alone, as the installed unit does; the
        // rest stop as a whole.
        let kill = match self {
            Self::Daemon => KillMode::Mixed,
            Self::Provider | Self::Ui | Self::Traffic => KillMode::ControlGroup,
        };
        TransientUnit {
            name: name.to_owned(),
            description: description.to_owned(),
            kill,
        }
    }

    pub fn name(self) -> String {
        self.unit().name
    }

    pub fn active(self) -> Result<bool, InvocationError> {
        self.unit().active()
    }

    pub fn stop(self) -> Result<(), InvocationError> {
        self.unit().stop()
    }

    /// Run `command` as this unit, with `environment`, until stopped.
    pub fn start(
        self,
        dir: Option<&Path>,
        command: &[&Path],
        args: &[String],
        environment: &[(&str, String)],
    ) -> Result<(), InvocationError> {
        self.unit().start(dir, command, args, environment)
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
    Tailnet(TailscaleCli),
    /// Only on this host's loopback.
    Local,
}

/// Where the slot lives and listens.
pub struct Slot {
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
}

impl Slot {
    pub const PROVIDER: SocketAddr =
        SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), 18999);
    pub const DAEMON: SocketAddr =
        SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), 18787);
    pub const UI: SocketAddr =
        SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), 13000);
    /// The tailnet HTTPS ports of the dev preview, beside the installed
    /// service's.
    pub const GATEWAY_PORT: u16 = 9443;
    pub const UI_PORT: u16 = 9444;
    /// The traffic generator's control API, which the dev UI's toolbar
    /// reaches through the UI server; never published.
    pub const CONTROL: SocketAddr =
        SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), 13001);
    /// The operator key the dev config's operator-key entries read: fake.
    const OPERATOR_KEY_ENV: &'static str = "REN_DEV_OPERATOR_KEY";
    const OPERATOR_KEY: &'static str = "fake-dev-operator-key";
    /// Seconds between checks for runs mid-way.
    const RUN_POLL: Duration = Duration::from_secs(2);

    pub fn new(env: &HostEnv) -> Result<Self, DevError> {
        let home = env
            .absolute_path("HOME")
            .ok_or(DevError::MissingEnv("HOME"))?;
        let share = home.join(".local/share/ren-dev");
        Ok(Self {
            repo: Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .map_or_else(|| PathBuf::from("."), Path::to_path_buf),
            cargo: env
                .absolute_path("CARGO")
                .unwrap_or_else(|| PathBuf::from("cargo")),
            config: home.join(".config/ren-dev/ren.toml"),
            data_dir: home.join(".local/state/ren-dev"),
            bin_dir: share.join("bin"),
            ui_dir: share.join("ui"),
            ui_next: share.join("ui.next"),
            work_dir: share.join("work"),
            places_dir: share.join("config"),
            scratch_dir: share.join("scratch"),
        })
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
        let fake = |prefix: &str| format!("http://{}/{prefix}", Self::PROVIDER);
        let id = |text: &str| text.parse::<UpstreamId>().map_err(|_| DevError::Config);
        let user = |text: &str| text.parse::<UserId>().map_err(|_| DevError::Config);
        let login = "alice@example.com"
            .parse::<TailscaleLogin>()
            .map_err(|_| DevError::Config)?;
        Ok(Config {
            listen: Self::DAEMON,
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
                // Held as the installed service holds them, so a restart
                // of the slot leaves its runs going.
                hold: RunHold::Auto,
                workspace_env: Default::default(),
            },
            tools: toml::from_str::<ToolsOptions>(Self::TOOLS).map_err(|_| DevError::Config)?,
            ..Config::default()
        })
    }

    /// An operator catalog entry, so the Tools view shows one. Its download
    /// is invented and never answers, so installing it fails at fetch.
    const TOOLS: &'static str = r#"
[[package]]
id = "acme-lint"
kind = "utility"
program = "acme-lint"

[package.release]
version = "0.3.0"

[[package.release.artifact]]
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

    /// Put the fake agent first on the dev daemon's PATH as `codex`,
    /// `claude`, and `pi`, and create the fake projects under the run root.
    fn write_agent(&self) -> Result<(), DevError> {
        let dir = self.agent_dir();
        fs::create_dir_all(&dir).map_err(FileError::at(&dir))?;
        for (program, client, name) in [
            ("codex", "codex", "Codex"),
            ("claude", "claude", "Claude Code"),
            ("pi", "pi", "Pi"),
            ("opencode", "opencode", "OpenCode"),
        ] {
            let script = format!(
                "#!/bin/sh\n# The dev slot's fake {name} (`cargo xtask dev agent`); never the real one.\nexec {} dev agent --client {client} --data-dir {} \"$@\"\n",
                Self::quote(&self.bin_dir.join("xtask")),
                Self::quote(&self.data_dir),
            );
            FileMode::Executable.write(&dir.join(program), script.as_bytes())?;
        }
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
            // briefs) can work in a workspace of it.
            if Git::repository(&folder) == Repository::None {
                git.init(&folder)
                    .and_then(|()| git.commit_all(&folder, "Start the fake project", &author))?;
            }
            // Its contract, so a workspace of it has checks and a preview.
            let contract = folder.join(ren_project::ProjectContract::FILE);
            if !contract.exists() {
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

    /// A fake project's page and its contract: a static page served by
    /// Python's own server on the workspace's first port, checked by
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
                ren_project::ProjectContract::FILE,
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

    fn write_config(&self) -> Result<(), DevError> {
        let config = self.config()?;
        config.check().map_err(|_| DevError::Config)?;
        let text = toml::to_string(&config).map_err(|_| DevError::Config)?;
        let dir = self.config.parent().unwrap_or(Path::new("/"));
        fs::create_dir_all(dir).map_err(FileError::at(dir))?;
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
        Unit::Ui.stop()?;
        if self.ui_dir.exists() {
            fs::remove_dir_all(&self.ui_dir).map_err(FileError::at(&self.ui_dir))?;
        }
        fs::rename(&self.ui_next, &self.ui_dir).map_err(FileError::at(&self.ui_dir))?;
        let (public_gateway, origin) = match preview {
            Preview::Local => (
                format!("http://{}", Self::DAEMON),
                format!("http://{}", Self::UI),
            ),
            Preview::Tailnet(tailscale) => {
                let dns = tailscale
                    .publish(Self::GATEWAY_PORT, &format!("http://{}", Self::DAEMON))?
                    .ok_or(TailscaleError::NoDnsName)?;
                (
                    format!("https://{dns}:{}", Self::GATEWAY_PORT),
                    format!("https://{dns}:{}", Self::UI_PORT),
                )
            }
        };
        Progress(&format!("Starting the dev UI on {}", Self::UI)).show();
        Unit::Ui.start(
            Some(&self.ui_dir),
            &[ui.node()],
            &[UiBuild::entry().display().to_string()],
            &[
                ("HOST", Self::UI.ip().to_string()),
                ("PORT", Self::UI.port().to_string()),
                ("REN_GATEWAY_URL", format!("http://{}", Self::DAEMON)),
                ("REN_PUBLIC_GATEWAY_URL", public_gateway),
                ("ORIGIN", origin.clone()),
                ("REN_DEV_TRAFFIC_URL", format!("http://{}", Self::CONTROL)),
                (
                    "REN_CONTROL_TOKEN_FILE",
                    self.data_dir.join(ControlToken::FILE).display().to_string(),
                ),
                // A fake project stands in for Ren's checkout: the slot's
                // fake agents never work in the real one.
                (
                    "REN_SOURCE_DIR",
                    self.work_dir.join(Self::PROJECTS[1]).display().to_string(),
                ),
            ],
        )?;
        // The UI's /health relays the dev daemon's.
        Self::wait_for(Self::UI, "/health")?;
        if let Preview::Tailnet(tailscale) = preview {
            tailscale.publish(Self::UI_PORT, &format!("http://{}", Self::UI))?;
        }
        println!("\nDev preview: {origin}\n");
        Ok(())
    }

    fn start_core(&self, wait: RunWait) -> Result<(), DevError> {
        self.build()?;
        self.write_config()?;
        let ending = self.settle_runs(wait)?;
        ending.announce();
        for unit in [Unit::Daemon, Unit::Provider] {
            unit.stop()?;
        }
        let xtask = self.bin_dir.join("xtask");
        let ren = self.bin_dir.join("ren");
        Progress(&format!("Starting the fake provider on {}", Self::PROVIDER)).show();
        Unit::Provider.start(
            None,
            &[&xtask],
            &[
                "dev".into(),
                "provider".into(),
                "--listen".into(),
                Self::PROVIDER.to_string(),
                "--settings".into(),
                self.traffic_settings().display().to_string(),
            ],
            &[],
        )?;
        Self::wait_for(Self::PROVIDER, "/anthropic/v1/models")?;
        Progress(&format!("Starting the dev daemon on {}", Self::DAEMON)).show();
        Unit::Daemon.start(
            None,
            &[&ren],
            &[
                "--config".into(),
                self.config.display().to_string(),
                "serve".into(),
            ],
            &[
                (Self::OPERATOR_KEY_ENV, Self::OPERATOR_KEY.into()),
                // Runs start the fake agent as `codex`, never a real client.
                (
                    "PATH",
                    format!(
                        "{}:/usr/local/bin:/usr/bin:/bin",
                        self.agent_dir().display()
                    ),
                ),
            ],
        )?;
        Self::wait_for(Self::DAEMON, "/health")?;
        ending.remind();
        self.start_traffic()
    }

    /// Waits until the running dev daemon has no run mid-way (calls are not
    /// waited for: the fake traffic never stops), and returns the runs that
    /// end with it. Polls are counted rather than timed, so this reads no
    /// clock.
    fn settle_runs(&self, wait: RunWait) -> Result<Ending, DevError> {
        if !Unit::Daemon.active()? {
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
        Unit::Traffic.stop()?;
        Progress("Starting the traffic generator").show();
        Unit::Traffic.start(
            None,
            &[&self.bin_dir.join("xtask")],
            &[
                "dev".into(),
                "traffic".into(),
                "run".into(),
                "--control".into(),
                Self::CONTROL.to_string(),
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

    /// Stop the slot's units, then its runs' run hosts, which outlive its
    /// daemon: each in a unit of its own, named for the run.
    pub fn down(&self) -> Result<(), DevError> {
        for unit in Unit::ALL {
            Progress(&format!("Stopping {}", unit.name())).show();
            unit.stop()?;
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
        }
        Ok(())
    }

    /// Remove the dev preview's Serve routes, each only if it still proxies
    /// to the slot; the installed service's routes are never touched.
    pub fn unpublish(tailscale: TailscaleCli) -> Result<(), DevError> {
        Ok(tailscale.unpublish(&[
            (Self::GATEWAY_PORT, format!("http://{}", Self::DAEMON)),
            (Self::UI_PORT, format!("http://{}", Self::UI)),
        ])?)
    }

    /// Stop the slot and remove its data. Only the slot's own directory is
    /// removed, and only if it is the one this slot names.
    pub fn wipe(&self) -> Result<(), DevError> {
        self.down()?;
        if self.data_dir.ends_with(".local/state/ren-dev") && self.data_dir.exists() {
            Progress(&format!("Removing {}", self.data_dir.display())).show();
            fs::remove_dir_all(&self.data_dir).map_err(FileError::at(&self.data_dir))?;
        }
        Ok(())
    }

    pub fn status(&self) -> Result<(), DevError> {
        for unit in Unit::ALL.iter().rev() {
            let state = if unit.active()? { "running" } else { "stopped" };
            println!("{:<18} {state}", unit.name());
        }
        println!("fake provider      http://{}", Self::PROVIDER);
        println!("dev gateway        http://{}", Self::DAEMON);
        println!("dev UI             http://{}", Self::UI);
        println!("traffic control    http://{}", Self::CONTROL);
        println!("config             {}", self.config.display());
        println!("data               {}", self.data_dir.display());
        println!("run root           {}", self.work_dir.display());
        println!(
            "history            {} --config {} history",
            self.bin_dir.join("ren").display(),
            self.config.display()
        );
        println!("logs               journalctl --user -u 'ren-dev-*'");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{Slot, Unit};
    use ren_harness::host::HostEnv;

    #[test]
    fn every_unit_is_named_for_the_dev_slot() {
        for unit in Unit::ALL {
            assert!(unit.name().starts_with("ren-dev-"), "{unit:?}");
            assert_ne!(unit.name(), "ren");
            assert_ne!(unit.name(), "ren-ui");
        }
    }

    #[test]
    fn every_fake_project_declares_a_contract_ren_reads() {
        for project in Slot::PROJECTS {
            let [_, (name, text)] = Slot::site(project);
            assert_eq!(name, ren_project::ProjectContract::FILE);
            let contract = ren_project::ProjectContract::read(&text).unwrap();
            assert!(contract.serve().is_some() && !contract.checks().is_empty());
            assert!(contract.deploy().is_some());
        }
    }

    #[test]
    fn the_dev_config_is_valid_and_reaches_only_the_fake_provider() {
        let slot = Slot::new(&HostEnv::from_pairs([("HOME", "/home/dev")])).unwrap();
        let config = slot.config().unwrap();
        config.check().unwrap();
        assert_eq!(config.tools.package.len(), 1);
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
        assert!(!text.contains(Slot::ci_token().expose()));
    }
}
