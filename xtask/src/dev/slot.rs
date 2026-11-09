//! The dev slot: this checkout's build running beside the installed service,
//! with its own configuration, data, and loopback ports, pointed only at the
//! fake provider. Each process runs as a transient systemd user unit named
//! `tgv-dev-*`, so it is stopped by name and never by a pattern that could
//! match the installed service.

use super::DevError;
use crate::{
    install::{InstallError, InstallVps},
    invocation::{Echo, Invocation},
};
use std::{
    fs,
    io::{Read as _, Write as _},
    net::{SocketAddr, TcpStream},
    path::{Path, PathBuf},
    thread,
    time::Duration,
};
use tgv_daemon::config::{Config, UpstreamConfig};
use tgv_gateway::{
    EntryIngress,
    user::{TokenDigest, UserConfig},
};
use tgv_harness::{endpoint::UserToken, host::HostEnv};
use tgv_protocol::route::{Provider, Wire};
use tgv_recorder::{
    caller::{TailscaleLogin, UserId},
    routing::UpstreamId,
};

/// One process of the slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    Provider,
    Daemon,
    Ui,
    Trickle,
}

impl Unit {
    /// Every unit, in the order they stop.
    pub const ALL: [Self; 4] = [Self::Trickle, Self::Ui, Self::Daemon, Self::Provider];

    /// The transient unit's name; every one starts with `tgv-dev-`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Provider => "tgv-dev-provider",
            Self::Daemon => "tgv-dev-daemon",
            Self::Ui => "tgv-dev-ui",
            Self::Trickle => "tgv-dev-trickle",
        }
    }

    fn service(self) -> String {
        format!("{}.service", self.name())
    }

    pub fn active(self) -> Result<bool, InstallError> {
        Invocation::new("systemctl")
            .args(["--user", "is-active", "--quiet"])
            .arg(self.service())
            .succeeds(Echo::Hide)
    }

    pub fn stop(self) -> Result<(), InstallError> {
        if self.active()? {
            Invocation::new("systemctl")
                .args(["--user", "stop"])
                .arg(self.service())
                .run()?;
        }
        // A failed transient unit stays loaded until reset.
        Invocation::new("systemctl")
            .args(["--user", "reset-failed"])
            .arg(self.service())
            .succeeds(Echo::Hide)?;
        Ok(())
    }

    /// Run `command` as this unit, with `environment`, until stopped.
    pub fn start(
        self,
        command: &[&Path],
        args: &[String],
        environment: &[(&str, String)],
    ) -> Result<(), InstallError> {
        let mut run = Invocation::new("systemd-run")
            .args(["--user", "--collect", "--quiet"])
            .arg(format!("--unit={}", self.name()))
            .arg(format!("--description=TGV dev slot: {}", self.name()));
        for (name, value) in environment {
            run = run.arg(format!("--setenv={name}={value}"));
        }
        run.arg("--").args(command).args(args).run()
    }
}

/// Where the slot lives and listens.
pub struct Slot {
    pub repo: PathBuf,
    cargo: PathBuf,
    pub config: PathBuf,
    pub data_dir: PathBuf,
    pub bin_dir: PathBuf,
}

impl Slot {
    pub const PROVIDER: SocketAddr =
        SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), 18999);
    pub const DAEMON: SocketAddr =
        SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), 18787);
    pub const UI: SocketAddr =
        SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), 13000);
    /// The operator key the dev config's operator-key entries read: fake.
    const OPERATOR_KEY_ENV: &'static str = "TGV_DEV_OPERATOR_KEY";
    const OPERATOR_KEY: &'static str = "fake-dev-operator-key";

    pub fn new(env: &HostEnv) -> Result<Self, DevError> {
        let home = env
            .absolute_path("HOME")
            .ok_or(DevError::Command(InstallError::MissingEnv("HOME")))?;
        let share = home.join(".local/share/tgv-dev");
        Ok(Self {
            repo: Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .map_or_else(|| PathBuf::from("."), Path::to_path_buf),
            cargo: env
                .absolute_path("CARGO")
                .unwrap_or_else(|| PathBuf::from("cargo")),
            config: home.join(".config/tgv-dev/tgv.toml"),
            data_dir: home.join(".local/state/tgv-dev"),
            bin_dir: share.join("bin"),
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
        let id = |text: &str| UpstreamId::parse(text).map_err(|_| DevError::Config);
        let user = |text: &str| UserId::parse(text).map_err(|_| DevError::Config);
        let login = TailscaleLogin::parse("alice@example.com").map_err(|_| DevError::Config)?;
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
                    max_concurrent: std::num::NonZeroU32::new(4),
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
            ..Config::default()
        })
    }

    fn step(message: &str) {
        InstallVps::step(message);
    }

    /// Build this checkout and copy the binaries the units run, so a later
    /// build never replaces one under a running unit.
    fn build(&self) -> Result<(), DevError> {
        Self::step("Building tgv and xtask from this checkout");
        Invocation::new(&self.cargo)
            .current_dir(&self.repo)
            .args(["build", "--locked", "-p", "tgv-rs", "-p", "xtask"])
            .run()?;
        fs::create_dir_all(&self.bin_dir).map_err(InstallError::file(&self.bin_dir))?;
        for name in ["tgv", "xtask"] {
            let built = self.repo.join("target/debug").join(name);
            let contents = fs::read(&built).map_err(InstallError::file(&built))?;
            InstallVps::write_private(&self.bin_dir.join(name), &contents, 0o755)?;
        }
        Ok(())
    }

    fn write_config(&self) -> Result<(), DevError> {
        let config = self.config()?;
        config.check().map_err(|_| DevError::Config)?;
        let text = toml::to_string(&config).map_err(|_| DevError::Config)?;
        let dir = self.config.parent().unwrap_or(Path::new("/"));
        fs::create_dir_all(dir).map_err(InstallError::file(dir))?;
        fs::create_dir_all(&self.data_dir).map_err(InstallError::file(&self.data_dir))?;
        InstallVps::write_private(
            &self.config,
            format!("# Generated by `cargo xtask dev up`; rewritten on every run.\n{text}")
                .as_bytes(),
            0o600,
        )?;
        Ok(())
    }

    /// Whether `address` answers `GET /health` (or `GET /` for the UI)
    /// within a few seconds.
    pub fn wait_for(address: SocketAddr, path: &str) -> Result<(), DevError> {
        for _ in 0..100 {
            if Self::answers(address, path) {
                return Ok(());
            }
            thread::sleep(Duration::from_millis(100));
        }
        Err(DevError::NotReady(address))
    }

    fn answers(address: SocketAddr, path: &str) -> bool {
        let Ok(mut stream) = TcpStream::connect_timeout(&address, Duration::from_millis(500))
        else {
            return false;
        };
        let request =
            format!("GET {path} HTTP/1.1\r\nhost: {address}\r\nconnection: close\r\n\r\n");
        let mut head = [0_u8; 12];
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .is_ok()
            && stream.write_all(request.as_bytes()).is_ok()
            && stream.read_exact(&mut head).is_ok()
            && (head.ends_with(b"200") || head.ends_with(b"503"))
    }

    /// Build, configure, and (re)start the provider and daemon.
    pub fn up(&self) -> Result<(), DevError> {
        self.build()?;
        self.write_config()?;
        for unit in [Unit::Daemon, Unit::Provider] {
            unit.stop()?;
        }
        let xtask = self.bin_dir.join("xtask");
        let tgv = self.bin_dir.join("tgv");
        Self::step(&format!("Starting the fake provider on {}", Self::PROVIDER));
        Unit::Provider.start(
            &[&xtask],
            &[
                "dev".into(),
                "provider".into(),
                "--listen".into(),
                Self::PROVIDER.to_string(),
            ],
            &[],
        )?;
        Self::wait_for(Self::PROVIDER, "/anthropic/v1/models")?;
        Self::step(&format!("Starting the dev daemon on {}", Self::DAEMON));
        Unit::Daemon.start(
            &[&tgv],
            &[
                "--config".into(),
                self.config.display().to_string(),
                "serve".into(),
            ],
            &[(Self::OPERATOR_KEY_ENV, Self::OPERATOR_KEY.into())],
        )?;
        Self::wait_for(Self::DAEMON, "/health")
    }

    /// Run `dev seed sessions --forever` as the trickle unit.
    pub fn trickle(&self) -> Result<(), DevError> {
        Self::step("Starting a trickle of fake calls");
        Unit::Trickle.start(
            &[&self.bin_dir.join("xtask")],
            &[
                "dev".into(),
                "seed".into(),
                "sessions".into(),
                "--forever".into(),
            ],
            // The user manager gives its units HOME, which the seeder reads.
            &[],
        )?;
        Ok(())
    }

    pub fn down() -> Result<(), DevError> {
        for unit in Unit::ALL {
            Self::step(&format!("Stopping {}", unit.name()));
            unit.stop()?;
        }
        Ok(())
    }

    /// Stop the slot and remove its data. Only the slot's own directory is
    /// removed, and only if it is the one this slot names.
    pub fn wipe(&self) -> Result<(), DevError> {
        Self::down()?;
        if self.data_dir.ends_with(".local/state/tgv-dev") && self.data_dir.exists() {
            Self::step(&format!("Removing {}", self.data_dir.display()));
            fs::remove_dir_all(&self.data_dir).map_err(InstallError::file(&self.data_dir))?;
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
        println!("config             {}", self.config.display());
        println!("data               {}", self.data_dir.display());
        println!(
            "history            {} --config {} history",
            self.bin_dir.join("tgv").display(),
            self.config.display()
        );
        println!("logs               journalctl --user -u 'tgv-dev-*'");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{Slot, Unit};
    use tgv_harness::host::HostEnv;

    #[test]
    fn every_unit_is_named_for_the_dev_slot() {
        for unit in Unit::ALL {
            assert!(unit.name().starts_with("tgv-dev-"), "{unit:?}");
            assert_ne!(unit.name(), "tgv");
            assert_ne!(unit.name(), "tgv-ui");
        }
    }

    #[test]
    fn the_dev_config_is_valid_and_reaches_only_the_fake_provider() {
        let slot = Slot::new(&HostEnv::from_pairs([("HOME", "/home/dev")])).unwrap();
        let config = slot.config().unwrap();
        config.check().unwrap();
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
            slot.data_dir,
            std::path::Path::new("/home/dev/.local/state/tgv-dev")
        );
        // The token itself is never written, only its digest.
        assert!(!text.contains(Slot::ci_token().expose()));
    }
}
