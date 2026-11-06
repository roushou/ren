//! Which gateway this machine's clients use: `--gateway`, then
//! `TGV_GATEWAY`, then `client.toml`, then the configured local listener.
//!
//! `client.toml` lives in `$XDG_CONFIG_HOME/tgv` (or `~/.config/tgv`) and is
//! written by `tgv init`. It is separate from the daemon's `tgv.toml`: it says
//! where this machine sends traffic, not how a daemon runs.
use serde::{Deserialize, Serialize};
use std::{
    fmt,
    io::Write as _,
    net::SocketAddr,
    path::{Path, PathBuf},
};
use tgv_harness::{
    endpoint::{GatewayEndpoint, GatewayEndpointError},
    host::HostEnv,
};

/// The environment variable that names a gateway root URL.
pub const GATEWAY_ENV: &str = "TGV_GATEWAY";

/// Why the client settings could not be read, chosen, or saved. Values are
/// never echoed: a URL can carry credentials.
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    // The reason is part of the message, not a separate cause, so it is
    // printed once.
    #[error("{reason} (from {origin})")]
    Gateway {
        origin: GatewaySource,
        reason: GatewayEndpointError,
    },
    #[error("cannot locate client settings: set HOME or XDG_CONFIG_HOME to an absolute path")]
    NoConfigDir,
    #[error("cannot read {}", path.display())]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid TOML or unknown field in {}", path.display())]
    Syntax { path: PathBuf },
    #[error("cannot write {}", path.display())]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// The file's representation; parsed into [`ClientConfig`] on load.
#[derive(Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ClientFile {
    #[serde(skip_serializing_if = "Option::is_none")]
    gateway: Option<String>,
}

/// This machine's client settings, as `client.toml` holds them.
pub struct ClientConfig {
    path: PathBuf,
    gateway: Option<GatewayEndpoint>,
}

impl ClientConfig {
    /// Where `client.toml` lives for this environment.
    pub fn locate(env: &HostEnv) -> Result<PathBuf, ClientError> {
        env.absolute_path("XDG_CONFIG_HOME")
            .or_else(|| env.absolute_path("HOME").map(|home| home.join(".config")))
            .map(|dir| dir.join("tgv").join("client.toml"))
            .ok_or(ClientError::NoConfigDir)
    }

    /// The settings at `path`; a missing file means no settings.
    pub fn load(path: PathBuf) -> Result<Self, ClientError> {
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self {
                    path,
                    gateway: None,
                });
            }
            Err(source) => return Err(ClientError::Read { path, source }),
        };
        // TOML diagnostics quote the offending value, which may be a secret.
        let Ok(file) = toml::from_str::<ClientFile>(&text) else {
            return Err(ClientError::Syntax { path });
        };
        let gateway = file
            .gateway
            .as_deref()
            .map(GatewayEndpoint::parse)
            .transpose()
            .map_err(|reason| ClientError::Gateway {
                origin: GatewaySource::ClientConfig(path.clone()),
                reason,
            })?;
        Ok(Self { path, gateway })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn gateway(&self) -> Option<&GatewayEndpoint> {
        self.gateway.as_ref()
    }

    #[must_use]
    pub fn with_gateway(self, gateway: GatewayEndpoint) -> Self {
        Self {
            gateway: Some(gateway),
            ..self
        }
    }

    /// Writes the settings atomically, owner-only, creating the directory.
    pub fn save(&self) -> Result<(), ClientError> {
        let write = |source| ClientError::Write {
            path: self.path.clone(),
            source,
        };
        let dir = self.path.parent().unwrap_or(Path::new("."));
        {
            use std::os::unix::fs::DirBuilderExt;
            std::fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(dir)
                .map_err(write)?;
        }
        let file = ClientFile {
            gateway: self
                .gateway
                .as_ref()
                .map(|gateway| gateway.root().to_owned()),
        };
        let text = toml::to_string(&file).map_err(|error| write(std::io::Error::other(error)))?;
        // tempfile creates the file with mode 0600.
        let mut staged = tempfile::NamedTempFile::new_in(dir).map_err(write)?;
        staged.write_all(text.as_bytes()).map_err(write)?;
        staged.as_file().sync_all().map_err(write)?;
        staged
            .persist(&self.path)
            .map_err(|error| write(error.error))?;
        Ok(())
    }
}

/// Where a selected gateway came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GatewaySource {
    Flag,
    Environment,
    ClientConfig(PathBuf),
}

impl fmt::Display for GatewaySource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Flag => f.write_str("--gateway"),
            Self::Environment => f.write_str(GATEWAY_ENV),
            Self::ClientConfig(path) => write!(f, "{}", path.display()),
        }
    }
}

/// The gateway a command talks to.
pub enum GatewayChoice {
    /// A gateway named by the flag, the environment, or `client.toml`.
    Selected {
        endpoint: GatewayEndpoint,
        source: GatewaySource,
    },
    /// Nothing names one: the local daemon's configured listener.
    LocalListener,
}

impl GatewayChoice {
    /// The first of `flag`, `TGV_GATEWAY`, and `client.toml` that names a
    /// gateway. A `client.toml` that cannot be located or read is an error
    /// only when nothing earlier names a gateway.
    pub fn select(flag: Option<&str>, env: &HostEnv) -> Result<Self, ClientError> {
        let named = |value: &str, source: GatewaySource| match GatewayEndpoint::parse(value) {
            Ok(endpoint) => Ok(Self::Selected { endpoint, source }),
            Err(reason) => Err(ClientError::Gateway {
                origin: source,
                reason,
            }),
        };
        if let Some(value) = flag {
            return named(value, GatewaySource::Flag);
        }
        if let Some(value) = env.get(GATEWAY_ENV) {
            return named(value, GatewaySource::Environment);
        }
        let config = match ClientConfig::locate(env) {
            Ok(path) => ClientConfig::load(path)?,
            Err(ClientError::NoConfigDir) => return Ok(Self::LocalListener),
            Err(error) => return Err(error),
        };
        Ok(match config.gateway {
            Some(endpoint) => Self::Selected {
                endpoint,
                source: GatewaySource::ClientConfig(config.path),
            },
            None => Self::LocalListener,
        })
    }

    /// The gateway to contact, given the local daemon's `listen` address.
    pub fn endpoint(&self, listen: SocketAddr) -> Result<GatewayEndpoint, GatewayEndpointError> {
        match self {
            Self::Selected { endpoint, .. } => Ok(endpoint.clone()),
            Self::LocalListener => GatewayEndpoint::resolve(None, listen),
        }
    }

    /// A one-line description of the choice, for diagnostics.
    pub fn describe(&self) -> String {
        match self {
            Self::Selected { endpoint, source } => format!("{} (from {source})", endpoint.root()),
            Self::LocalListener => {
                "local daemon; no gateway configured (`tgv init` sets one)".to_owned()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ClientConfig, ClientError, GatewayChoice, GatewaySource};
    use tgv_harness::{endpoint::GatewayEndpoint, host::HostEnv};

    fn source(choice: &GatewayChoice) -> Option<GatewaySource> {
        match choice {
            GatewayChoice::Selected { source, .. } => Some(source.clone()),
            GatewayChoice::LocalListener => None,
        }
    }

    #[test]
    fn flag_then_environment_then_file_then_local() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().to_str().unwrap();
        let file = dir.path().join(".config/tgv/client.toml");
        let none = HostEnv::from_pairs([("HOME", home)]);
        assert!(source(&GatewayChoice::select(None, &none).unwrap()).is_none());

        ClientConfig::load(file.clone())
            .unwrap()
            .with_gateway(GatewayEndpoint::parse("https://file.example.ts.net:8443").unwrap())
            .save()
            .unwrap();
        let choice = GatewayChoice::select(None, &none).unwrap();
        assert_eq!(source(&choice), Some(GatewaySource::ClientConfig(file)));
        assert_eq!(
            choice
                .endpoint("127.0.0.1:8787".parse().unwrap())
                .unwrap()
                .root(),
            "https://file.example.ts.net:8443/"
        );

        let env = HostEnv::from_pairs([("HOME", home), ("TGV_GATEWAY", "https://env.example")]);
        assert_eq!(
            source(&GatewayChoice::select(None, &env).unwrap()),
            Some(GatewaySource::Environment)
        );
        assert_eq!(
            source(&GatewayChoice::select(Some("https://flag.example"), &env).unwrap()),
            Some(GatewaySource::Flag)
        );
    }

    #[test]
    fn xdg_config_home_wins_over_home() {
        let env = HostEnv::from_pairs([("HOME", "/home/u"), ("XDG_CONFIG_HOME", "/xdg")]);
        assert_eq!(
            ClientConfig::locate(&env).unwrap(),
            std::path::Path::new("/xdg/tgv/client.toml")
        );
        let relative = HostEnv::from_pairs([("HOME", "/home/u"), ("XDG_CONFIG_HOME", "rel")]);
        assert_eq!(
            ClientConfig::locate(&relative).unwrap(),
            std::path::Path::new("/home/u/.config/tgv/client.toml")
        );
    }

    #[test]
    fn invalid_values_name_their_origin_without_echoing_them() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("client.toml");
        std::fs::write(&file, "gateway = 'https://u:SECRET@example.com'\n").unwrap();
        let error = ClientConfig::load(file.clone()).err().unwrap();
        assert!(matches!(error, ClientError::Gateway { .. }));
        assert!(error.to_string().contains("client.toml"));
        assert!(!error.to_string().contains("SECRET"));

        std::fs::write(&file, "gateway = 'SECRET\n").unwrap();
        let error = ClientConfig::load(file.clone()).err().unwrap();
        assert!(matches!(error, ClientError::Syntax { .. }));
        assert!(!error.to_string().contains("SECRET"));

        std::fs::write(&file, "gatway = 'https://example.com'\n").unwrap();
        assert!(matches!(
            ClientConfig::load(file).err().unwrap(),
            ClientError::Syntax { .. }
        ));

        let env = HostEnv::from_pairs([("TGV_GATEWAY", "http://SECRET.example")]);
        let error = GatewayChoice::select(None, &env).err().unwrap();
        assert!(error.to_string().contains("TGV_GATEWAY"));
        assert!(!error.to_string().contains("SECRET"));
    }

    #[test]
    fn saved_settings_are_owner_only_and_read_back() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("nested/tgv/client.toml");
        ClientConfig::load(file.clone())
            .unwrap()
            .with_gateway(GatewayEndpoint::parse("https://vps.example.ts.net:8443").unwrap())
            .save()
            .unwrap();
        let mode = std::fs::metadata(&file).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            "gateway = \"https://vps.example.ts.net:8443/\"\n"
        );
        let loaded = ClientConfig::load(file).unwrap();
        assert_eq!(
            loaded.gateway().unwrap().root(),
            "https://vps.example.ts.net:8443/"
        );
    }
}
