//! Ren on a copy of production's history: production, which a replica
//! copies (a backup, through production's own `ren`), and a [`Replica`], a
//! build started on that copy with a configuration derived from
//! production's: its own loopback ports and data folder, its own
//! `[instance] environment`, no runs, and a placeholder for each operator
//! key, so it forwards no operator-keyed call. Its UI only reads. The
//! canary is one (`cargo xtask canary`); each workspace's preview is
//! another (`cargo xtask preview`).

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
use ren_gateway::{
    ControlToken, Environment, InstanceOptions,
    api::control::request::{ControlRequest, WorkChange},
};
use ren_harness::host::HostEnv;
use ren_protocol::label::Stored;
use ren_recorder::{
    id::WorkItemId,
    work::{PreviewUrl, WorkStateKind},
};
use ren_release::{Commit, PromotionRequest, Release, Revision, ShotName};
use serde::Deserialize;
use std::{
    fs,
    io::Write as _,
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
    #[error("{} is not a replica's data folder; it is not removed", .0.display())]
    NotOurs(PathBuf),
    #[error("cannot write the release: {0}")]
    Json(serde_json::Error),
    #[error("production's work items: {0}")]
    WorkAnswer(String),
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

/// One work item, as production's `/api/work` lists it, as far as xtask
/// needs.
#[derive(Debug, Deserialize)]
pub struct ListedWork {
    pub id: WorkItemId,
    pub state: Stored<WorkStateKind>,
    pub landed: Option<Commit>,
    pub preview: Option<ListedPreview>,
}

/// A listed item's preview.
#[derive(Debug, Deserialize)]
pub struct ListedPreview {
    pub ui: PreviewUrl,
}

#[derive(Deserialize)]
struct WorkListing {
    work: Vec<ListedWork>,
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

    /// The work items under way, or none when production's daemon does
    /// not serve work items.
    pub fn work_items(&self) -> Result<Option<Vec<ListedWork>>, ReplicaError> {
        let answer = Invocation::new("curl")
            .args([
                "--silent",
                "--max-time",
                "20",
                "--write-out",
                "\n%{http_code}",
            ])
            .arg(format!("http://{}/api/work?limit=1000", self.listen))
            .capture()?;
        let (body, status) = answer.rsplit_once('\n').unwrap_or(("", answer.as_str()));
        match status.trim() {
            "200" => serde_json::from_str::<WorkListing>(body)
                .map(|listing| Some(listing.work))
                .map_err(|_| ReplicaError::WorkAnswer("/api/work".to_owned())),
            "404" => Ok(None),
            other => Err(ReplicaError::WorkAnswer(format!(
                "/api/work answered {other}"
            ))),
        }
    }

    /// Moves `item` on through production's control API, with its control
    /// token, as an operator would; production checks it against the
    /// item's state.
    pub fn move_work(&self, item: WorkItemId, change: WorkChange) -> Result<(), ReplicaError> {
        let request = ControlRequest::Work { item, change };
        let path = self.data_dir.join(ControlToken::FILE);
        let token = fs::read_to_string(&path)
            .ok()
            .and_then(|text| ControlToken::parse(&text))
            .ok_or_else(|| {
                ReplicaError::WorkAnswer(format!("no control token in {}", path.display()))
            })?;
        // The token goes in a private file curl reads, never on its command
        // line, where any process could read it.
        let mut header = tempfile::Builder::new()
            .prefix(".ren-work.")
            .tempfile()
            .map_err(FileError::at(&path))?;
        writeln!(header, "authorization: Bearer {}", token.expose())
            .map_err(FileError::at(&path))?;
        let body = request
            .body()
            .map_err(ReplicaError::Json)?
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
            .unwrap_or_default();
        let answer = Invocation::new("curl")
            .args([
                "--silent",
                "--max-time",
                "20",
                "--write-out",
                "\n%{http_code}",
            ])
            .args(["-X", "POST", "-H", "content-type: application/json"])
            .arg("-H")
            .arg(format!("@{}", header.path().display()))
            .arg("--data")
            .arg(body)
            .arg(format!(
                "http://{}/control/v1/{}",
                self.listen,
                request.path()
            ))
            .capture()?;
        let status = answer
            .rsplit_once('\n')
            .map_or(answer.as_str(), |(_, status)| status);
        match status.trim() {
            "202" => Ok(()),
            other => Err(ReplicaError::WorkAnswer(format!(
                "{} answered {other}",
                request.path()
            ))),
        }
    }

    /// Keeps the pages the canary captured in `shots` where the Release view
    /// reads them, in place of the last pass's.
    pub fn keep_shots(&self, shots: &Path) -> Result<(), ReplicaError> {
        let kept = self.data_dir.join(ShotName::FOLDER);
        if kept.exists() {
            fs::remove_dir_all(&kept).map_err(FileError::at(&kept))?;
        }
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&kept)
            .map_err(FileError::at(&kept))?;
        let Ok(entries) = fs::read_dir(shots) else {
            return Ok(());
        };
        for entry in entries.filter_map(Result::ok) {
            let name = entry.file_name();
            if name.to_str().and_then(ShotName::of_file).is_some() {
                fs::copy(entry.path(), kept.join(&name)).map_err(FileError::at(&entry.path()))?;
            }
        }
        Ok(())
    }

    /// The operator's promotion request, if one waits, taken: it is
    /// carried out or dropped by the pass that reads it.
    pub fn take_request(&self) -> Option<PromotionRequest> {
        let path = self.data_dir.join(PromotionRequest::FILE);
        let request = fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok());
        let _ = fs::remove_file(&path);
        request
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

    /// The preview of the workspace `name`: its daemon on the workspace's
    /// first port and its UI on the next, in `<data>/preview`.
    pub fn preview(name: &str, first_port: u16, data: &Path) -> Self {
        let root = data.join("preview");
        Self {
            name: format!("preview-{name}"),
            environment: Environment::Preview,
            daemon: SocketAddr::new(Self::LOOPBACK, first_port),
            ui: SocketAddr::new(Self::LOOPBACK, first_port.saturating_add(1)),
            config: root.join("ren.toml"),
            data_dir: root.join("data"),
        }
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
    /// Only a data folder a replica names (the canary's, or a preview's
    /// `preview/data`) is ever removed.
    pub fn replace_data(&self, backup: &Path) -> Result<(), ReplicaError> {
        let ours = self.data_dir.ends_with(".local/state/ren-canary")
            || self.data_dir.ends_with("preview/data");
        if !ours {
            return Err(ReplicaError::NotOurs(self.data_dir.clone()));
        }
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

        let preview = Replica::preview("ren-3", 20_200, Path::new("/data/worktrees/.data/ren-3"));
        let config = preview.derive(production());
        assert_eq!(config.listen.port(), 20_200);
        assert_eq!(preview.ui().port(), 20_201);
        assert_eq!(config.instance.environment, Environment::Preview);
        assert_eq!(
            preview.data_dir,
            Path::new("/data/worktrees/.data/ren-3/preview/data")
        );
        // A data folder no replica names is never removed.
        let stray = Replica {
            data_dir: Path::new("/home/op").to_path_buf(),
            ..Replica::preview("ren-3", 20_200, Path::new("/data/worktrees/.data/ren-3"))
        };
        assert!(matches!(
            stray.replace_data(Path::new("/tmp/backup")),
            Err(super::ReplicaError::NotOurs(_))
        ));
    }

    #[test]
    fn a_preview_runs_as_the_units_its_workspace_stops() {
        let workspace = ren_supervisor::Workspace::of(
            Path::new("/data/worktrees"),
            Path::new("/data/worktrees/ren-3"),
        )
        .unwrap();
        let preview = Replica::preview("ren-3", 20_200, &workspace.data());
        let names = preview.units().map(|unit| unit.name);
        assert_eq!(names, workspace.preview_units()[..2]);
    }

    #[test]
    fn every_unit_is_named_for_its_replica() {
        let preview = Replica::preview("acme-web-2", 20_100, Path::new("/tmp/acme"));
        let names: Vec<String> = preview.units().into_iter().map(|unit| unit.name).collect();
        assert_eq!(
            names,
            ["ren-preview-acme-web-2-ui", "ren-preview-acme-web-2-daemon"]
        );
    }
}
