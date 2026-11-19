//! Ren on a copy of production's history: production, which a replica
//! copies (a backup, through production's own `ren`), and a [`Replica`], a
//! build started on that copy with a configuration derived from
//! production's: its own loopback ports and data folder, its own
//! `[instance] environment`, no runs, and a placeholder for each operator
//! key, so it forwards no operator-keyed call. Its UI only reads. The
//! canary is one (`cargo xtask canary`).

use crate::{
    dev::{DevError, slot::Slot},
    file::{FileError, FileMode},
    invocation::{Echo, Invocation, InvocationError},
    progress::Progress,
    tailscale::{TailscaleCli, TailscaleError},
    ui::{UiBuild, UiBuildError},
    unit::{KillMode, Listener, TransientUnit},
};
use ren_daemon::config::{Config, ConfigError, RunsOptions};
use ren_gateway::{Environment, InstanceOptions};
use ren_harness::host::HostEnv;
use ren_release::{Release, Revision};
use std::{
    fs,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    os::unix::fs::DirBuilderExt,
    path::{Path, PathBuf},
};

/// Which instance is production to a replica: the one it copies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Production {
    /// The installed service (`ren.service`), which promotions install into.
    Installed,
    /// The dev slot, with its fake history: for trying a replica out. It is
    /// never promoted into.
    Dev,
}

/// Where production is.
pub struct ProductionPlace {
    pub kind: Production,
    config: PathBuf,
    pub data_dir: PathBuf,
    binary: PathBuf,
    listen: SocketAddr,
}

/// One replica: where it lives and listens.
pub struct Replica {
    /// Its units' middle name: `ren-<name>-daemon` and `ren-<name>-ui`.
    name: String,
    /// What it says it is (`/health`), and what it calls itself here.
    environment: Environment,
    daemon: SocketAddr,
    ui: SocketAddr,
    config: PathBuf,
    /// Its data, replaced on every start; only this folder is removed.
    data_dir: PathBuf,
}

#[derive(Debug, thiserror::Error)]
pub enum ReplicaError {
    #[error("{0} is not set")]
    MissingEnv(&'static str),
    #[error("invalid configuration {}: {source}", path.display())]
    Config {
        path: PathBuf,
        #[source]
        source: ConfigError,
    },
    #[error("the replica's configuration does not write; this is a bug in xtask")]
    Derive,
    #[error("production does not answer on {0}; a replica copies a running production")]
    ProductionDown(SocketAddr),
    #[error("production's backup failed; see `journalctl --user -u ren`")]
    Backup(PathBuf),
    #[error(
        "{name} did not answer on {address} in time; see `journalctl --user -u 'ren-{name}-*'`"
    )]
    NotReady { name: String, address: SocketAddr },
    #[error("Tailscale reports no MagicDNS name for this node")]
    NoDnsName,
    #[error("cannot write the release: {0}")]
    Json(serde_json::Error),
    #[error(transparent)]
    Invocation(#[from] InvocationError),
    #[error(transparent)]
    File(#[from] FileError),
    #[error(transparent)]
    Tailscale(#[from] TailscaleError),
    #[error(transparent)]
    Ui(#[from] UiBuildError),
    #[error(transparent)]
    Dev(#[from] Box<DevError>),
}

impl From<DevError> for ReplicaError {
    fn from(error: DevError) -> Self {
        Self::Dev(Box::new(error))
    }
}

/// Where a started replica is reached.
pub struct Reached {
    /// Its gateway, as clients reach it.
    pub gateway: String,
    /// Its UI's origin.
    pub ui: String,
}

impl ProductionPlace {
    pub fn new(env: &HostEnv, kind: Production) -> Result<Self, ReplicaError> {
        let home = env
            .absolute_path("HOME")
            .ok_or(ReplicaError::MissingEnv("HOME"))?;
        match kind {
            Production::Installed => {
                let config = home.join(".config/ren/ren.toml");
                let listen = Config::load(Some(&config))
                    .map_err(|source| ReplicaError::Config {
                        path: config.clone(),
                        source,
                    })?
                    .listen;
                Ok(Self {
                    kind,
                    config,
                    data_dir: home.join(".local/state/ren"),
                    binary: home.join(".local/bin/ren"),
                    listen,
                })
            }
            Production::Dev => {
                let slot = Slot::new(env)?;
                Ok(Self {
                    kind,
                    config: slot.config,
                    data_dir: slot.data_dir,
                    binary: slot.bin_dir.join("ren"),
                    listen: Slot::DAEMON,
                })
            }
        }
    }

    /// Where production's daemon listens.
    pub fn listen(&self) -> SocketAddr {
        self.listen
    }

    fn ren(&self) -> Invocation {
        Invocation::new(&self.binary)
            .arg("--config")
            .arg(&self.config)
            .arg("--data-dir")
            .arg(&self.data_dir)
    }

    /// The build production runs, as its `/health` reports it.
    pub fn revision(&self) -> Result<Revision, ReplicaError> {
        let health = Invocation::new("curl")
            .args(["--silent", "--max-time", "10"])
            .arg(format!("http://{}/health", self.listen))
            .capture()?;
        let health: serde_json::Value =
            serde_json::from_str(&health).map_err(|_| ReplicaError::ProductionDown(self.listen))?;
        health["build"]["revision"]
            .as_str()
            .and_then(|text| text.parse().ok())
            .ok_or(ReplicaError::ProductionDown(self.listen))
    }

    /// A fresh backup of production's history, taken by the running daemon;
    /// its folder holds `ren.sqlite3` and `blobs.sqlite3`.
    pub fn backup(&self) -> Result<PathBuf, ReplicaError> {
        Progress("Backing up production's history").show();
        if !self
            .ren()
            .args(["control", "backup"])
            .succeeds(Echo::Show)?
        {
            return Err(ReplicaError::Backup(self.data_dir.clone()));
        }
        Ok(self.data_dir.join("backup"))
    }

    /// The release the factory last wrote here, if it reads.
    pub fn release(&self) -> Option<Release> {
        let text = fs::read(self.data_dir.join(Release::FILE)).ok()?;
        serde_json::from_slice(&text).ok()
    }

    /// Writes the release where production's gateway serves it from.
    pub fn write_release(&self, release: &Release) -> Result<(), ReplicaError> {
        let json = serde_json::to_vec_pretty(release).map_err(ReplicaError::Json)?;
        Ok(FileMode::Private.write(&self.data_dir.join(Release::FILE), &json)?)
    }

    /// Production's configuration, which a replica's is derived from.
    fn config(&self) -> Result<Config, ReplicaError> {
        Config::load(Some(&self.config)).map_err(|source| ReplicaError::Config {
            path: self.config.clone(),
            source,
        })
    }
}

impl Replica {
    /// What each operator key of a replica's configuration reads: no key.
    const PLACEHOLDER_KEY: &str = "ren-replica-has-no-operator-key";
    const LOOPBACK: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);

    /// The canary: loopback `17787` and `17000`, published on `9445` and
    /// `9446`, in `~/.config/ren-canary` and `~/.local/state/ren-canary`.
    pub fn canary(env: &HostEnv) -> Result<Self, ReplicaError> {
        let home = env
            .absolute_path("HOME")
            .ok_or(ReplicaError::MissingEnv("HOME"))?;
        Ok(Self {
            name: "canary".to_owned(),
            environment: Environment::Canary,
            daemon: SocketAddr::new(Self::LOOPBACK, 17787),
            ui: SocketAddr::new(Self::LOOPBACK, 17000),
            config: home.join(".config/ren-canary/ren.toml"),
            data_dir: home.join(".local/state/ren-canary"),
        })
    }

    pub fn daemon(&self) -> SocketAddr {
        self.daemon
    }

    pub fn ui(&self) -> SocketAddr {
        self.ui
    }

    /// Its units, in the order they stop: every one named `ren-<name>-`.
    pub fn units(&self) -> [TransientUnit; 2] {
        [
            TransientUnit {
                name: format!("ren-{}-ui", self.name),
                description: format!("Ren {}: the UI", self.name),
                kill: KillMode::ControlGroup,
            },
            TransientUnit {
                name: format!("ren-{}-daemon", self.name),
                description: format!("Ren {}: the daemon", self.name),
                kill: KillMode::Mixed,
            },
        ]
    }

    /// Its configuration, from production's.
    fn derive(&self, production: Config) -> Config {
        Config {
            listen: self.daemon,
            data_dir: None,
            instance: InstanceOptions {
                environment: self.environment,
            },
            // Only production runs the factory: a replica drives no runs.
            runs: RunsOptions::default(),
            ..production
        }
    }

    /// Stops the replica and replaces its data with `backup`'s history.
    pub fn replace_data(&self, backup: &Path) -> Result<(), ReplicaError> {
        self.stop()?;
        if self.data_dir.exists() {
            fs::remove_dir_all(&self.data_dir).map_err(FileError::at(&self.data_dir))?;
        }
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&self.data_dir)
            .map_err(FileError::at(&self.data_dir))?;
        for file in ["ren.sqlite3", "blobs.sqlite3"] {
            let (from, to) = (backup.join(file), self.data_dir.join(file));
            fs::copy(&from, &to).map_err(FileError::at(&from))?;
        }
        Ok(())
    }

    /// Writes its configuration, from production's, and returns the
    /// operator key variables it names.
    pub fn configure(&self, production: &ProductionPlace) -> Result<Vec<String>, ReplicaError> {
        let config = self.derive(production.config()?);
        let keys = config
            .upstreams
            .iter()
            .map(|upstream| upstream.operator_key_env.clone())
            .collect();
        config.check().map_err(|source| ReplicaError::Config {
            path: self.config.clone(),
            source,
        })?;
        let text = toml::to_string(&config).map_err(|_| ReplicaError::Derive)?;
        let dir = self.config.parent().unwrap_or(Path::new("/"));
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)
            .map_err(FileError::at(dir))?;
        FileMode::Private.write(
            &self.config,
            format!("# Generated by xtask from production's; rewritten on every start.\n{text}")
                .as_bytes(),
        )?;
        Ok(keys)
    }

    /// Starts `ren` as its daemon on the copied history, then the UI in
    /// `ui_dir`, read-only, published on the tailnet on `publish`'s ports
    /// (the gateway's, then the UI's) when given.
    pub fn start(
        &self,
        env: &HostEnv,
        ren: &Path,
        ui_dir: &Path,
        keys: &[String],
        publish: Option<(TailscaleCli, u16, u16)>,
    ) -> Result<Reached, ReplicaError> {
        self.stop()?;
        let [ui_unit, daemon_unit] = self.units();
        Progress(&format!(
            "Starting {}'s daemon on {}",
            self.name, self.daemon
        ))
        .show();
        let environment: Vec<(&str, String)> = keys
            .iter()
            .map(|key| (key.as_str(), Self::PLACEHOLDER_KEY.to_owned()))
            .collect();
        daemon_unit.start(
            None,
            &[ren],
            &[
                "--config".into(),
                self.config.display().to_string(),
                "--data-dir".into(),
                self.data_dir.display().to_string(),
                "serve".into(),
            ],
            &environment,
        )?;
        self.answering(self.daemon)?;
        let reached = match publish {
            None => Reached {
                gateway: format!("http://{}", self.daemon),
                ui: format!("http://{}", self.ui),
            },
            Some((tailscale, gateway_port, _)) => {
                let dns = tailscale
                    .publish(gateway_port, &format!("http://{}", self.daemon))?
                    .ok_or(ReplicaError::NoDnsName)?;
                let ui_port = publish.map_or(0, |(_, _, ui)| ui);
                Reached {
                    gateway: format!("https://{dns}:{gateway_port}"),
                    ui: format!("https://{dns}:{ui_port}"),
                }
            }
        };
        Progress(&format!("Starting {}'s UI on {}", self.name, self.ui)).show();
        let node = UiBuild::node_on(env)?;
        ui_unit.start(
            Some(ui_dir),
            &[&node],
            &[UiBuild::entry().display().to_string()],
            &[
                ("HOST", self.ui.ip().to_string()),
                ("PORT", self.ui.port().to_string()),
                ("REN_GATEWAY_URL", format!("http://{}", self.daemon)),
                ("REN_PUBLIC_GATEWAY_URL", reached.gateway.clone()),
                ("ORIGIN", reached.ui.clone()),
            ],
        )?;
        self.answering(self.ui)?;
        if let Some((tailscale, _, ui_port)) = publish {
            tailscale.publish(ui_port, &format!("http://{}", self.ui))?;
        }
        Ok(reached)
    }

    fn answering(&self, address: SocketAddr) -> Result<(), ReplicaError> {
        if Listener(address).answers_soon("/health") {
            Ok(())
        } else {
            Err(ReplicaError::NotReady {
                name: self.name.clone(),
                address,
            })
        }
    }

    /// `ren` at `binary`, against the replica.
    pub fn ren(&self, binary: &Path) -> Invocation {
        Invocation::new(binary)
            .arg("--config")
            .arg(&self.config)
            .arg("--data-dir")
            .arg(&self.data_dir)
    }

    pub fn stop(&self) -> Result<(), ReplicaError> {
        for unit in self.units() {
            unit.stop()?;
        }
        Ok(())
    }

    /// Removes its Serve routes on `ports` (the gateway's, then the UI's),
    /// each only while it still points at this replica.
    pub fn unpublish(
        &self,
        tailscale: TailscaleCli,
        ports: (u16, u16),
    ) -> Result<(), ReplicaError> {
        Ok(tailscale.unpublish(&[
            (ports.0, format!("http://{}", self.daemon)),
            (ports.1, format!("http://{}", self.ui)),
        ])?)
    }
}

#[cfg(test)]
mod tests {
    use super::Replica;
    use ren_daemon::config::Config;
    use ren_gateway::Environment;
    use std::path::Path;

    fn production() -> Config {
        toml::from_str(
            r#"
listen = "127.0.0.1:8787"

[instance]
environment = "production"

[runs]
roots = ["/home/op/dev"]

[[upstreams]]
id = "team-key"
provider = "anthropic"
base_url = "https://api.anthropic.com"
operator_key_env = "ACME_ANTHROPIC_KEY"
"#,
        )
        .unwrap()
    }

    #[test]
    fn a_replica_takes_production_settings_but_its_own_listener_and_no_runs() {
        let home = Path::new("/home/op");
        let env = ren_harness::host::HostEnv::from_pairs([("HOME", home.to_str().unwrap())]);
        let canary = Replica::canary(&env).unwrap();
        let config = canary.derive(production());
        assert_eq!(config.listen, canary.daemon());
        assert_eq!(config.instance.environment, Environment::Canary);
        assert!(config.runs.roots.is_empty());
        assert_eq!(config.upstreams[0].operator_key_env, "ACME_ANTHROPIC_KEY");
        config.check().unwrap();
    }

    #[test]
    fn every_unit_is_named_for_its_replica() {
        let env = ren_harness::host::HostEnv::from_pairs([("HOME", "/home/op")]);
        let names: Vec<String> = Replica::canary(&env)
            .unwrap()
            .units()
            .into_iter()
            .map(|unit| unit.name)
            .collect();
        assert_eq!(names, ["ren-canary-ui", "ren-canary-daemon"]);
    }
}
