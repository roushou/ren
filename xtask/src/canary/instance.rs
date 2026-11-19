//! The two instances a canary pass touches: production, which it copies
//! (a backup, through production's own `ren`) and promotes into, and the
//! canary, a build of `main` started on that copy with a configuration
//! derived from production's: its own loopback ports, data folder, and
//! `[instance] environment = "canary"`, no runs, and a placeholder for each
//! operator key, so it forwards no operator-keyed call.

use super::CanaryError;
use crate::{
    builds::StoredBuild,
    dev::slot::Slot,
    file::{FileError, FileMode},
    invocation::{Echo, Invocation},
    progress::Progress,
    tailscale::TailscaleCli,
    ui::UiBuild,
    unit::{KillMode, Listener, TransientUnit},
};
use ren_daemon::config::{Config, RunsOptions};
use ren_gateway::{Environment, InstanceOptions};
use ren_harness::host::HostEnv;
use ren_release::{Release, Revision};
use std::{
    fs,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    os::unix::fs::DirBuilderExt,
    path::{Path, PathBuf},
};

/// Which instance is production to the canary: the one it copies, writes
/// the release to, and promotes into.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Production {
    /// The installed service (`ren.service`), which promotions install into.
    Installed,
    /// The dev slot, with its fake history: for trying the canary out. It
    /// is never promoted into.
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

/// Where the canary lives and listens.
pub struct CanaryInstance {
    config: PathBuf,
    data_dir: PathBuf,
    /// Its pages, captured by its checks, kept past the next pass's wipe.
    pub shots: PathBuf,
}

/// One process of the canary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CanaryUnit {
    Daemon,
    Ui,
}

impl CanaryUnit {
    /// Every unit, in the order they stop.
    pub const ALL: [Self; 2] = [Self::Ui, Self::Daemon];

    pub fn unit(self) -> TransientUnit {
        match self {
            Self::Daemon => TransientUnit {
                name: "ren-canary-daemon",
                description: "Ren canary: the daemon",
                kill: KillMode::Mixed,
            },
            Self::Ui => TransientUnit {
                name: "ren-canary-ui",
                description: "Ren canary: the UI",
                kill: KillMode::ControlGroup,
            },
        }
    }
}

impl ProductionPlace {
    pub fn new(env: &HostEnv, kind: Production) -> Result<Self, CanaryError> {
        let home = env
            .absolute_path("HOME")
            .ok_or(CanaryError::MissingEnv("HOME"))?;
        match kind {
            Production::Installed => {
                let config = home.join(".config/ren/ren.toml");
                let listen = Config::load(Some(&config))
                    .map_err(|source| CanaryError::Config {
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
    pub fn revision(&self) -> Result<Revision, CanaryError> {
        let health = Invocation::new("curl")
            .args(["--silent", "--max-time", "10"])
            .arg(format!("http://{}/health", self.listen))
            .capture()?;
        let health: serde_json::Value =
            serde_json::from_str(&health).map_err(|_| CanaryError::ProductionDown(self.listen))?;
        health["build"]["revision"]
            .as_str()
            .and_then(|text| text.parse().ok())
            .ok_or(CanaryError::ProductionDown(self.listen))
    }

    /// A fresh backup of production's history, taken by the running daemon;
    /// its folder holds `ren.sqlite3` and `blobs.sqlite3`.
    pub fn backup(&self) -> Result<PathBuf, CanaryError> {
        Progress("Backing up production's history for the canary").show();
        if !self
            .ren()
            .args(["control", "backup"])
            .succeeds(Echo::Show)?
        {
            return Err(CanaryError::Backup(self.data_dir.clone()));
        }
        Ok(self.data_dir.join("backup"))
    }

    /// The release the factory last wrote here, if it reads.
    pub fn release(&self) -> Option<Release> {
        let text = fs::read(self.data_dir.join(Release::FILE)).ok()?;
        serde_json::from_slice(&text).ok()
    }

    /// Writes the release where production's gateway serves it from.
    pub fn write_release(&self, release: &Release) -> Result<(), CanaryError> {
        let json = serde_json::to_vec_pretty(release).map_err(CanaryError::Json)?;
        Ok(FileMode::Private.write(&self.data_dir.join(Release::FILE), &json)?)
    }

    /// Production's configuration, which the canary's is derived from.
    fn config(&self) -> Result<Config, CanaryError> {
        Config::load(Some(&self.config)).map_err(|source| CanaryError::Config {
            path: self.config.clone(),
            source,
        })
    }
}

impl CanaryInstance {
    pub const DAEMON: SocketAddr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 17787);
    pub const UI: SocketAddr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 17000);
    /// The tailnet HTTPS ports of the canary, with `--publish`, beside the
    /// installed service's and the dev slot's (9443 and 9444).
    pub const GATEWAY_PORT: u16 = 9445;
    pub const UI_PORT: u16 = 9446;
    /// What each operator key of the canary's configuration reads: no key.
    const PLACEHOLDER_KEY: &str = "ren-canary-has-no-operator-key";

    pub fn new(env: &HostEnv) -> Result<Self, CanaryError> {
        let home = env
            .absolute_path("HOME")
            .ok_or(CanaryError::MissingEnv("HOME"))?;
        Ok(Self {
            config: home.join(".config/ren-canary/ren.toml"),
            data_dir: home.join(".local/state/ren-canary"),
            shots: home.join(".local/share/ren-canary/shots"),
        })
    }

    /// The canary's configuration, from production's.
    fn derive(production: Config) -> Config {
        Config {
            listen: Self::DAEMON,
            data_dir: None,
            instance: InstanceOptions {
                environment: Environment::Canary,
            },
            // Only production runs the factory: the canary drives no runs.
            runs: RunsOptions::default(),
            ..production
        }
    }

    /// Stops the canary and replaces its data with `backup`'s history.
    pub fn replace_data(&self, backup: &Path) -> Result<(), CanaryError> {
        self.stop()?;
        // Only the canary's own folder is removed.
        if self.data_dir.ends_with(".local/state/ren-canary") && self.data_dir.exists() {
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

    /// Writes the canary's configuration, from production's.
    pub fn configure(&self, production: &ProductionPlace) -> Result<Vec<String>, CanaryError> {
        let config = Self::derive(production.config()?);
        let keys = config
            .upstreams
            .iter()
            .map(|upstream| upstream.operator_key_env.clone())
            .collect();
        config.check().map_err(|source| CanaryError::Config {
            path: self.config.clone(),
            source,
        })?;
        let text = toml::to_string(&config).map_err(|_| CanaryError::Derive)?;
        let dir = self.config.parent().unwrap_or(Path::new("/"));
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)
            .map_err(FileError::at(dir))?;
        FileMode::Private.write(
            &self.config,
            format!("# Generated by `cargo xtask canary` from production's; rewritten on every pass.\n{text}")
                .as_bytes(),
        )?;
        Ok(keys)
    }

    /// Starts `build` as the canary: its daemon on the copied history, then
    /// its UI, read-only, and publishes both with `publish`.
    pub fn start(
        &self,
        env: &HostEnv,
        build: &StoredBuild,
        keys: &[String],
        publish: Option<TailscaleCli>,
    ) -> Result<(), CanaryError> {
        self.stop()?;
        Progress(&format!("Starting the canary's daemon on {}", Self::DAEMON)).show();
        let ren = build.ren();
        let environment: Vec<(&str, String)> = keys
            .iter()
            .map(|key| (key.as_str(), Self::PLACEHOLDER_KEY.to_owned()))
            .collect();
        CanaryUnit::Daemon.unit().start(
            None,
            &[&ren],
            &[
                "--config".into(),
                self.config.display().to_string(),
                "--data-dir".into(),
                self.data_dir.display().to_string(),
                "serve".into(),
            ],
            &environment,
        )?;
        if !Listener(Self::DAEMON).answers_soon("/health") {
            return Err(CanaryError::NotReady(Self::DAEMON));
        }
        let (public_gateway, origin) = match publish {
            None => (
                format!("http://{}", Self::DAEMON),
                format!("http://{}", Self::UI),
            ),
            Some(tailscale) => {
                let dns = tailscale
                    .publish(Self::GATEWAY_PORT, &format!("http://{}", Self::DAEMON))?
                    .ok_or(CanaryError::NoDnsName)?;
                (
                    format!("https://{dns}:{}", Self::GATEWAY_PORT),
                    format!("https://{dns}:{}", Self::UI_PORT),
                )
            }
        };
        Progress(&format!("Starting the canary's UI on {}", Self::UI)).show();
        let ui = build.ui();
        let node = UiBuild::new(env, Path::new("."))?;
        CanaryUnit::Ui.unit().start(
            Some(&ui),
            &[node.node()],
            &[UiBuild::entry().display().to_string()],
            &[
                ("HOST", Self::UI.ip().to_string()),
                ("PORT", Self::UI.port().to_string()),
                ("REN_GATEWAY_URL", format!("http://{}", Self::DAEMON)),
                ("REN_PUBLIC_GATEWAY_URL", public_gateway),
                ("ORIGIN", origin.clone()),
            ],
        )?;
        if !Listener(Self::UI).answers_soon("/health") {
            return Err(CanaryError::NotReady(Self::UI));
        }
        if let Some(tailscale) = publish {
            tailscale.publish(Self::UI_PORT, &format!("http://{}", Self::UI))?;
        }
        println!("\nCanary: {origin}\n");
        Ok(())
    }

    /// `ren` of `build`, against the canary.
    pub fn ren(&self, build: &StoredBuild) -> Invocation {
        Invocation::new(build.ren())
            .arg("--config")
            .arg(&self.config)
            .arg("--data-dir")
            .arg(&self.data_dir)
    }

    pub fn stop(&self) -> Result<(), CanaryError> {
        for unit in CanaryUnit::ALL {
            unit.unit().stop()?;
        }
        Ok(())
    }

    /// Removes the canary's Serve routes, each only while it still points
    /// at the canary.
    pub fn unpublish(tailscale: TailscaleCli) -> Result<(), CanaryError> {
        Ok(tailscale.unpublish(&[
            (Self::GATEWAY_PORT, format!("http://{}", Self::DAEMON)),
            (Self::UI_PORT, format!("http://{}", Self::UI)),
        ])?)
    }
}

#[cfg(test)]
mod tests {
    use super::{CanaryInstance, CanaryUnit};
    use ren_daemon::config::Config;
    use ren_gateway::Environment;

    #[test]
    fn the_canary_takes_production_settings_but_its_own_listener_and_no_runs() {
        let production: Config = toml::from_str(
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
        .unwrap();
        let canary = CanaryInstance::derive(production);
        assert_eq!(canary.listen, CanaryInstance::DAEMON);
        assert_eq!(canary.instance.environment, Environment::Canary);
        assert!(canary.runs.roots.is_empty());
        assert_eq!(canary.upstreams.len(), 1);
        assert_eq!(canary.upstreams[0].operator_key_env, "ACME_ANTHROPIC_KEY");
        canary.check().unwrap();
    }

    #[test]
    fn every_unit_is_named_for_the_canary() {
        for unit in CanaryUnit::ALL {
            assert!(unit.unit().name.starts_with("ren-canary-"));
        }
    }
}
