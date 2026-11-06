//! How `install-vps` publishes TGV, remembered between runs.
//!
//! The choices are saved as the flags that made them, in
//! `~/.config/tgv/install-vps.toml`, and read back through the same clap
//! parser, so there is one vocabulary. A run given any of these flags uses
//! exactly those and saves them after it succeeds; a run given none uses the
//! saved ones.
use crate::install::{InstallError, InstallVps};
use clap::{Args, Parser};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Tailscale Serve publishing flags.
#[derive(Args, Debug, Clone, Default, PartialEq, Eq)]
pub struct Publish {
    /// Publish the listener privately with Tailscale Serve on this HTTPS port
    /// (for example 8443). Without it, Tailscale is left untouched.
    #[arg(long, value_parser = clap::value_parser!(u16).range(1..))]
    pub https_port: Option<u16>,
    /// Use `sudo -n` for the Serve steps only.
    #[arg(long, requires = "https_port")]
    pub sudo_tailscale: bool,
    /// Also build and run the UI as `tgv-ui.service`, published privately
    /// on this HTTPS port (for example 8444). Without it, the UI is left
    /// untouched.
    #[arg(long, requires = "https_port", value_parser = clap::value_parser!(u16).range(1..))]
    pub ui_https_port: Option<u16>,
}

/// Saved flags, parsed as if given on the command line.
#[derive(Parser)]
#[command(name = "install-vps")]
struct Saved {
    #[command(flatten)]
    publish: Publish,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedFile {
    args: Vec<String>,
}

impl Publish {
    /// Whether any publishing flag was given.
    fn given(&self) -> bool {
        *self != Self::default()
    }

    /// The flags that reproduce these choices.
    pub fn args(&self) -> Vec<String> {
        let mut args = Vec::new();
        if let Some(port) = self.https_port {
            args.extend(["--https-port".to_owned(), port.to_string()]);
        }
        if let Some(port) = self.ui_https_port {
            args.extend(["--ui-https-port".to_owned(), port.to_string()]);
        }
        if self.sudo_tailscale {
            args.push("--sudo-tailscale".to_owned());
        }
        args
    }

    fn parse(args: &[String]) -> Result<Self, clap::Error> {
        Saved::try_parse_from(std::iter::once("install-vps".to_owned()).chain(args.iter().cloned()))
            .map(|saved| saved.publish)
    }

    /// These flags if any were given, else the saved ones.
    pub fn resolve(self, path: &Path) -> Result<Self, InstallError> {
        if self.given() {
            return Ok(self);
        }
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(self),
            Err(source) => {
                return Err(InstallError::File {
                    path: path.to_path_buf(),
                    source,
                });
            }
        };
        let saved = toml::from_str::<SavedFile>(&text)
            .ok()
            .and_then(|file| Self::parse(&file.args).ok())
            .ok_or_else(|| InstallError::SavedOptions(path.to_path_buf()))?;
        InstallVps::step(&format!(
            "Using saved options from {}: {}",
            path.display(),
            saved.args().join(" ")
        ));
        Ok(saved)
    }

    /// Saves these choices for the next run.
    pub fn save(&self, path: &Path) -> Result<(), InstallError> {
        let text = toml::to_string(&SavedFile { args: self.args() }).map_err(|error| {
            InstallError::File {
                path: path.to_path_buf(),
                source: std::io::Error::other(error),
            }
        })?;
        InstallVps::write_private(path, text.as_bytes(), 0o600)
    }

    /// `~/.config/tgv/install-vps.toml`.
    pub fn path(home: &Path) -> PathBuf {
        home.join(".config/tgv/install-vps.toml")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all() -> Publish {
        Publish {
            https_port: Some(8443),
            sudo_tailscale: true,
            ui_https_port: Some(8444),
        }
    }

    #[test]
    fn saved_flags_round_trip_through_the_parser() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("install-vps.toml");
        all().save(&path).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "args = [\"--https-port\", \"8443\", \"--ui-https-port\", \"8444\", \"--sudo-tailscale\"]\n"
        );
        assert_eq!(Publish::default().resolve(&path).unwrap(), all());
    }

    #[test]
    fn given_flags_replace_the_saved_ones() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("install-vps.toml");
        all().save(&path).unwrap();
        let only = Publish {
            https_port: Some(9443),
            ..Publish::default()
        };
        assert_eq!(only.clone().resolve(&path).unwrap(), only);
    }

    #[test]
    fn nothing_saved_means_nothing_published() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing.toml");
        assert_eq!(
            Publish::default().resolve(&path).unwrap(),
            Publish::default()
        );
    }

    #[test]
    fn invalid_saved_flags_are_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("install-vps.toml");
        for text in [
            "args = [\"--ui-https-port\", \"8444\"]\n",
            "args = [\"--https-port\", \"0\"]\n",
            "args = [\"--skip-backup\"]\n",
            "nonsense",
        ] {
            std::fs::write(&path, text).unwrap();
            assert!(
                matches!(
                    Publish::default().resolve(&path),
                    Err(InstallError::SavedOptions(_))
                ),
                "{text}"
            );
        }
    }
}
